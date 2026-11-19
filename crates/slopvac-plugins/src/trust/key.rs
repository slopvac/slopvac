//! The per-user key behind the trust hash's commitment to environment values.
//!
//! A plugin's declared environment can carry a credential. The trust hash must
//! change when that value changes, but it is shown in diagnostics and pinned in
//! `SLOPVAC_TRUST`, so it must not let anyone test guesses for the value. Each
//! value therefore enters the hash as an HMAC-SHA256 under a random key that
//! stays in the user's trust store (`trust.key`, mode 0600, owned by the user).
//!
//! The key is created only by `slopvac plugins trust`, and only when no trust
//! record is left. Each record names the key it was made under by
//! [`TrustKey::id`]. A missing key while records exist, or a key whose id
//! differs from a record's, is an error: the run fails closed and never falls
//! back to an unkeyed hash or a fresh key. Trusting again is the one way back,
//! and it drops the records that the lost key made, because their hashes can
//! no longer be recomputed.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;

use hmac::{Hmac, Mac};
use sha2::Sha256;

use super::{TrustError, TrustRecord, TrustStore, io_err, set_private_file};

/// The key file in the store directory.
const KEY_FILE: &str = "trust.key";
const KEY_BYTES: usize = 32;
/// A key file is 64 hex digits and a newline; anything much longer is not one.
const KEY_FILE_MAX: u64 = 256;
const ID_LABEL: &[u8] = b"slopvac trust key id v1";
const ENV_LABEL: &[u8] = b"slopvac trust env v1\0";

type HmacSha256 = Hmac<Sha256>;

/// The secret that keys the trust hash's commitment to environment values.
///
/// Obtain it from [`TrustStore::key`]. [`Self::from_bytes`] builds one from
/// known bytes, for tests and for tools that hold their own key.
#[derive(Clone)]
pub struct TrustKey {
    bytes: [u8; KEY_BYTES],
    id: String,
}

impl fmt::Debug for TrustKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrustKey")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl TrustKey {
    /// A key from its bytes.
    #[must_use]
    pub fn from_bytes(bytes: [u8; KEY_BYTES]) -> Self {
        let id = hex(&mac(&bytes, &[ID_LABEL])[..8]);
        Self { bytes, id }
    }

    /// A short public name for the key, which records carry. It is an HMAC of
    /// a fixed label, so it reveals nothing that helps recover the key.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The commitment to one environment variable: HMAC-SHA256 over its name
    /// and value, as lowercase hex.
    pub(super) fn commit_env(&self, name: &str, value: &str) -> String {
        hex(&mac(
            &self.bytes,
            &[ENV_LABEL, name.as_bytes(), &[0], value.as_bytes()],
        ))
    }

    fn generate() -> Result<Self, TrustError> {
        let mut bytes = [0_u8; KEY_BYTES];
        getrandom::fill(&mut bytes).map_err(|e| TrustError::Io {
            what: "drawing a trust key from the system".into(),
            source: io::Error::other(e),
        })?;
        Ok(Self::from_bytes(bytes))
    }

    fn parse(text: &str) -> Option<Self> {
        let text = text.trim_end_matches('\n');
        let lower_hex = text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if text.len() != KEY_BYTES * 2 || !lower_hex {
            return None;
        }
        let mut bytes = [0_u8; KEY_BYTES];
        for (slot, pair) in bytes.iter_mut().zip(text.as_bytes().chunks(2)) {
            *slot = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
        }
        Some(Self::from_bytes(bytes))
    }

    fn to_file_text(&self) -> String {
        format!("{}\n", hex(&self.bytes))
    }
}

fn mac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut m = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    for part in parts {
        m.update(part);
    }
    m.finalize().into_bytes().into()
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    out
}

/// Why a key file with this owner and mode cannot be used, if it cannot.
/// `euid` is the user running the process.
fn mode_problem(mode: u32, uid: u32, euid: u32) -> Option<String> {
    if uid != euid {
        return Some(format!("it belongs to user {uid}, not to you ({euid})"));
    }
    let bits = mode & 0o777;
    (bits != 0o600).then(|| format!("its mode is {bits:03o}, not 600"))
}

#[cfg(unix)]
fn check_owner_and_mode(meta: &fs::Metadata) -> Option<String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    mode_problem(
        meta.permissions().mode(),
        meta.uid(),
        rustix::process::geteuid().as_raw(),
    )
}

#[cfg(not(unix))]
fn check_owner_and_mode(_: &fs::Metadata) -> Option<String> {
    None
}

