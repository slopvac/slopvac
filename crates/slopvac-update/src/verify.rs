//! The verification step of `self update`.
//!
//! The pipeline calls [`ArtifactVerifier::verify`] on the downloaded bytes before it extracts or
//! replaces anything. A verifier that cannot say yes must say no: the pipeline treats every error
//! as fatal and leaves the installed binary alone.

use thiserror::Error;

use crate::messages::{UpdatePhrase, UpdateText};

/// Why an artifact was not accepted.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum VerifyError {
    /// No signature was published for the artifact.
    #[error("{}", UpdatePhrase::VerifyMissing.as_static())]
    MissingSignature,
    /// The signature or the key does not parse.
    #[error("{}", invalid_text(.0))]
    Invalid(String),
    /// The signature does not match the bytes.
    #[error("{}", UpdatePhrase::VerifyRejected.as_static())]
    Rejected,
    /// The signature is valid but names a different file or release than the one requested.
    #[error("{}", wrong_release_text(.expected, .signed))]
    WrongRelease {
        /// The trusted comment this download must carry.
        expected: String,
        /// The trusted comment the signature carries (empty if it has none).
        signed: String,
    },
}

fn invalid_text(detail: &str) -> String {
    UpdateText::VerifyInvalid {
        detail: detail.to_owned(),
    }
    .to_string()
}

fn wrong_release_text(expected: &str, signed: &str) -> String {
    UpdateText::VerifyWrongRelease {
        expected: expected.to_owned(),
        signed: signed.to_owned(),
    }
    .to_string()
}

/// Which release file a download must be: the identity that release signing binds in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReleaseFile<'a> {
    /// Asset file name, for example `slopvac-x86_64-unknown-linux-gnu.tar.gz`.
    pub name: &'a str,
    /// Release version without the `v`, for example `3.1.0`.
    pub version: &'a str,
}

impl ReleaseFile<'_> {
    /// The minisign trusted comment that release CI signs this file with
    /// (`scripts/release_signing.py`). Minisign's global signature covers it, so a verified
    /// signature proves the key owner signed exactly this file of exactly this release.
    #[must_use]
    pub fn trusted_comment(&self) -> String {
        // ast-grep-ignore: user-text-literal
        format!(
            "slopvac-release file:{} version:{}",
            self.name, self.version
        )
    }
}

/// Decides whether downloaded bytes may be installed.
pub trait ArtifactVerifier {
    /// Check `artifact` against the signature text (the `.minisig` file), and require that the
    /// signature was made for `expected`.
    fn verify(
        &self,
        artifact: &[u8],
        signature: &[u8],
        expected: &ReleaseFile<'_>,
    ) -> Result<(), VerifyError>;
}

/// Minisign verification with one public key.
#[derive(Debug, Clone)]
pub struct MinisignVerifier {
    key: minisign_verify::PublicKey,
}

impl MinisignVerifier {
    /// Parse a whole `minisign.pub` file: the comment line and the base64 line.
    pub fn from_pub_file(text: &str) -> Result<Self, VerifyError> {
        minisign_verify::PublicKey::decode(text)
            .map(|key| Self { key })
            .map_err(|e| VerifyError::Invalid(e.to_string()))
    }

    /// Parse the base64 line of a `minisign.pub` file.
    pub fn from_base64(key: &str) -> Result<Self, VerifyError> {
        minisign_verify::PublicKey::from_base64(key.trim())
            .map(|key| Self { key })
            .map_err(|e| VerifyError::Invalid(e.to_string()))
    }
}

