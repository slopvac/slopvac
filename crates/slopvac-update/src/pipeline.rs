//! Download, verify, extract, replace.
//!
//! The order is fixed and every step is fail-closed:
//!
//! 1. take the update lock, and refuse if the installed version is already at or past the target;
//! 2. download the archive and its `.minisig`;
//! 3. [`ArtifactVerifier::verify`] the archive bytes and the release identity the signature names;
//! 4. extract the binary into memory;
//! 5. write it beside the installed binary and rename it into place, then release the lock.
//!
//! Nothing but the lock file changes in the install directory before step 5, and step 5 starts
//! only after steps 2 to 4 succeed. The lock spans steps 1 to 5, so two updates cannot interleave
//! and a slower older one cannot rename over a newer install. Verification covers the archive bytes, not a digest taken from the same server.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use thiserror::Error;

use crate::fetch::{FetchError, Fetcher};
use crate::lock::{self, LockError, Running, UpdateLock};
use crate::probe;
use crate::verify::{ArtifactVerifier, ReleaseFile, VerifyError};

/// Where release assets live.
pub const DOWNLOAD_BASE: &str = "https://github.com/slopvac/slopvac/releases/download";
/// Largest archive accepted.
const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_SIGNATURE_BYTES: u64 = 16 * 1024;
const MAX_BINARY_BYTES: u64 = 512 * 1024 * 1024;

/// Why an update stopped. The installed binary is unchanged for every variant except
/// [`UpdateError::Replace`], which fails before the final rename.
#[derive(Debug, Error)]
pub enum UpdateError {
    /// This platform has no published archive that `self update` can read.
    #[error("self update does not support this platform; download a release archive instead")]
    UnsupportedPlatform,
    /// A download failed.
    #[error("download of {asset} failed: {source}")]
    Download {
        /// Asset file name.
        asset: String,
        /// Transport failure.
        source: FetchError,
    },
    /// The requested version is not newer than the running one. `self update` never downgrades.
    #[error("refusing to install {requested}: it is not newer than the running {current}")]
    NotNewer {
        /// Version of this binary.
        current: String,
        /// Version the update asked for.
        requested: String,
    },
    /// The requested version is a prerelease. `self update` installs stable releases only.
    #[error(
        "refusing to install {0}: it is a prerelease, and self update installs stable releases only"
    )]
    Prerelease(String),
    /// The signature check refused the archive.
    #[error("refusing to install: {0}")]
    Verify(#[from] VerifyError),
    /// The archive does not hold the binary.
    #[error("the archive does not contain a `slopvac` binary")]
    NoBinary,
    /// The archive is damaged.
    #[error("cannot read the archive: {0}")]
    Archive(String),
    /// Another update held the lock for the whole wait.
    #[error("{0}")]
    Busy(String),
    /// The update lock file is not safe to use.
    #[error("{0}")]
    LockUnsafe(String),
    /// An earlier update died part-way and left an installed binary that it does not know.
    #[error("{0}")]
    Interrupted(String),
    /// The installed binary's version cannot be established.
    #[error("{0}")]
    VersionUnknown(String),
    /// Writing the new binary failed.
    #[error("cannot replace the installed binary: {0}")]
    Replace(String),
}

impl From<LockError> for UpdateError {
    fn from(e: LockError) -> Self {
        match e {
            LockError::Busy(_) => Self::Busy(e.to_string()),
            LockError::Unsafe { .. } => Self::LockUnsafe(e.to_string()),
            LockError::Interrupted => Self::Interrupted(e.to_string()),
            LockError::VersionUnknown { .. } => Self::VersionUnknown(e.to_string()),
            LockError::Io { .. } => Self::Replace(e.to_string()),
        }
    }
}

/// The Rust target triple of this build, for the platforms `self update` supports.
#[must_use]
pub fn target_triple() -> Option<&'static str> {
    // Unix only: cargo-dist's Windows archive is a zip, which this pipeline does not read yet.
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("linux", "x86_64") if cfg!(target_env = "musl") => "x86_64-unknown-linux-musl",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        _ => return None,
    })
}