/// Read the key file. `None` when there is none; an error when it is not a
/// regular file the user alone can read and write, or does not hold a key.
fn read_key_file(path: &Path) -> Result<Option<TrustKey>, TrustError> {
    let unusable = |why: String| TrustError::KeyUnusable {
        path: path.to_owned(),
        why,
    };
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.file_type().is_file() => {
            return Err(unusable("it is not a regular file".into()));
        }
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_err(format!("inspecting {}", path.display()))(e)),
    }
    let file = File::open(path).map_err(io_err(format!("opening {}", path.display())))?;
    // The opened file, not the name, so a swap after the check above changes nothing.
    let meta = file
        .metadata()
        .map_err(io_err(format!("inspecting {}", path.display())))?;
    if let Some(why) = check_owner_and_mode(&meta) {
        return Err(unusable(why));
    }
    let mut raw = Vec::new();
    file.take(KEY_FILE_MAX + 1)
        .read_to_end(&mut raw)
        .map_err(io_err(format!("reading {}", path.display())))?;
    String::from_utf8(raw)
        .ok()
        .and_then(|text| TrustKey::parse(&text))
        .map(Some)
        .ok_or_else(|| unusable("it does not hold a 32-byte key in hex".into()))
}

/// Write a new key beside the file, then link it into place without replacing
/// anything. Callers hold the store lock.
fn create_key_file(dir: &Path) -> Result<TrustKey, TrustError> {
    let key = TrustKey::generate()?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".trust-key-")
        .tempfile_in(dir)
        .map_err(io_err("creating a temporary trust key file"))?;
    set_private_file(tmp.path())?;
    tmp.write_all(key.to_file_text().as_bytes())
        .map_err(io_err("writing the trust key"))?;
    tmp.as_file()
        .sync_all()
        .map_err(io_err("syncing the trust key"))?;
    tmp.persist_noclobber(dir.join(KEY_FILE))
        .map_err(|e| TrustError::Io {
            what: "installing the trust key".into(),
            source: e.error,
        })?;
    Ok(key)
}

fn names_key(rec: &TrustRecord, key: &TrustKey) -> bool {
    rec.trust_key_id.as_deref() == Some(key.id())
}

impl TrustStore {
    /// The key for hashing a plugin's environment. Read-only: it writes
    /// nothing, so a check never creates the store (`slopvac plugins trust` does).
    /// With no key file and no records the hash is computed under a throwaway
    /// key, which only matters for a plugin that declares environment values:
    /// the hash it shows is then not one to pin.
    ///
    /// # Errors
    /// [`TrustError::KeyMissing`] when records exist and the key does not,
    /// [`TrustError::KeyMismatch`] when a record names another key,
    /// [`TrustError::KeyUnusable`] for a key file that is not private to the
    /// user or does not hold a key, plus store failures.
    pub fn key(&self) -> Result<TrustKey, TrustError> {
        let records = self.read()?.records;
        let path = self.dir.join(KEY_FILE);
        match read_key_file(&path)? {
            Some(key) if records.iter().all(|r| names_key(r, &key)) => Ok(key),
            Some(_) => Err(TrustError::KeyMismatch),
            None if records.is_empty() => TrustKey::generate(),
            None => Err(TrustError::KeyMissing { path }),
        }
    }

    /// The key for recording trust, and the way back after a lost or replaced
    /// key: the records that name any other key are dropped, since their
    /// hashes cannot be recomputed, and a key is created if the file is
    /// missing. This is the only place a key is created, and only once no
    /// record is left. A key file that is unusable is still an error, so a key
    /// someone else could read is never replaced silently.
    ///
    /// # Errors
    /// [`TrustError::KeyUnusable`], plus store failures.
    pub(super) fn key_for_recording(&self) -> Result<TrustKey, TrustError> {
        let path = self.dir.join(KEY_FILE);
        self.update(|file| {
            let existing = read_key_file(&path)?;
            file.records
                .retain(|r| existing.as_ref().is_some_and(|k| names_key(r, k)));
            match existing {
                Some(key) => Ok(key),
                None => create_key_file(&self.dir),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::mode_problem;

    #[rstest::rstest]
    #[case::mine_and_private(1000, 0o100_600, 1000, false)]
    #[case::another_user(1001, 0o100_600, 1000, true)]
    #[case::root_when_i_am_not(0, 0o100_600, 1000, true)]
    #[case::group_bits(1000, 0o100_660, 1000, true)]
    fn the_owner_and_mode_decide_whether_a_key_file_is_usable(
        #[case] uid: u32,
        #[case] mode: u32,
        #[case] euid: u32,
        #[case] refused: bool,
    ) {
        assert_eq!(mode_problem(mode, uid, euid).is_some(), refused);
    }
}