impl ArtifactVerifier for MinisignVerifier {
    fn verify(
        &self,
        artifact: &[u8],
        signature: &[u8],
        expected: &ReleaseFile<'_>,
    ) -> Result<(), VerifyError> {
        let text = std::str::from_utf8(signature).map_err(|_| {
            VerifyError::Invalid(UpdatePhrase::VerifyNotUtf8.as_static().to_owned())
        })?;
        let sig = minisign_verify::Signature::decode(text)
            .map_err(|e| VerifyError::Invalid(e.to_string()))?;
        // `allow_legacy = false`: only prehashed signatures verify, which is what release CI
        // writes (`docs/release/distribution.md`). A legacy signature is rejected.
        self.key
            .verify(artifact, &sig, false)
            .map_err(|_| VerifyError::Rejected)?;
        // The global signature, checked above, covers the trusted comment, so it is authentic
        // here. Without this check an old signed archive would pass for a new release.
        let wanted = expected.trusted_comment();
        if sig.trusted_comment() != wanted {
            return Err(VerifyError::WrongRelease {
                expected: wanted,
                signed: sig.trusted_comment().to_owned(),
            });
        }
        Ok(())
    }
}

/// The current release public key, compiled in from `keys/slopvac-release.pub`. The
/// `slopvac-plugins` crate includes the same file for first-party pack verification.
pub const RELEASE_KEY_FILE: &str = include_str!("../../../keys/slopvac-release.pub");

/// The pre-published next release key, as the text of a minisign public key file.
///
/// It is `None` until a maintainer adds the real key in a rotation release; agents never
/// generate or commit release keys. The rotation procedure is in
/// `docs/release/maintainer-setup.md`.
pub const NEXT_RELEASE_KEY_FILE: Option<&str> = None;

/// The compiled-in trusted key files: the current key, then the next key when one is published.
fn trusted_key_files() -> Vec<String> {
    std::iter::once(RELEASE_KEY_FILE)
        .chain(NEXT_RELEASE_KEY_FILE)
        .map(str::to_owned)
        .collect()
}

/// The verifier that real downloads use: [`MinisignVerifier`] over the trusted release keys.
///
/// A signature from any trusted key is accepted, and the trusted-comment check applies to it.
/// A client that already embeds the next key therefore accepts the release that the next key
/// signs once release CI switches to it (`docs/release/maintainer-setup.md`, "Rotate the key").
///
/// It fails closed. If a non-blank trusted key does not parse, every artifact is refused with
/// [`VerifyError::Invalid`]. Tests swap the keys through `ReleaseVerifier::with_key_files`, which
/// exists only in test builds of this crate, so no shipped code path takes another key.
#[derive(Debug, Clone)]
pub struct ReleaseVerifier {
    key_files: Vec<String>,
}

impl Default for ReleaseVerifier {
    fn default() -> Self {
        Self {
            key_files: trusted_key_files(),
        }
    }
}

impl ReleaseVerifier {
    #[cfg(test)]
    fn with_key_files<S: AsRef<str>>(key_files: impl IntoIterator<Item = S>) -> Self {
        Self {
            key_files: key_files
                .into_iter()
                .map(|f| f.as_ref().to_owned())
                .collect(),
        }
    }
}

impl ArtifactVerifier for ReleaseVerifier {
    fn verify(
        &self,
        artifact: &[u8],
        signature: &[u8],
        expected: &ReleaseFile<'_>,
    ) -> Result<(), VerifyError> {
        let verifiers = self
            .key_files
            .iter()
            .map(|f| MinisignVerifier::from_pub_file(f))
            .collect::<Result<Vec<_>, _>>()?;
        // A key that did not sign the file reports `Rejected`. Keep a more specific error, such
        // as a signature that verifies but names another release, in preference to it.
        let mut failure = VerifyError::Rejected;
        for v in &verifiers {
            match v.verify(artifact, signature, expected) {
                Ok(()) => return Ok(()),
                Err(VerifyError::Rejected) => {}
                Err(e) => failure = e,
            }
        }
        Err(failure)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAME: &str = "slopvac-x86_64-unknown-linux-gnu.tar.gz";
    const FILE: ReleaseFile<'static> = ReleaseFile {
        name: NAME,
        version: "3.1.0",
    };

    /// Sign `data` the way release CI does, with `comment` as the trusted comment.
    fn sign(kp: &minisign::KeyPair, data: &[u8], comment: Option<&str>) -> String {
        minisign::sign(None, &kp.sk, std::io::Cursor::new(data), comment, None)
            .expect("sign")
            .to_string()
    }