/// Archive file name for a target.
///
/// This reads `.tar.gz` only, because the workspace has no xz decoder. The release publishes
/// `.tar.gz` through `unix-archive = ".tar.gz"` in `dist-workspace.toml`.
#[must_use]
pub fn asset_name(target: &str) -> String {
    format!("slopvac-{target}.tar.gz")
}

/// The installed executable and what is known about the process that updates it.
#[derive(Debug, Clone)]
pub struct Host<'a> {
    exe: &'a Path,
    running_sha: Option<String>,
    lock_timeout: Duration,
    probe_timeout: Duration,
}

impl<'a> Host<'a> {
    /// The executable to replace, and the fingerprint of the running binary taken when the
    /// process started ([`running_fingerprint`]), or `None` when it could not be taken.
    ///
    /// The running version counts as the installed version only while the file at `exe` still
    /// has that fingerprint.
    #[must_use]
    pub fn new(exe: &'a Path, running_sha: Option<String>) -> Self {
        Self {
            exe,
            running_sha,
            lock_timeout: lock::DEFAULT_TIMEOUT,
            probe_timeout: probe::DEFAULT_TIMEOUT,
        }
    }
}

/// The fingerprint of the binary at `exe`, to hand to [`Host::new`].
///
/// Take it once, when the process starts the update: it later tells whether another installer
/// replaced the file. `None` means the file could not be read, and then nothing vouches for the
/// running version.
#[must_use]
pub fn running_fingerprint(exe: &Path) -> Option<String> {
    lock::fingerprint_file(exe).ok()
}

/// Run the pipeline and replace `host`'s executable, which runs `current`, with the release for
/// `version`.
///
/// The verifier must see a signature made for this exact asset of this exact `version`, so a
/// server cannot pass off an older or another target's signed archive. `version` must also be
/// a stable release newer than `current`: `self update` has no downgrade flag and no
/// candidate channel, so a signed prerelease is refused before any download.
///
/// The update holds an exclusive lock beside the executable from before the download until
/// after the rename. Under the lock it establishes the installed version from evidence tied to
/// the file's fingerprint: the lock record, the running version when the file is still the
/// running binary, or else what the file reports when it runs. It refuses when that is not older
/// than `version`, and fails with [`UpdateError::VersionUnknown`] when it cannot tell. It
/// records the update before the rename, so an updater that dies part-way is recovered or
/// refused by the next one, never installed over. It waits up to a minute for another update,
/// then fails with [`UpdateError::Busy`].
pub fn update(
    fetcher: &dyn Fetcher,
    verifier: &dyn ArtifactVerifier,
    base_url: &str,
    current: &str,
    version: &str,
    target: &str,
    host: &Host<'_>,
) -> Result<(), UpdateError> {
    let exe = host.exe;
    let not_newer = |current: &str| UpdateError::NotNewer {
        current: current.to_owned(),
        requested: version.to_owned(),
    };
    let (Ok(cur), Ok(new)) = (
        semver::Version::parse(current),
        semver::Version::parse(version),
    ) else {
        return Err(not_newer(current));
    };
    if !new.pre.is_empty() {
        return Err(UpdateError::Prerelease(version.to_owned()));
    }
    if new <= cur {
        return Err(not_newer(current));
    }
    let mut lock = UpdateLock::acquire(exe, host.lock_timeout)?;
    // Another update, or another installer, may have put a version on disk since this process
    // started. The version is established under the lock, so no update is in flight while it is
    // compared.
    let running = Running {
        version: &cur,
        sha: host.running_sha.as_deref(),
    };
    let installed = match lock.installed_version(&running, host.probe_timeout) {
        Ok(v) => v,
        Err(LockError::Interrupted) => {
            // Release our hold so that the advice can try the lock.
            drop(lock);
            return Err(UpdateError::Interrupted(format!(
                "{} {}",
                LockError::Interrupted,
                lock::recovery_hint(exe)
            )));
        }
        Err(e) => return Err(e.into()),
    };
    if new <= installed.version {
        return Err(not_newer(&installed.version.to_string()));
    }
    let asset = asset_name(target);
    let url = format!("{}/v{version}/{asset}", base_url.trim_end_matches('/'));
    let download = |url: &str, name: &str, limit| {
        fetcher
            .get(url, limit)
            .map_err(|source| UpdateError::Download {
                asset: name.to_owned(),
                source,
            })
    };
    let archive = download(&url, &asset, MAX_ARCHIVE_BYTES)?;
    let sig_name = format!("{asset}.minisig");
    let signature = download(&format!("{url}.minisig"), &sig_name, MAX_SIGNATURE_BYTES)?;

    let expected = ReleaseFile {
        name: &asset,
        version,
    };
    verifier.verify(&archive, &signature, &expected)?;

    let binary = extract_binary(&archive)?;
    let payload = lock.begin(&installed, &new, &binary)?;
    replace(exe, &binary)?;
    lock.finish(&new, &payload)?;
    Ok(())
}

