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
    /// No release key is embedded in this build yet.
    #[error("this build embeds no release key, so it cannot verify downloads")]
    NoKey,
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
        self.key
            .verify(artifact, &sig, false)
            .map_err(|_| VerifyError::Rejected)
    }
}

/// The verifier that real downloads use.
///
/// STUB(slopvac-8l4.13.25): no release key is embedded yet (`keys/slopvac-release.pub` does not
/// exist), so this rejects every artifact. That fails closed: `slopvac self update` cannot replace
/// the binary until 13.25 embeds the key and returns a [`MinisignVerifier`] here.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReleaseVerifier;

impl ArtifactVerifier for ReleaseVerifier {
    fn verify(&self, _artifact: &[u8], _signature: &[u8]) -> Result<(), VerifyError> {
        Err(VerifyError::NoKey)
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
    fn release_verifier_rejects_everything_until_a_key_is_embedded() {
        let (_, sig) = signed(b"artifact");
        assert_eq!(
            ReleaseVerifier.verify(b"artifact", sig.as_bytes()),
            Err(VerifyError::NoKey)
        );
    }
}