    fn keypair() -> minisign::KeyPair {
        minisign::KeyPair::generate_unencrypted_keypair().expect("keypair")
    }

    fn verifier(kp: &minisign::KeyPair) -> MinisignVerifier {
        MinisignVerifier::from_base64(&kp.pk.to_base64()).unwrap()
    }

    fn release_sig(kp: &minisign::KeyPair, data: &[u8]) -> String {
        sign(kp, data, Some(&FILE.trusted_comment()))
    }

    #[test]
    fn trusted_comment_format_is_the_one_release_signing_writes() {
        assert_eq!(
            FILE.trusted_comment(),
            "slopvac-release file:slopvac-x86_64-unknown-linux-gnu.tar.gz version:3.1.0"
        );
    }

    #[test]
    fn accepts_the_signed_bytes_for_the_requested_release() {
        let kp = keypair();
        let sig = release_sig(&kp, b"artifact");
        assert_eq!(
            verifier(&kp).verify(b"artifact", sig.as_bytes(), &FILE),
            Ok(())
        );
    }

    #[test]
    fn rejects_tampered_bytes() {
        let kp = keypair();
        let sig = release_sig(&kp, b"artifact");
        assert_eq!(
            verifier(&kp).verify(b"tampered bytes", sig.as_bytes(), &FILE),
            Err(VerifyError::Rejected)
        );
    }

    #[test]
    fn rejects_a_signature_from_another_key() {
        let sig = release_sig(&keypair(), b"artifact");
        assert_eq!(
            verifier(&keypair()).verify(b"artifact", sig.as_bytes(), &FILE),
            Err(VerifyError::Rejected)
        );
    }

    #[test]
    fn rejects_an_older_release_served_as_the_requested_one() {
        let kp = keypair();
        let older = ReleaseFile {
            version: "3.0.0",
            ..FILE
        };
        // A genuine signature over genuine bytes, made for 3.0.0.
        let sig = sign(&kp, b"artifact", Some(&older.trusted_comment()));
        assert_eq!(
            verifier(&kp).verify(b"artifact", sig.as_bytes(), &FILE),
            Err(VerifyError::WrongRelease {
                expected: FILE.trusted_comment(),
                signed: older.trusted_comment(),
            })
        );
    }

    #[test]
    fn rejects_another_targets_archive_served_as_the_requested_one() {
        let kp = keypair();
        let other = ReleaseFile {
            name: "slopvac-aarch64-apple-darwin.tar.gz",
            ..FILE
        };
        let sig = sign(&kp, b"artifact", Some(&other.trusted_comment()));
        assert!(matches!(
            verifier(&kp).verify(b"artifact", sig.as_bytes(), &FILE),
            Err(VerifyError::WrongRelease { .. })
        ));
    }

    #[test]
    fn rejects_a_signature_without_the_release_trusted_comment() {
        let kp = keypair();
        // minisign's default trusted comment is a timestamp and a file name.
        let sig = sign(&kp, b"artifact", None);
        assert!(matches!(
            verifier(&kp).verify(b"artifact", sig.as_bytes(), &FILE),
            Err(VerifyError::WrongRelease { .. })
        ));
    }

    #[test]
    fn rejects_a_garbled_trusted_comment() {
        let kp = keypair();
        let good = FILE.trusted_comment();
        for garbled in [
            String::new(),
            good.to_uppercase(),
            format!("{good} extra"),
            format!("{good}\tfile:other"),
            good.replace("slopvac-release", "other-release"),
            good.replace(" version:", " ver:"),
            format!("{good} version:3.1.0"),
            format!(" {good}"),
            format!("{good} "),
            format!("{good}\t"),
            good.replace(' ', "  "),
        ] {
            let sig = sign(&kp, b"artifact", Some(&garbled));
            assert!(
                matches!(
                    verifier(&kp).verify(b"artifact", sig.as_bytes(), &FILE),
                    Err(VerifyError::WrongRelease { .. })
                ),
                "{garbled:?}"
            );
        }
    }

