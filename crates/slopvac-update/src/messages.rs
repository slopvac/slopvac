//! The messages `slopvac-update` reports: error codes, the update notice, sentences and fixed phrases.
//!
//! Three catalogs hold them. [`UpdateMessage`] has one entry per published code and the update
//! notice. `UpdateText` holds sentences with parameters. `UpdatePhrase` holds fixed phrases,
//! which callers read as `&'static str` through `as_static`. The binary registers the three
//! through `error_entries`, `text_entries` and `phrase_entries`.

use std::path::PathBuf;

use slopvac_messages::{Catalog, Entry};

/// The error codes and the notice of `slopvac self update` and the update check.
///
/// The enum is public because the update lock and the pipeline return the lock refusals as
/// errors, and because `offline-network-needed` is shared: the binary's `report` command renders
/// it with its own command name.
#[derive(Debug, Clone, PartialEq, Eq, Catalog)]
pub enum UpdateMessage {
    /// `self update` ran on an install that the slopvac installer does not manage.
    #[code("self-update-unmanaged")]
    #[message("this install ({method}) is not managed by the slopvac installer; {advice}")]
    SelfUpdateUnmanaged {
        /// The install method id.
        method: String,
        /// What to do instead.
        advice: String,
    },
    /// A command that needs the network ran with offline mode on.
    #[code("offline-network-needed")]
    #[message("offline: {command} needs the network")]
    OfflineNetworkNeeded {
        /// The command that needs the network, as the user types it after `slopvac`.
        command: String,
    },
    /// Asking the registry for the newest version failed.
    #[code("update-check-failed")]
    #[message("{detail}")]
    UpdateCheckFailed {
        /// The failure, as text.
        detail: String,
    },
    /// The download, extraction or replacement of the binary failed.
    #[code("self-update-failed")]
    #[message("{detail}")]
    SelfUpdateFailed {
        /// The failure, as text.
        detail: String,
    },
    /// The signature check refused the downloaded archive.
    #[code("update-verification-failed")]
    #[message("{detail}")]
    UpdateVerificationFailed {
        /// The failure, as text.
        detail: String,
    },
    /// The newest release is a prerelease, which `self update` never installs.
    #[code("update-prerelease-refused")]
    #[message("{detail}")]
    UpdatePrereleaseRefused {
        /// The failure, as text.
        detail: String,
    },
    /// The nearest `slopvac.toml` is larger than the policy read allows. The text follows the path.
    #[code("update-policy-too-large")]
    #[message("larger than {cap} bytes")]
    UpdatePolicyTooLarge {
        /// The cap, in bytes.
        #[count]
        cap: u64,
    },
    /// A newer release exists. The advice names the upgrade command or the releases page.
    #[code("update-available")]
    #[notice("slopvac {latest} is available (you have {current}). {advice}")]
    UpdateAvailable {
        /// The newest release.
        latest: String,
        /// The running version.
        current: String,
        /// The upgrade command or the releases page, and the migrate hint for a new major version.
        advice: String,
    },
    /// Another `self update` held the update lock for the whole 60-second wait.
    #[code("update-in-progress")]
    #[message("another slopvac update is running (lock file {file}); try again when it finishes")]
    InProgress {
        /// Lock file name.
        file: String,
    },
    /// The update lock file is a symbolic link, has more than one hard link, belongs to another user or has a mode looser than 0600.
    #[code("update-lock-unsafe")]
    #[message("refusing to use the update lock {file}: {problem}")]
    LockUnsafe {
        /// Lock file name.
        file: String,
        /// What is wrong with it.
        problem: String,
    },
    /// An earlier update died part-way and the installed binary matches no known build, or the update record is unreadable.
    #[code("update-interrupted")]
    #[message(
        "an earlier update was interrupted and the installed binary is not one it knows; \
         check the installed slopvac. {hint}"
    )]
    Interrupted {
        /// What to do next, which depends on whether another update holds the lock.
        hint: String,
    },
    /// No update record vouches for the installed binary and it did not report a version when run.
    #[code("update-version-unknown")]
    #[message(
        "cannot tell which version of slopvac is installed ({reason}); refusing to install over \
         it. Check that the installed slopvac runs, then run the update again"
    )]
    VersionUnknown {
        /// Why the executable's version is unknown.
        reason: String,
    },
}

