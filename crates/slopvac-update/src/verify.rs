//! The verification step of `self update`.
//!
//! The pipeline calls [`ArtifactVerifier::verify`] on the downloaded bytes before it extracts or
//! replaces anything. A verifier that cannot say yes must say no: the pipeline treats every error
//! as fatal and leaves the installed binary alone.

use thiserror::Error;

/// Why an artifact was not accepted.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum VerifyError {
    /// No signature was published for the artifact.
    #[error("the release has no signature for this artifact")]
    MissingSignature,
    /// The signature or the key does not parse.
    #[error("invalid signature or key: {0}")]
    Invalid(String),
    /// The signature does not match the bytes.
    #[error("the signature does not verify")]
    Rejected,
}

/// Decides whether downloaded bytes may be installed.
pub trait ArtifactVerifier {
    /// Check `artifact` against the signature text (the `.minisig` file).
    fn verify(&self, artifact: &[u8], signature: &[u8]) -> Result<(), VerifyError>;
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
    fn verify(&self, artifact: &[u8], signature: &[u8]) -> Result<(), VerifyError> {
        let text = std::str::from_utf8(signature)
            .map_err(|_| VerifyError::Invalid("signature is not UTF-8".into()))?;
        let sig = minisign_verify::Signature::decode(text)
            .map_err(|e| VerifyError::Invalid(e.to_string()))?;
        // `allow_legacy = false`: only prehashed signatures verify, which is what release CI
        // writes (`docs/release/distribution.md`). A legacy signature is rejected.
        self.key
            .verify(artifact, &sig, false)
            .map_err(|_| VerifyError::Rejected)
    }
}

/// The release public key, compiled in from `keys/slopvac-release.pub`. The `slopvac-plugins`
/// crate includes the same file for first-party pack verification, so there is one key to rotate.
pub const RELEASE_KEY_FILE: &str = include_str!("../../../keys/slopvac-release.pub");

/// The verifier that real downloads use: [`MinisignVerifier`] with the compiled-in release key.
///
/// It fails closed. If the compiled key does not parse, every artifact is refused with
/// [`VerifyError::Invalid`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ReleaseVerifier;

impl ArtifactVerifier for ReleaseVerifier {
    fn verify(&self, artifact: &[u8], signature: &[u8]) -> Result<(), VerifyError> {
        MinisignVerifier::from_pub_file(RELEASE_KEY_FILE)?.verify(artifact, signature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed(data: &[u8]) -> (String, String) {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().expect("keypair");
        let sig = minisign::sign(None, &kp.sk, std::io::Cursor::new(data), None, None)
            .expect("sign")
            .to_string();
        (kp.pk.to_base64(), sig)
    }

    #[test]
    fn accepts_the_signed_bytes() {
        let (pk, sig) = signed(b"artifact");
        let v = MinisignVerifier::from_base64(&pk).unwrap();
        assert_eq!(v.verify(b"artifact", sig.as_bytes()), Ok(()));
    }

    #[test]
    fn rejects_tampered_bytes() {
        let (pk, sig) = signed(b"artifact");
        let v = MinisignVerifier::from_base64(&pk).unwrap();
        assert_eq!(
            v.verify(b"tampered bytes", sig.as_bytes()),
            Err(VerifyError::Rejected)
        );
    }

    #[test]
    fn rejects_a_signature_from_another_key() {
        let (_, sig) = signed(b"artifact");
        let (other_pk, _) = signed(b"x");
        let v = MinisignVerifier::from_base64(&other_pk).unwrap();
        assert_eq!(
            v.verify(b"artifact", sig.as_bytes()),
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

    #[test]
    fn release_verifier_rejects_a_signature_from_another_key() {
        let (_, sig) = signed(b"artifact");
        assert!(matches!(
            ReleaseVerifier.verify(b"artifact", sig.as_bytes()),
            Err(VerifyError::Rejected)
        ));
    }

    #[test]
    fn release_verifier_rejects_a_garbage_signature() {
        assert!(matches!(
            ReleaseVerifier.verify(b"artifact", b"not a signature"),
            Err(VerifyError::Invalid(_))
        ));
    }

    #[test]
    fn pub_file_parsing_accepts_a_whole_key_file() {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let file = kp.pk.to_box().unwrap().to_string();
        let sig = minisign::sign(None, &kp.sk, std::io::Cursor::new(b"a"), None, None)
            .unwrap()
            .to_string();
        let v = MinisignVerifier::from_pub_file(&file).unwrap();
        assert_eq!(v.verify(b"a", sig.as_bytes()), Ok(()));
    }

    /// A genuine non-prehashed signature of `test`, from the `minisign-verify` test vectors.
    const LEGACY_KEY: &str = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
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
            v.verify(b"test", LEGACY_SIG.as_bytes()),
            Err(VerifyError::Rejected)
        );
    }
}