    #[test]
    fn a_trusted_comment_edited_after_signing_fails_the_signature() {
        let kp = keypair();
        let older = ReleaseFile {
            version: "3.0.0",
            ..FILE
        };
        let sig = sign(&kp, b"artifact", Some(&older.trusted_comment()))
            .replace("version:3.0.0", "version:3.1.0");
        assert_eq!(
            verifier(&kp).verify(b"artifact", sig.as_bytes(), &FILE),
            Err(VerifyError::Rejected)
        );
    }

    #[test]
    fn compiled_release_key_parses_and_has_the_published_id() {
        MinisignVerifier::from_pub_file(RELEASE_KEY_FILE).expect("compiled key parses");
        let pk = minisign::PublicKeyBox::from_string(RELEASE_KEY_FILE)
            .and_then(minisign::PublicKeyBox::into_public_key)
            .expect("minisign parses the key file");
        // The key id is stored little-endian; minisign prints it as big-endian hex.
        let id = pk
            .keynum()
            .iter()
            .rev()
            .fold(String::new(), |acc, b| acc + &format!("{b:02X}"));
        assert_eq!(id, "D9914BDB4570AC6D");
    }

    // A fixture that `scripts/release_signing.py sign --version 3.1.0` produced with the real
    // minisign CLI. The key is a throwaway, test-only key (`tests/fixtures/README.md`).
    const FIXTURE_KEY: &str = include_str!("../tests/fixtures/test-only.pub");
    const FIXTURE_ARCHIVE: &[u8] =
        include_bytes!("../tests/fixtures/slopvac-x86_64-unknown-linux-gnu.tar.gz");
    const FIXTURE_SIG: &[u8] =
        include_bytes!("../tests/fixtures/slopvac-x86_64-unknown-linux-gnu.tar.gz.minisig");

    #[test]
    fn release_verifier_accepts_an_artifact_signed_the_way_release_ci_signs() {
        let v = ReleaseVerifier::with_key_files([FIXTURE_KEY]);
        assert_eq!(v.verify(FIXTURE_ARCHIVE, FIXTURE_SIG, &FILE), Ok(()));
    }

    #[test]
    fn release_verifier_rejects_the_fixture_once_tampered() {
        let v = ReleaseVerifier::with_key_files([FIXTURE_KEY]);
        let mut tampered = FIXTURE_ARCHIVE.to_vec();
        tampered.push(0);
        assert_eq!(
            v.verify(&tampered, FIXTURE_SIG, &FILE),
            Err(VerifyError::Rejected)
        );
    }

    #[test]
    fn release_verifier_rejects_the_fixture_for_another_version() {
        let v = ReleaseVerifier::with_key_files([FIXTURE_KEY]);
        let newer = ReleaseFile {
            version: "3.2.0",
            ..FILE
        };
        assert!(matches!(
            v.verify(FIXTURE_ARCHIVE, FIXTURE_SIG, &newer),
            Err(VerifyError::WrongRelease { .. })
        ));
    }

    #[test]
    fn release_verifier_with_the_compiled_key_rejects_the_test_key_fixture() {
        assert_eq!(
            ReleaseVerifier::default().verify(FIXTURE_ARCHIVE, FIXTURE_SIG, &FILE),
            Err(VerifyError::Rejected)
        );
    }

    #[test]
    fn release_verifier_rejects_a_garbage_signature() {
        assert!(matches!(
            ReleaseVerifier::default().verify(b"artifact", b"not a signature", &FILE),
            Err(VerifyError::Invalid(_))
        ));
    }

    /// A public key file for `kp`, as `keys/*.pub` holds it.
    fn key_file(kp: &minisign::KeyPair) -> String {
        kp.pk.to_box().unwrap().to_string()
    }