/// Pull the `slopvac` executable out of a `.tar.gz`.
fn extract_binary(archive: &[u8]) -> Result<Vec<u8>, UpdateError> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    let err = |e: std::io::Error| UpdateError::Archive(e.to_string());
    for entry in tar.entries().map_err(err)? {
        let entry = entry.map_err(err)?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path().map_err(err)?;
        if path.file_name().is_some_and(|n| n == "slopvac") {
            let mut buf = Vec::new();
            entry
                .take(MAX_BINARY_BYTES + 1)
                .read_to_end(&mut buf)
                .map_err(err)?;
            if buf.len() as u64 > MAX_BINARY_BYTES {
                return Err(UpdateError::Archive("binary is too large".into()));
            }
            return Ok(buf);
        }
    }
    Err(UpdateError::NoBinary)
}

/// Write `bytes` beside `exe` and rename them over it, keeping the old file's permissions.
fn replace(exe: &Path, bytes: &[u8]) -> Result<(), UpdateError> {
    let fail = |e: &dyn std::fmt::Display| UpdateError::Replace(e.to_string());
    let dir = exe.parent().ok_or_else(|| fail(&"no parent directory"))?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".slopvac-update-")
        .tempfile_in(dir)
        .map_err(|e| fail(&e))?;
    std::io::Write::write_all(&mut tmp, bytes).map_err(|e| fail(&e))?;
    if let Ok(meta) = std::fs::metadata(exe) {
        std::fs::set_permissions(tmp.path(), meta.permissions()).map_err(|e| fail(&e))?;
    }
    tmp.persist(exe).map_err(|e| fail(&e.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// `super::update` for a process that is the binary at `exe` right now.
    fn update(
        fetcher: &dyn Fetcher,
        verifier: &dyn ArtifactVerifier,
        base_url: &str,
        current: &str,
        version: &str,
        target: &str,
        exe: &Path,
    ) -> Result<(), UpdateError> {
        let host = Host::new(exe, running_fingerprint(exe));
        super::update(fetcher, verifier, base_url, current, version, target, &host)
    }

    struct Files(HashMap<String, Vec<u8>>);

    impl Fetcher for Files {
        fn get(&self, url: &str, _limit: u64) -> Result<Vec<u8>, FetchError> {
            self.0
                .get(url)
                .cloned()
                .ok_or_else(|| FetchError::Network(format!("404 {url}")))
        }
    }

    struct Verdict(Result<(), VerifyError>);
    impl ArtifactVerifier for Verdict {
        fn verify(&self, _: &[u8], _: &[u8], _: &ReleaseFile<'_>) -> Result<(), VerifyError> {
            self.0.clone()
        }
    }

    const BASE: &str = "https://example.test/dl";
    const TARGET: &str = "x86_64-unknown-linux-gnu";

    fn tar_gz(name: &str, body: &[u8]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append_data(&mut header, name, body).unwrap();
        let tar = builder.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &tar).unwrap();
        gz.finish().unwrap()
    }

    fn release(archive: Vec<u8>) -> Files {
        let url = format!("{BASE}/v3.1.0/slopvac-{TARGET}.tar.gz");
        Files(HashMap::from([
            (format!("{url}.minisig"), b"sig".to_vec()),
            (url, archive),
        ]))
    }

    fn installed() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("slopvac");
        std::fs::write(&exe, b"old").unwrap();
        (dir, exe)
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn verified_archive_replaces_the_binary() {
        let (dir, exe) = installed();
        let files = release(tar_gz("slopvac-x/slopvac", b"new"));
        update(
            &files,
            &Verdict(Ok(())),
            BASE,
            "3.0.0",
            "3.1.0",
            TARGET,
            &exe,
        )
        .unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        assert_eq!(leftovers(dir.path()), ["slopvac", "slopvac.update.lock"]);
    }

    #[test]
    fn production_base_downloads_from_the_slopvac_org_releases() {
        let (_dir, exe) = installed();
        let url = format!(
            "https://github.com/slopvac/slopvac/releases/download/v3.1.0/slopvac-{TARGET}.tar.gz"
        );
        let files = Files(HashMap::from([
            (format!("{url}.minisig"), b"sig".to_vec()),
            (url, tar_gz("slopvac", b"new")),
        ]));
        update(
            &files,
            &Verdict(Ok(())),
            DOWNLOAD_BASE,
            "3.0.0",
            "3.1.0",
            TARGET,
            &exe,
        )
        .unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
    }

    #[cfg(unix)]
    #[test]
    fn replacement_keeps_the_executable_bit() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, exe) = installed();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let files = release(tar_gz("slopvac", b"new"));
        update(
            &files,
            &Verdict(Ok(())),
            BASE,
            "3.0.0",
            "3.1.0",
            TARGET,
            &exe,
        )
        .unwrap();
        assert_eq!(
            std::fs::metadata(&exe).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[test]
    fn verifier_rejection_leaves_the_binary_untouched() {
        for reject in [
            VerifyError::Rejected,
            VerifyError::Invalid("bad key".into()),
            VerifyError::MissingSignature,
        ] {
            let (dir, exe) = installed();
            let files = release(tar_gz("slopvac", b"tampered"));
            let err = update(
                &files,
                &Verdict(Err(reject.clone())),
                BASE,
                "3.0.0",
                "3.1.0",
                TARGET,
                &exe,
            )
            .unwrap_err();
            assert!(
                matches!(err, UpdateError::Verify(ref v) if *v == reject),
                "{err}"
            );
            assert_eq!(std::fs::read(&exe).unwrap(), b"old");
            assert_eq!(leftovers(dir.path()), ["slopvac", "slopvac.update.lock"]);
        }
    }

    #[test]
    fn verifier_sees_the_downloaded_bytes_before_extraction() {
        type Seen = (Vec<u8>, Vec<u8>, String);
        struct Spy(std::cell::RefCell<Option<Seen>>);
        impl ArtifactVerifier for Spy {
            fn verify(&self, a: &[u8], s: &[u8], f: &ReleaseFile<'_>) -> Result<(), VerifyError> {
                *self.0.borrow_mut() = Some((a.to_vec(), s.to_vec(), f.trusted_comment()));
                Err(VerifyError::Rejected)
            }
        }
        let (_dir, exe) = installed();
        // Not a valid archive: if extraction ran first the error would be `Archive`.
        let files = release(b"not an archive".to_vec());
        let spy = Spy(std::cell::RefCell::default());
        let err = update(&files, &spy, BASE, "3.0.0", "3.1.0", TARGET, &exe).unwrap_err();
        assert!(matches!(err, UpdateError::Verify(_)));
        assert_eq!(
            spy.0.take().unwrap(),
            (
                b"not an archive".to_vec(),
                b"sig".to_vec(),
                format!("slopvac-release file:slopvac-{TARGET}.tar.gz version:3.1.0")
            )
        );
    }

    #[test]
    fn a_prerelease_target_is_refused_before_any_download() {
        for requested in ["3.2.0-rc.1", "4.0.0-alpha", "3.1.1-rc.2+build"] {
            let (_dir, exe) = installed();
            let files = Files(HashMap::new());
            let err = update(
                &files,
                &Verdict(Ok(())),
                BASE,
                "3.1.0",
                requested,
                TARGET,
                &exe,
            )
            .unwrap_err();
            assert!(
                matches!(err, UpdateError::Prerelease(_)),
                "{requested}: {err}"
            );
            assert_eq!(std::fs::read(&exe).unwrap(), b"old");
        }
    }

    #[test]
    fn a_version_that_is_not_newer_is_refused_before_any_download() {
        for (current, requested) in [
            ("3.1.0", "3.1.0"),
            ("3.1.0", "3.0.9"),
            ("3.1.0", "2.9.0"),
            ("3.1.0", "latest"),
            ("garbage", "3.1.0"),
        ] {
            let (_dir, exe) = installed();
            // An empty server: any download would fail with `Download`, not `NotNewer`.
            let files = Files(HashMap::new());
            let err = update(
                &files,
                &Verdict(Ok(())),
                BASE,
                current,
                requested,
                TARGET,
                &exe,
            )
            .unwrap_err();
            assert!(
                matches!(err, UpdateError::NotNewer { .. }),
                "{current} -> {requested}: {err}"
            );
            assert_eq!(std::fs::read(&exe).unwrap(), b"old");
        }
    }

    #[test]
    fn missing_signature_download_stops_before_replacing() {
        let (_dir, exe) = installed();
        let mut files = release(tar_gz("slopvac", b"new"));
        files.0.retain(|k, _| !k.ends_with(".minisig"));
        let err = update(
            &files,
            &Verdict(Ok(())),
            BASE,
            "3.0.0",
            "3.1.0",
            TARGET,
            &exe,
        )
        .unwrap_err();
        assert!(matches!(err, UpdateError::Download { .. }));
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
    }

    #[test]
    fn archive_without_the_binary_leaves_it_untouched() {
        let (_dir, exe) = installed();
        let files = release(tar_gz("README.md", b"hi"));
        let err = update(
            &files,
            &Verdict(Ok(())),
            BASE,
            "3.0.0",
            "3.1.0",
            TARGET,
            &exe,
        )
        .unwrap_err();
        assert!(matches!(err, UpdateError::NoBinary));
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
    }

    #[test]
    fn corrupt_archive_leaves_it_untouched() {
        let (_dir, exe) = installed();
        let files = release(b"\x1f\x8b garbage".to_vec());
        let err = update(
            &files,
            &Verdict(Ok(())),
            BASE,
            "3.0.0",
            "3.1.0",
            TARGET,
            &exe,
        )
        .unwrap_err();
        assert!(matches!(err, UpdateError::Archive(_)));
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
    }

    fn ver(s: &str) -> semver::Version {
        semver::Version::parse(s).unwrap()
    }

    /// Another installer's build at `exe`, as a script that reports `body`'s output. The returned
    /// fingerprint is the binary this process "started" as, taken before the replacement.
    #[cfg(unix)]
    fn replaced_by_other_installer(exe: &Path, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let started_as = running_fingerprint(exe).unwrap();
        std::fs::write(exe, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        started_as
    }

    #[cfg(unix)]
    const REPORTS_3_3_0: &str = r#"echo '{"name":"slopvac","version":"3.3.0"}'"#;

    /// A host whose version probe may take long, so a slow first run of a new script on a busy
    /// machine is not mistaken for a hang.
    #[cfg(unix)]
    fn patient_host(exe: &Path, running_sha: Option<String>) -> Host<'_> {
        let mut host = Host::new(exe, running_sha);
        host.probe_timeout = Duration::from_secs(120);
        host
    }

    #[cfg(unix)]
    #[test]
    fn an_older_target_does_not_replace_a_newer_independent_install() {
        let (_dir, exe) = installed();
        // This process runs 3.0.0, and another installer has since put 3.3.0 on disk.
        let started_as = replaced_by_other_installer(&exe, REPORTS_3_3_0);
        let before = std::fs::read(&exe).unwrap();
        let files = release(tar_gz("slopvac", b"binary 3.1.0"));
        let err = super::update(
            &files,
            &Verdict(Ok(())),
            BASE,
            "3.0.0",
            "3.1.0",
            TARGET,
            &patient_host(&exe, Some(started_as)),
        )
        .unwrap_err();
        assert!(
            matches!(err, UpdateError::NotNewer { ref current, .. } if current == "3.3.0"),
            "{err}"
        );
        assert_eq!(std::fs::read(&exe).unwrap(), before);
    }

    #[cfg(unix)]
    #[test]
    fn a_newer_target_still_replaces_an_independent_install() {
        let (_dir, exe) = installed();
        let started_as = replaced_by_other_installer(&exe, REPORTS_3_3_0);
        let files = release(tar_gz("slopvac", b"binary 3.4.0"));
        let files = Files(
            files
                .0
                .into_iter()
                .map(|(k, v)| (k.replace("v3.1.0", "v3.4.0"), v))
                .collect(),
        );
        super::update(
            &files,
            &Verdict(Ok(())),
            BASE,
            "3.0.0",
            "3.4.0",
            TARGET,
            &patient_host(&exe, Some(started_as)),
        )
        .unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"binary 3.4.0");
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_running_fingerprint_is_not_trusted_as_the_installed_version() {
        let (_dir, exe) = installed();
        replaced_by_other_installer(&exe, REPORTS_3_3_0);
        let files = release(tar_gz("slopvac", b"binary 3.1.0"));
        let err = super::update(
            &files,
            &Verdict(Ok(())),
            BASE,
            "3.0.0",
            "3.1.0",
            TARGET,
            &patient_host(&exe, None),
        )
        .unwrap_err();
        assert!(matches!(err, UpdateError::NotNewer { .. }), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn an_installed_binary_that_will_not_report_a_version_fails_closed() {
        for body in [
            "echo garbage",
            "exit 1",
            "echo '{\"name\":\"slopvac\",\"version\":\"3\"}'",
        ] {
            let (_dir, exe) = installed();
            let started_as = replaced_by_other_installer(&exe, body);
            let before = std::fs::read(&exe).unwrap();
            let files = release(tar_gz("slopvac", b"binary 3.1.0"));
            let err = super::update(
                &files,
                &Verdict(Ok(())),
                BASE,
                "3.0.0",
                "3.1.0",
                TARGET,
                &patient_host(&exe, Some(started_as)),
            )
            .unwrap_err();
            assert!(
                matches!(err, UpdateError::VersionUnknown(_)),
                "{body}: {err}"
            );
            assert_eq!(std::fs::read(&exe).unwrap(), before);
        }
    }

    #[cfg(unix)]
    #[test]
    fn an_installed_binary_that_times_out_fails_closed() {
        let (_dir, exe) = installed();
        let started_as = replaced_by_other_installer(&exe, "exec sleep 30");
        let files = release(tar_gz("slopvac", b"binary 3.1.0"));
        let mut host = Host::new(&exe, Some(started_as));
        host.probe_timeout = Duration::from_millis(200);
        let err = super::update(
            &files,
            &Verdict(Ok(())),
            BASE,
            "3.0.0",
            "3.1.0",
            TARGET,
            &host,
        )
        .unwrap_err();
        assert!(matches!(err, UpdateError::VersionUnknown(_)), "{err}");
    }

    /// `exe` as the update lock establishes it, for a process that runs `version` from this file.
    fn checked(lock: &mut UpdateLock, exe: &Path, version: &str) -> lock::Installed {
        let version = ver(version);
        lock.installed_version(
            &Running {
                version: &version,
                sha: running_fingerprint(exe).as_deref(),
            },
            Duration::from_secs(300),
        )
        .unwrap()
    }

    /// An updater to `to` that dies right after the rename: pending record, new bytes, no final.
    fn die_after_rename(exe: &Path, from: &str, to: &str, bytes: &[u8]) {
        let mut dead = UpdateLock::acquire(exe, Duration::ZERO).unwrap();
        let before = checked(&mut dead, exe, from);
        dead.begin(&before, &ver(to), bytes).unwrap();
        std::fs::write(exe, bytes).unwrap();
    }

    #[test]
    fn an_older_update_refuses_to_install_over_a_newer_one_that_died_before_recording() {
        let (_dir, exe) = installed();
        die_after_rename(&exe, "3.0.0", "3.2.0", b"binary 3.2.0");
        let files = release(tar_gz("slopvac", b"binary 3.1.0"));
        let err = update(
            &files,
            &Verdict(Ok(())),
            BASE,
            "3.0.0",
            "3.1.0",
            TARGET,
            &exe,
        )
        .unwrap_err();
        assert!(
            matches!(err, UpdateError::NotNewer { ref current, .. } if current == "3.2.0"),
            "{err}"
        );
        assert_eq!(std::fs::read(&exe).unwrap(), b"binary 3.2.0");
    }

    #[test]
    fn an_update_over_an_unknown_binary_left_by_a_dead_updater_fails_interrupted() {
        let (_dir, exe) = installed();
        die_after_rename(&exe, "3.0.0", "3.2.0", b"binary 3.2.0");
        std::fs::write(&exe, b"another installer's build").unwrap();
        let files = release(tar_gz("slopvac", b"binary 3.1.0"));
        let err = update(
            &files,
            &Verdict(Ok(())),
            BASE,
            "3.0.0",
            "3.1.0",
            TARGET,
            &exe,
        )
        .unwrap_err();
        assert!(matches!(err, UpdateError::Interrupted(_)), "{err}");
        // No update holds the lock now, so the advice says it is safe to remove.
        assert!(
            err.to_string().contains("remove slopvac.update.lock"),
            "{err}"
        );
        assert_eq!(std::fs::read(&exe).unwrap(), b"another installer's build");
    }

    #[test]
    fn an_update_proceeds_when_the_dead_updater_never_reached_the_rename() {
        let (_dir, exe) = installed();
        let mut dead = UpdateLock::acquire(&exe, Duration::ZERO).unwrap();
        let before = checked(&mut dead, &exe, "3.0.0");
        dead.begin(&before, &ver("3.2.0"), b"binary 3.2.0")
            .unwrap();
        drop(dead);
        let files = release(tar_gz("slopvac", b"binary 3.1.0"));
        update(
            &files,
            &Verdict(Ok(())),
            BASE,
            "3.0.0",
            "3.1.0",
            TARGET,
            &exe,
        )
        .unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"binary 3.1.0");
    }

    #[cfg(unix)]
    #[test]
    fn a_planted_lock_symlink_fails_the_update_before_any_download() {
        let (dir, exe) = installed();
        let victim = dir.path().join("victim");
        std::fs::write(&victim, b"precious").unwrap();
        std::os::unix::fs::symlink(&victim, dir.path().join("slopvac.update.lock")).unwrap();
        let err = update(
            &Files(HashMap::new()),
            &Verdict(Ok(())),
            BASE,
            "3.0.0",
            "3.1.0",
            TARGET,
            &exe,
        )
        .unwrap_err();
        assert!(matches!(err, UpdateError::LockUnsafe(_)), "{err}");
        assert_eq!(std::fs::read(&victim).unwrap(), b"precious");
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
    }

    #[test]
    fn an_update_that_cannot_get_the_lock_in_time_fails_busy_and_changes_nothing() {
        let (_dir, exe) = installed();
        let _held = UpdateLock::acquire(&exe, Duration::ZERO).unwrap();
        // An empty server: reaching the download would fail with `Download`, not `Busy`.
        let mut host = Host::new(&exe, running_fingerprint(&exe));
        host.lock_timeout = Duration::ZERO;
        let err = super::update(
            &Files(HashMap::new()),
            &Verdict(Ok(())),
            BASE,
            "3.0.0",
            "3.1.0",
            TARGET,
            &host,
        )
        .unwrap_err();
        assert!(matches!(err, UpdateError::Busy(_)), "{err}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
    }

    #[test]
    fn real_minisign_signature_end_to_end_and_tamper_rejected() {
        use crate::verify::MinisignVerifier;
        let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let archive = tar_gz("slopvac", b"new");
        let comment = format!("slopvac-release file:slopvac-{TARGET}.tar.gz version:3.1.0");
        let sig = minisign::sign(
            None,
            &kp.sk,
            std::io::Cursor::new(&archive),
            Some(&comment),
            None,
        )
        .unwrap()
        .to_string();
        let verifier = MinisignVerifier::from_base64(&kp.pk.to_base64()).unwrap();
        let url = format!("{BASE}/v3.1.0/slopvac-{TARGET}.tar.gz");

        let (_d, exe) = installed();
        let good = Files(HashMap::from([
            (format!("{url}.minisig"), sig.clone().into_bytes()),
            (url.clone(), archive.clone()),
        ]));
        update(&good, &verifier, BASE, "3.0.0", "3.1.0", TARGET, &exe).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");

        let (_d, exe) = installed();
        let evil = tar_gz("slopvac", b"evil");
        let bad = Files(HashMap::from([
            (format!("{url}.minisig"), sig.into_bytes()),
            (url, evil),
        ]));
        let err = update(&bad, &verifier, BASE, "3.0.0", "3.1.0", TARGET, &exe).unwrap_err();
        assert!(matches!(err, UpdateError::Verify(VerifyError::Rejected)));
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
    }
}