/// Sentences with parameters: success lines, advice, error details and policy problems.
#[derive(Debug, Clone, PartialEq, Eq, Catalog)]
pub(crate) enum UpdateText {
    /// `self update` found nothing newer.
    #[code("self-update-up-to-date")]
    #[text("slopvac {current} is up to date.")]
    UpToDate { current: String },
    /// `self update` replaced the binary.
    #[code("self-update-done")]
    #[text("Updated slopvac {current} to {latest}.")]
    Updated { current: String, latest: String },
    /// Advice for an install that has its own upgrade command.
    #[code("self-update-advice-run")]
    #[text("run `{command}`")]
    AdviceRun { command: String },
    /// Notice advice for an install that has its own upgrade command.
    #[code("update-notice-upgrade-with")]
    #[text("Upgrade with `{command}`.")]
    UpgradeWith { command: String },
    /// Notice advice followed by the hint for a new major version.
    #[code("update-notice-advice-and-hint")]
    #[text("{advice} {hint}")]
    AdviceAndHint { advice: String, hint: String },
    /// The response was larger than the fetch limit.
    #[code("update-fetch-too-large")]
    #[text("response is larger than {limit} bytes")]
    FetchTooLarge {
        #[count]
        limit: u64,
    },
    /// A download failed.
    #[code("update-pipeline-download-failed")]
    #[text("download of {asset} failed: {detail}")]
    DownloadFailed { asset: String, detail: String },
    /// The requested version is not newer than the running one.
    #[code("update-pipeline-not-newer")]
    #[text("refusing to install {requested}: it is not newer than the running {current}")]
    NotNewer { requested: String, current: String },
    /// The requested version is a prerelease.
    #[code("update-pipeline-prerelease")]
    #[text(
        "refusing to install {version}: it is a prerelease, and self update installs stable releases only"
    )]
    Prerelease { version: String },
    /// The signature check refused the archive.
    #[code("update-pipeline-verify-refused")]
    #[text("refusing to install: {detail}")]
    VerifyRefused { detail: String },
    /// The archive could not be read.
    #[code("update-pipeline-archive-unreadable")]
    #[text("cannot read the archive: {detail}")]
    ArchiveUnreadable { detail: String },
    /// The new binary could not be written over the old one.
    #[code("update-pipeline-replace-failed")]
    #[text("cannot replace the installed binary: {detail}")]
    ReplaceFailed { detail: String },
    /// The signature or the key does not parse.
    #[code("update-verify-invalid")]
    #[text("invalid signature or key: {detail}")]
    VerifyInvalid { detail: String },
    /// The signature names another release or file.
    #[code("update-verify-wrong-release")]
    #[text(
        "the signature is for a different release or file: expected `{expected}`, signed `{signed}`"
    )]
    VerifyWrongRelease { expected: String, signed: String },
    /// A policy problem with the file it is about.
    #[code("update-policy-at-path")]
    #[text("{path}: {problem}")]
    PolicyAtPath {
        #[display_path]
        path: PathBuf,
        problem: String,
    },
    /// The update lock file could not be opened, read, written or locked.
    #[code("update-lock-io")]
    #[text("cannot use the update lock {file}: {detail}")]
    LockIo { file: String, detail: String },
    /// Why the installed version is unknown: the executable's name and the reason.
    #[code("update-version-reason")]
    #[text("{name}: {reason}")]
    VersionReason { name: String, reason: String },
    /// The installed executable did not start.
    #[code("update-probe-spawn")]
    #[text("it did not start: {detail}")]
    ProbeSpawn { detail: String },
    /// The installed executable did not report its version in time.
    #[code("update-probe-timeout")]
    #[text("it did not answer within {seconds} s")]
    ProbeTimeout { seconds: String },
    /// The installed executable exited with a failure.
    #[code("update-probe-failed")]
    #[text("it exited with {status}")]
    ProbeFailed { status: String },
    /// The installed executable wrote more than the output cap.
    #[code("update-probe-too-large")]
    #[text("its output is larger than {limit} bytes")]
    ProbeTooLarge {
        #[count]
        limit: u64,
    },
    /// The output of the installed executable could not be read.
    #[code("update-probe-read")]
    #[text("cannot read its output: {detail}")]
    ProbeRead { detail: String },
    /// Advice after an interrupted update when no other update runs.
    #[code("update-hint-remove-lock")]
    #[text("No other update is running, so remove {name} beside it and run the update again.")]
    HintRemoveLock { name: String },
    /// Advice after an interrupted update when the lock state is unknown.
    #[code("update-hint-check-then-remove")]
    #[text(
        "Make sure that no slopvac update is running before you remove {name}, then run the update again."
    )]
    HintCheckThenRemove { name: String },
    /// The policy file could not be read.
    #[code("update-policy-unreadable")]
    #[text("cannot read: {reason}")]
    PolicyUnreadable { reason: String },
}

