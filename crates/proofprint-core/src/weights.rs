//! Weight manifests: a SHA-256 per tensor and one digest for the whole set.
//!
//! Recorders compute manifests where the tensors live. This crate builds small
//! ones for examples and tests, and checks that an attached manifest hashes to
//! the digest its record states. Format: `spec/v1/training.md`.

use std::io::Read;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::canonical;
use crate::error::{Error, Result};

/// Prefix of every manifest and tensor-set digest.
pub const DIGEST_PREFIX: &str = "sha256:";

/// One tensor: its name, safetensors dtype name, shape, and the SHA-256 of its bytes.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct TensorDigest {
    pub name: String,
    pub dtype: String,
    pub shape: Vec<u64>,
    pub sha256: String,
}

impl TensorDigest {
    /// Describe a tensor from its raw little-endian, row-major bytes.
    pub fn of_bytes(
        name: impl Into<String>,
        dtype: impl Into<String>,
        shape: Vec<u64>,
        bytes: &[u8],
    ) -> Self {
        Self {
            name: name.into(),
            dtype: dtype.into(),
            shape,
            sha256: hex::encode(Sha256::digest(bytes)),
        }
    }
}

/// Every tensor of one model state, sorted by name.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct WeightsManifest {
    tensors: Vec<TensorDigest>,
}

impl WeightsManifest {
    /// Sort by name. Two tensors with one name are an error.
    pub fn new(mut tensors: Vec<TensorDigest>) -> Result<Self> {
        tensors.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
        if let Some(pair) = tensors.windows(2).find(|pair| pair[0].name == pair[1].name) {
            return Err(Error::MalformedRecord(format!(
                "tensor {:?} appears twice in a manifest",
                pair[0].name
            )));
        }
        Ok(Self { tensors })
    }

    pub fn tensors(&self) -> &[TensorDigest] {
        &self.tensors
    }

    /// The bytes a manifest attachment holds.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        canonical::encode(self)
    }

    /// `sha256:<hex>` of the canonical bytes.
    pub fn digest(&self) -> Result<String> {
        Ok(digest_bytes(&self.canonical_bytes()?))
    }
}

/// `sha256:<hex>` of `bytes`.
pub fn digest_bytes(bytes: &[u8]) -> String {
    format!("{DIGEST_PREFIX}{}", hex::encode(Sha256::digest(bytes)))
}

/// `sha256:<hex>` of everything `reader` yields, streamed.
pub fn digest_reader(mut reader: impl Read) -> Result<String> {
    let mut hasher = Sha256::new();
    std::io::copy(&mut reader, &mut hasher)?;
    Ok(format!("{DIGEST_PREFIX}{}", hex::encode(hasher.finalize())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> WeightsManifest {
        WeightsManifest::new(vec![
            TensorDigest::of_bytes(
                "layer.weight",
                "F32",
                vec![2],
                &[0, 0, 128, 63, 0, 0, 0, 64],
            ),
            TensorDigest::of_bytes("layer.bias", "F32", vec![], &[0, 0, 0, 0]),
        ])
        .unwrap()
    }

    #[test]
    fn tensors_are_sorted_and_encoded_canonically() {
        let text = String::from_utf8(manifest().canonical_bytes().unwrap()).unwrap();
        assert_eq!(
            text,
            concat!(
                r#"{"tensors":[{"dtype":"F32","name":"layer.bias","#,
                r#""sha256":"df3f619804a92fdb4057192dc43dd748ea778adc52bc498ce80524c014b81119","shape":[]},"#,
                r#"{"dtype":"F32","name":"layer.weight","#,
                r#""sha256":"b9c80b5adeca450753a16950c3cc655d271f7bef7a485bc83f112b72fef21d37","shape":[2]}]}"#,
            )
        );
    }

    #[test]
    fn digest_covers_the_canonical_bytes() {
        let manifest = manifest();
        let bytes = manifest.canonical_bytes().unwrap();
        assert_eq!(manifest.digest().unwrap(), digest_bytes(&bytes));
        assert_eq!(
            digest_reader(bytes.as_slice()).unwrap(),
            digest_bytes(&bytes)
        );
        assert!(manifest.digest().unwrap().starts_with("sha256:"));
    }

    #[test]
    fn duplicate_names_are_refused() {
        let tensor = TensorDigest::of_bytes("w", "F32", vec![], &[0; 4]);
        assert!(WeightsManifest::new(vec![tensor.clone(), tensor]).is_err());
    }
}
