//! Ed25519 signing keys and the `signer` string format.
//!
//! For v1 a signer is rendered as `ed25519:<64 lowercase hex>`. Aligning this
//! with `did:key` / W3C Verifiable Credentials is tracked as future work; the
//! raw form keeps the dependency surface small and the ids easy to read.
//!
//! # Key custody
//!
//! [`KeyPair::save`] writes the **unencrypted 32-byte seed** and restricts file
//! permissions where the platform allows it. That is fine for development and
//! for keys whose compromise is not catastrophic. Production publishers should
//! keep the seed in a KMS/HSM and expose only a signing operation.

use std::fs;
use std::io::Write;
use std::path::Path;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand_core::OsRng;

use crate::error::{Error, Result};

/// Prefix of the v1 signer string format.
pub const SIGNER_PREFIX: &str = "ed25519:";

const SEED_LEN: usize = 32;
const PUBKEY_HEX_LEN: usize = 64;

/// A signing key and its public counterpart.
#[derive(Clone)]
pub struct KeyPair {
    signing: SigningKey,
}

impl KeyPair {
    /// Generate a fresh key from the OS CSPRNG.
    pub fn generate() -> Self {
        Self {
            signing: SigningKey::generate(&mut OsRng),
        }
    }

    /// Reconstruct from a raw 32-byte seed.
    pub fn from_seed(seed: [u8; SEED_LEN]) -> Self {
        Self {
            signing: SigningKey::from_bytes(&seed),
        }
    }

    /// The raw seed. Handle with care.
    pub fn to_seed(&self) -> [u8; SEED_LEN] {
        self.signing.to_bytes()
    }

    /// The public key.
    pub fn public_key(&self) -> PublicKey {
        PublicKey(self.signing.verifying_key())
    }

    /// The `ed25519:<hex>` signer string for this key.
    pub fn signer(&self) -> String {
        self.public_key().to_signer()
    }

    /// Sign `message`.
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signing.sign(message)
    }

    /// Atomically save a new key without replacing an existing file.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        self.save_to(path.as_ref(), false)
    }

    /// Atomically replace a key file when replacement is explicitly requested.
    pub fn replace(&self, path: impl AsRef<Path>) -> Result<()> {
        self.save_to(path.as_ref(), true)
    }

    fn save_to(&self, path: &Path, replace: bool) -> Result<()> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        let mut staging = tempfile::NamedTempFile::new_in(parent)?;
        staging.write_all(&self.signing.to_bytes())?;
        staging.as_file().sync_all()?;
        // NamedTempFile uses owner-only permissions on Unix. Windows uses the
        // parent directory's inherited ACL, as does the rest of the local store.
        if replace {
            staging.persist(path).map_err(|e| Error::Io(e.error))?;
        } else {
            staging
                .persist_noclobber(path)
                .map_err(|e| Error::Io(e.error))?;
        }
        Ok(())
    }

    /// Load a seed written by [`KeyPair::save`].
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = fs::read(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::KeyNotFound(path.to_path_buf()),
            _ => Error::Io(e),
        })?;
        if bytes.len() != SEED_LEN {
            return Err(Error::InvalidKey(format!(
                "expected {SEED_LEN} bytes, found {}",
                bytes.len()
            )));
        }
        let mut seed = [0u8; SEED_LEN];
        seed.copy_from_slice(&bytes);
        Ok(Self::from_seed(seed))
    }
}

/// An Ed25519 public key.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicKey(VerifyingKey);

impl PublicKey {
    /// Parse an `ed25519:<hex>` signer string.
    pub fn parse(signer: &str) -> Result<Self> {
        let Some(hexed) = signer.trim().strip_prefix(SIGNER_PREFIX) else {
            return Err(Error::InvalidSigner(format!(
                "{signer:?} does not start with {SIGNER_PREFIX:?}"
            )));
        };
        if hexed.len() != PUBKEY_HEX_LEN {
            return Err(Error::InvalidSigner(format!(
                "{signer:?} has {} hex chars, expected {PUBKEY_HEX_LEN}",
                hexed.len()
            )));
        }
        let mut raw = [0u8; 32];
        hex::decode_to_slice(hexed, &mut raw)
            .map_err(|_| Error::InvalidSigner(signer.to_string()))?;
        VerifyingKey::from_bytes(&raw)
            .map(Self)
            .map_err(|e| Error::InvalidKey(e.to_string()))
    }

    /// Render as `ed25519:<hex>`.
    pub fn to_signer(&self) -> String {
        format!("{SIGNER_PREFIX}{}", hex::encode(self.0.to_bytes()))
    }

    /// Verify `signature` over `message`.
    pub fn verify(&self, message: &[u8], signature: &Signature) -> Result<()> {
        self.0
            .verify_strict(message, signature)
            .map_err(|_| Error::BadSignature)
    }
}

impl std::fmt::Display for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_signer())
    }
}

impl std::fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PublicKey({})", self.to_signer())
    }
}

/// Parse an Ed25519 signature from 64 bytes.
pub fn signature_from_slice(bytes: &[u8]) -> Result<Signature> {
    Signature::from_slice(bytes).map_err(|_| Error::BadSignature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_round_trip() {
        let kp = KeyPair::generate();
        let sig = kp.sign(b"hello");
        assert!(kp.public_key().verify(b"hello", &sig).is_ok());
        assert!(matches!(
            kp.public_key().verify(b"tampered", &sig),
            Err(Error::BadSignature)
        ));
    }

    #[test]
    fn seed_round_trip_preserves_identity() {
        let kp = KeyPair::generate();
        let restored = KeyPair::from_seed(kp.to_seed());
        assert_eq!(restored.signer(), kp.signer());
    }

    #[test]
    fn signer_string_round_trip() {
        let kp = KeyPair::generate();
        let s = kp.signer();
        assert!(s.starts_with(SIGNER_PREFIX));
        assert_eq!(s.len(), SIGNER_PREFIX.len() + PUBKEY_HEX_LEN);
        assert_eq!(PublicKey::parse(&s).unwrap(), kp.public_key());
    }

    #[test]
    fn rejects_bad_signer_strings() {
        assert!(PublicKey::parse("rsa:abcd").is_err());
        assert!(PublicKey::parse("ed25519:zz").is_err());
        assert!(PublicKey::parse(&format!("ed25519:{}", "0".repeat(63))).is_err());
        assert!(PublicKey::parse("ed25519:").is_err());
    }

    #[test]
    fn save_and_load_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("key");
        let kp = KeyPair::generate();
        kp.save(&path).unwrap();
        assert_eq!(KeyPair::load(&path).unwrap().signer(), kp.signer());
    }

    #[test]
    fn save_preserves_existing_key_unless_replacement_is_explicit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.key");
        let first = KeyPair::generate();
        let second = KeyPair::generate();
        first.save(&path).unwrap();
        assert!(second.save(&path).is_err());
        assert_eq!(KeyPair::load(&path).unwrap().signer(), first.signer());
        second.replace(&path).unwrap();
        assert_eq!(KeyPair::load(&path).unwrap().signer(), second.signer());
    }

    #[test]
    fn load_reports_missing_key() {
        assert!(matches!(
            KeyPair::load(Path::new("does/not/exist.key")),
            Err(Error::KeyNotFound(_))
        ));
    }

    #[test]
    fn load_rejects_wrong_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.key");
        fs::write(&path, b"too-short").unwrap();
        assert!(matches!(KeyPair::load(&path), Err(Error::InvalidKey(_))));
    }
}