/// Fixed phrases: command help, advice, fetch and archive failures. Each reads as a `&'static str`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Catalog)]
pub(crate) enum UpdatePhrase {
    /// Command help for `self`.
    #[code("self-about")]
    #[text("Manage this installation.")]
    SelfAbout,
    /// Command help for `self update`.
    #[code("self-update-about")]
    #[text("Update slopvac to the latest release.")]
    SelfUpdateAbout,
    /// Long command help for `self update`.
    #[code("self-update-long-about")]
    #[text(
        "Update slopvac to the latest release. Works for installs made by the slopvac installer. For any other install it prints the command that upgrades it. The download must carry a valid minisign signature, or nothing is replaced."
    )]
    SelfUpdateLongAbout,
    /// A `self` subcommand that the command tree does not define.
    #[code("self-unknown-subcommand")]
    #[text("unknown `self` subcommand")]
    UnknownSubcommand,
    /// Advice for an install with no upgrade command.
    #[code("self-update-advice-download")]
    #[text("download a release from https://github.com/slopvac/slopvac/releases")]
    AdviceDownload,
    /// Notice advice for an install with no upgrade command.
    #[code("update-notice-see-releases")]
    #[text("See https://github.com/slopvac/slopvac/releases.")]
    SeeReleases,
    /// Notice hint for a new major version.
    #[code("update-notice-migrate-hint")]
    #[text("This is a new major version. Run `slopvac migrate` to update your configuration.")]
    MigrateHint,
    /// The URL is not HTTPS.
    #[code("update-fetch-not-https")]
    #[text("refusing a non-HTTPS URL")]
    FetchNotHttps,
    /// A redirect led to a host that does not serve releases.
    #[code("update-fetch-redirect-host")]
    #[text("refusing a redirect to a host that does not serve GitHub releases")]
    FetchRedirectHost,
    /// The redirect limit was passed.
    #[code("update-fetch-too-many-redirects")]
    #[text("too many redirects")]
    FetchTooManyRedirects,
    /// Redirects are refused in this build.
    #[code("update-fetch-redirects-refused")]
    #[text("redirects are refused")]
    FetchRedirectsRefused,
    /// A redirect named no target.
    #[code("update-fetch-redirect-no-location")]
    #[text("redirect without a Location")]
    FetchRedirectNoLocation,
    /// This platform has no archive that `self update` reads.
    #[code("update-pipeline-unsupported-platform")]
    #[text("self update does not support this platform; download a release archive instead")]
    UnsupportedPlatform,
    /// The archive holds no `slopvac` binary.
    #[code("update-pipeline-no-binary")]
    #[text("the archive does not contain a `slopvac` binary")]
    NoBinary,
    /// The binary in the archive is over the size limit.
    #[code("update-pipeline-binary-too-large")]
    #[text("binary is too large")]
    BinaryTooLarge,
    /// The installed binary has no parent directory to write beside.
    #[code("update-pipeline-no-parent")]
    #[text("no parent directory")]
    NoParentDirectory,
    /// The lock file is not a regular file.
    #[code("update-lock-not-regular")]
    #[text("it is not a regular file")]
    LockNotRegular,
    /// The lock file has more than one hard link.
    #[code("update-lock-hard-links")]
    #[text("it has more than one hard link")]
    LockHardLinks,
    /// Another user owns the lock file.
    #[code("update-lock-other-owner")]
    #[text("it is owned by another user")]
    LockOtherOwner,
    /// The lock file mode is looser than 0600.
    #[code("update-lock-loose-mode")]
    #[text("its mode is looser than 0600")]
    LockLooseMode,
    /// The lock file is a symbolic link.
    #[code("update-lock-symlink")]
    #[text("it is a symbolic link")]
    LockSymlink,
    /// The installed binary changed between the version check and the record write.
    #[code("update-lock-binary-changed")]
    #[text("the installed binary changed after its version was checked")]
    LockBinaryChanged,
    /// The installed binary is not the build the update wrote.
    #[code("update-lock-binary-not-written")]
    #[text("the installed binary is not the build this update wrote")]
    LockBinaryNotWritten,
    /// The installed executable's output is not a slopvac version document.
    #[code("update-probe-unparseable")]
    #[text("its output is not a slopvac version document")]
    ProbeUnparseable,
    /// Advice after an interrupted update when another update holds the lock.
    #[code("update-hint-wait")]
    #[text("Another slopvac update is running; wait for it to finish, then run the update again.")]
    HintWait,
    /// The registry answered in an unexpected shape.
    #[code("update-registry-unexpected")]
    #[text("unexpected registry response")]
    RegistryUnexpected,
    /// The release has no signature for the artifact.
    #[code("update-verify-missing")]
    #[text("the release has no signature for this artifact")]
    VerifyMissing,
    /// The signature does not match the bytes.
    #[code("update-verify-rejected")]
    #[text("the signature does not verify")]
    VerifyRejected,
    /// The signature text is not UTF-8.
    #[code("update-verify-not-utf8")]
    #[text("signature is not UTF-8")]
    VerifyNotUtf8,
    /// The policy file is not a regular file.
    #[code("update-policy-not-regular")]
    #[text("not a regular file")]
    PolicyNotRegular,
}