    #[test]
    fn without_a_published_next_key_only_the_current_key_is_trusted() {
        assert_eq!(trusted_key_files(), [RELEASE_KEY_FILE.to_owned()]);
    }

    // The two-key transition. `current` signs the rotation release, `next` signs later ones.
    #[test]
    fn a_client_with_both_keys_accepts_a_release_signed_by_either() {
        let (current, next) = (keypair(), keypair());
        let v = ReleaseVerifier::with_key_files([key_file(&current), key_file(&next)]);
        for signer in [&current, &next] {
            let sig = release_sig(signer, b"artifact");
            assert_eq!(v.verify(b"artifact", sig.as_bytes(), &FILE), Ok(()));
        }
    }

    #[test]
    fn a_client_without_the_next_key_rejects_a_release_signed_by_it() {
        let (current, next) = (keypair(), keypair());
        let v = ReleaseVerifier::with_key_files([key_file(&current)]);
        let sig = release_sig(&next, b"artifact");
        assert_eq!(
            v.verify(b"artifact", sig.as_bytes(), &FILE),
            Err(VerifyError::Rejected)
        );
    }

    #[test]
    fn a_key_outside_the_trusted_list_is_rejected() {
        let (current, next) = (keypair(), keypair());
        let v = ReleaseVerifier::with_key_files([key_file(&current), key_file(&next)]);
        let sig = release_sig(&keypair(), b"artifact");
        assert_eq!(
            v.verify(b"artifact", sig.as_bytes(), &FILE),
            Err(VerifyError::Rejected)
        );
    }

    #[test]
    fn the_trusted_comment_is_checked_for_the_next_key_too() {
        let (current, next) = (keypair(), keypair());
        let v = ReleaseVerifier::with_key_files([key_file(&current), key_file(&next)]);
        let older = ReleaseFile {
            version: "3.0.0",
            ..FILE
        };
        let sig = sign(&next, b"artifact", Some(&older.trusted_comment()));
        assert!(matches!(
            v.verify(b"artifact", sig.as_bytes(), &FILE),
            Err(VerifyError::WrongRelease { .. })
        ));
    }

    #[test]
    fn an_unparsable_trusted_key_refuses_everything() {
        let current = keypair();
        let v = ReleaseVerifier::with_key_files([key_file(&current), "not a key".to_owned()]);
        let sig = release_sig(&current, b"artifact");
        assert!(matches!(
            v.verify(b"artifact", sig.as_bytes(), &FILE),
            Err(VerifyError::Invalid(_))
        ));
    }

    #[test]
    fn pub_file_parsing_accepts_a_whole_key_file() {
        let kp = keypair();
        let file = kp.pk.to_box().unwrap().to_string();
        let sig = release_sig(&kp, b"a");
        let v = MinisignVerifier::from_pub_file(&file).unwrap();
        assert_eq!(v.verify(b"a", sig.as_bytes(), &FILE), Ok(()));
    }

    /// A genuine non-prehashed signature of `test`, from the `minisign-verify` test vectors.
    // Split so the typos check does not read the base64 as a word.
    const LEGACY_KEY: &str = concat!(
        "RWQf6LRCGA9i53mlYecO4I",
        "zT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3"
    );
    const LEGACY_SIG: &str = "untrusted comment: signature from minisign secret key
RWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=
trusted comment: timestamp:1555779966\tfile:test
QtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==";

    #[test]
    fn rejects_a_legacy_signature_that_is_otherwise_valid() {
        // Precondition: the signature is genuine, so only the legacy check can reject it.
        let key = minisign_verify::PublicKey::from_base64(LEGACY_KEY).unwrap();
        let sig = minisign_verify::Signature::decode(LEGACY_SIG).unwrap();
        key.verify(b"test", &sig, true)
            .expect("valid when legacy is allowed");

        let v = MinisignVerifier::from_base64(LEGACY_KEY).unwrap();
        assert_eq!(
            v.verify(b"test", LEGACY_SIG.as_bytes(), &FILE),
            Err(VerifyError::Rejected)
        );
    }
}