/// The entries of `UpdateMessage`: one per code, and the update notice.
#[must_use]
pub fn error_entries() -> &'static [Entry] {
    UpdateMessage::entries()
}

/// The entries of `UpdateText`: sentences with parameters.
#[must_use]
pub fn text_entries() -> &'static [Entry] {
    UpdateText::entries()
}

/// The entries of `UpdatePhrase`: fixed phrases.
#[must_use]
pub fn phrase_entries() -> &'static [Entry] {
    UpdatePhrase::entries()
}

/// `text` followed by the newline that ends a line of command output.
pub(crate) fn line(text: &impl ToString) -> String {
    let mut out = text.to_string();
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(text: &str) -> String {
        text.to_owned()
    }

    /// Every message, rendered with sample parameters, beside the text that 3.0 builds published
    /// before the registry. A change to any pair changes what users read.
    #[test]
    fn every_code_keeps_its_published_text_and_is_registered() {
        let messages: Vec<(UpdateMessage, &str)> = vec![
            (
                UpdateMessage::SelfUpdateUnmanaged {
                    method: s("cargo"),
                    advice: s("run `cargo install --locked --force slopvac`"),
                },
                "this install (cargo) is not managed by the slopvac installer; run `cargo install --locked --force slopvac`",
            ),
            (
                UpdateMessage::OfflineNetworkNeeded {
                    command: s("self update"),
                },
                "offline: self update needs the network",
            ),
            (
                UpdateMessage::InProgress {
                    file: s("slopvac.update.lock"),
                },
                "another slopvac update is running (lock file slopvac.update.lock); try again when it finishes",
            ),
            (
                UpdateMessage::LockUnsafe {
                    file: s("slopvac.update.lock"),
                    problem: s("it is a symbolic link"),
                },
                "refusing to use the update lock slopvac.update.lock: it is a symbolic link",
            ),
            (
                UpdateMessage::Interrupted {
                    hint: s("Try again."),
                },
                "an earlier update was interrupted and the installed binary is not one it knows; check the installed slopvac. Try again.",
            ),
            (
                UpdateMessage::VersionUnknown {
                    reason: s("it printed nothing"),
                },
                "cannot tell which version of slopvac is installed (it printed nothing); refusing to install over it. Check that the installed slopvac runs, then run the update again",
            ),
            (UpdateMessage::UpdateCheckFailed { detail: s("d") }, "d"),
            (UpdateMessage::SelfUpdateFailed { detail: s("d") }, "d"),
            (
                UpdateMessage::UpdateVerificationFailed { detail: s("d") },
                "d",
            ),
            (
                UpdateMessage::UpdatePrereleaseRefused { detail: s("d") },
                "d",
            ),
            (
                UpdateMessage::UpdatePolicyTooLarge { cap: 1_048_576 },
                "larger than 1048576 bytes",
            ),
            (
                UpdateMessage::UpdateAvailable {
                    latest: s("3.2.0"),
                    current: s("3.1.0"),
                    advice: s("Upgrade with `brew upgrade slopvac/tap/slopvac`."),
                },
                "slopvac 3.2.0 is available (you have 3.1.0). Upgrade with `brew upgrade slopvac/tap/slopvac`.",
            ),
        ];
        let registered = error_entries();
        for (message, text) in &messages {
            assert_eq!(message.to_string(), *text);
            assert!(registered.iter().any(|e| e.code == message.code()));
        }
        assert_eq!(registered.len(), messages.len());
    }

    fn command_texts() -> Vec<(UpdateText, &'static str)> {
        vec![
            (
                UpdateText::UpToDate {
                    current: s("3.1.0"),
                },
                "slopvac 3.1.0 is up to date.",
            ),
            (
                UpdateText::Updated {
                    current: s("3.1.0"),
                    latest: s("3.2.0"),
                },
                "Updated slopvac 3.1.0 to 3.2.0.",
            ),
            (
                UpdateText::AdviceRun {
                    command: s("mise upgrade slopvac"),
                },
                "run `mise upgrade slopvac`",
            ),
            (
                UpdateText::UpgradeWith {
                    command: s("mise upgrade slopvac"),
                },
                "Upgrade with `mise upgrade slopvac`.",
            ),
            (
                UpdateText::AdviceAndHint {
                    advice: s("A."),
                    hint: s("H."),
                },
                "A. H.",
            ),
            (
                UpdateText::FetchTooLarge { limit: 4096 },
                "response is larger than 4096 bytes",
            ),
        ]
    }

    fn pipeline_texts() -> Vec<(UpdateText, &'static str)> {
        vec![
            (
                UpdateText::DownloadFailed {
                    asset: s("a.tar.gz"),
                    detail: s("boom"),
                },
                "download of a.tar.gz failed: boom",
            ),
            (
                UpdateText::NotNewer {
                    requested: s("3.0.0"),
                    current: s("3.1.0"),
                },
                "refusing to install 3.0.0: it is not newer than the running 3.1.0",
            ),
            (
                UpdateText::Prerelease {
                    version: s("3.2.0-rc.1"),
                },
                "refusing to install 3.2.0-rc.1: it is a prerelease, and self update installs stable releases only",
            ),
            (
                UpdateText::VerifyRefused { detail: s("d") },
                "refusing to install: d",
            ),
            (
                UpdateText::ArchiveUnreadable { detail: s("d") },
                "cannot read the archive: d",
            ),
            (
                UpdateText::ReplaceFailed { detail: s("d") },
                "cannot replace the installed binary: d",
            ),
            (
                UpdateText::VerifyInvalid { detail: s("d") },
                "invalid signature or key: d",
            ),
            (
                UpdateText::VerifyWrongRelease {
                    expected: s("e"),
                    signed: s("g"),
                },
                "the signature is for a different release or file: expected `e`, signed `g`",
            ),
            (
                UpdateText::PolicyAtPath {
                    path: PathBuf::from("a/slopvac.toml"),
                    problem: s("p"),
                },
                "a/slopvac.toml: p",
            ),
            (
                UpdateText::HintRemoveLock {
                    name: s("slopvac.update.lock"),
                },
                "No other update is running, so remove slopvac.update.lock beside it and run the update again.",
            ),
            (
                UpdateText::HintCheckThenRemove {
                    name: s("slopvac.update.lock"),
                },
                "Make sure that no slopvac update is running before you remove slopvac.update.lock, then run the update again.",
            ),
        ]
    }

    fn lock_texts() -> Vec<(UpdateText, &'static str)> {
        vec![
            (
                UpdateText::LockIo {
                    file: s("slopvac.update.lock"),
                    detail: s("denied"),
                },
                "cannot use the update lock slopvac.update.lock: denied",
            ),
            (
                UpdateText::VersionReason {
                    name: s("slopvac"),
                    reason: s("it exited with exit status: 1"),
                },
                "slopvac: it exited with exit status: 1",
            ),
            (
                UpdateText::ProbeSpawn {
                    detail: s("denied"),
                },
                "it did not start: denied",
            ),
            (
                UpdateText::ProbeTimeout { seconds: s("0.05") },
                "it did not answer within 0.05 s",
            ),
            (
                UpdateText::ProbeFailed {
                    status: s("exit status: 1"),
                },
                "it exited with exit status: 1",
            ),
            (
                UpdateText::ProbeTooLarge { limit: 65536 },
                "its output is larger than 65536 bytes",
            ),
            (
                UpdateText::ProbeRead { detail: s("eof") },
                "cannot read its output: eof",
            ),
            (
                UpdateText::PolicyUnreadable {
                    reason: s("denied"),
                },
                "cannot read: denied",
            ),
        ]
    }

    #[test]
    fn every_sentence_keeps_its_published_text_and_is_registered() {
        let texts: Vec<(UpdateText, &str)> = command_texts()
            .into_iter()
            .chain(pipeline_texts())
            .chain(lock_texts())
            .collect();
        let registered = text_entries();
        for (text, expected) in &texts {
            assert_eq!(text.to_string(), *expected);
            assert!(registered.iter().any(|e| e.code == text.code()));
        }
        assert_eq!(registered.len(), texts.len());
    }

    #[test]
    fn every_phrase_keeps_its_published_text_and_is_registered() {
        let phrases = [
            (UpdatePhrase::SelfAbout, "Manage this installation."),
            (
                UpdatePhrase::SelfUpdateAbout,
                "Update slopvac to the latest release.",
            ),
            (
                UpdatePhrase::SelfUpdateLongAbout,
                "Update slopvac to the latest release. Works for installs made by the slopvac installer. For any other install it prints the command that upgrades it. The download must carry a valid minisign signature, or nothing is replaced.",
            ),
            (UpdatePhrase::UnknownSubcommand, "unknown `self` subcommand"),
            (
                UpdatePhrase::AdviceDownload,
                "download a release from https://github.com/slopvac/slopvac/releases",
            ),
            (
                UpdatePhrase::SeeReleases,
                "See https://github.com/slopvac/slopvac/releases.",
            ),
            (
                UpdatePhrase::MigrateHint,
                "This is a new major version. Run `slopvac migrate` to update your configuration.",
            ),
            (UpdatePhrase::FetchNotHttps, "refusing a non-HTTPS URL"),
            (
                UpdatePhrase::FetchRedirectHost,
                "refusing a redirect to a host that does not serve GitHub releases",
            ),
            (UpdatePhrase::FetchTooManyRedirects, "too many redirects"),
            (UpdatePhrase::FetchRedirectsRefused, "redirects are refused"),
            (
                UpdatePhrase::FetchRedirectNoLocation,
                "redirect without a Location",
            ),
            (
                UpdatePhrase::UnsupportedPlatform,
                "self update does not support this platform; download a release archive instead",
            ),
            (
                UpdatePhrase::NoBinary,
                "the archive does not contain a `slopvac` binary",
            ),
            (UpdatePhrase::BinaryTooLarge, "binary is too large"),
            (UpdatePhrase::NoParentDirectory, "no parent directory"),
            (UpdatePhrase::LockNotRegular, "it is not a regular file"),
            (
                UpdatePhrase::LockHardLinks,
                "it has more than one hard link",
            ),
            (UpdatePhrase::LockOtherOwner, "it is owned by another user"),
            (UpdatePhrase::LockLooseMode, "its mode is looser than 0600"),
            (UpdatePhrase::LockSymlink, "it is a symbolic link"),
            (
                UpdatePhrase::LockBinaryChanged,
                "the installed binary changed after its version was checked",
            ),
            (
                UpdatePhrase::LockBinaryNotWritten,
                "the installed binary is not the build this update wrote",
            ),
            (
                UpdatePhrase::ProbeUnparseable,
                "its output is not a slopvac version document",
            ),
            (
                UpdatePhrase::HintWait,
                "Another slopvac update is running; wait for it to finish, then run the update again.",
            ),
            (
                UpdatePhrase::RegistryUnexpected,
                "unexpected registry response",
            ),
            (
                UpdatePhrase::VerifyMissing,
                "the release has no signature for this artifact",
            ),
            (
                UpdatePhrase::VerifyRejected,
                "the signature does not verify",
            ),
            (UpdatePhrase::VerifyNotUtf8, "signature is not UTF-8"),
            (UpdatePhrase::PolicyNotRegular, "not a regular file"),
        ];
        let registered = phrase_entries();
        for (phrase, expected) in phrases {
            assert_eq!(phrase.as_static(), expected);
            assert!(registered.iter().any(|e| e.code == phrase.code()));
        }
        assert_eq!(registered.len(), phrases.len());
    }
}
