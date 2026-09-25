//! Content-addressed identifiers.
//!
//! Every identifier is a BLAKE3-256 digest rendered as `prefix:hex`. The prefix
//! is part of the identity, so a blob id can never be silently substituted where
//! a record id is expected.
//!
//! ```text
//! ppb1:4f2a...   blob / attachment content
//! ppr1:9c31...   signed record
//! ```

use std::fmt;
use std::str::FromStr;

use serde::de::{self, Deserialize, Deserializer};
use serde::{Serialize, Serializer};

use crate::error::{Error, Result};

/// Prefix for content-addressed blobs.
pub const BLOB_PREFIX: &str = "ppb1:";
/// Prefix for signed records.
pub const RECORD_PREFIX: &str = "ppr1:";

/// Length of the hex digest (32 bytes).
pub const DIGEST_HEX_LEN: usize = 64;

/// Hash `bytes` and return the raw 32-byte BLAKE3-256 digest.
pub fn digest(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

macro_rules! define_id {
    ($name:ident, $prefix:expr, $invalid:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; 32]);

        impl $name {
            /// Compute the identifier for `bytes`.
            pub fn from_bytes(bytes: &[u8]) -> Self {
                Self(digest(bytes))
            }

            /// Build from a raw digest, without hashing.
            pub fn from_digest(digest: [u8; 32]) -> Self {
                Self(digest)
            }

            /// The raw 32-byte digest.
            pub fn as_digest(&self) -> &[u8; 32] {
                &self.0
            }

            /// Hex digest without the prefix.
            pub fn to_hex(&self) -> String {
                hex::encode(self.0)
            }

            /// Two-byte shard prefix used for on-disk fan-out.
            pub fn shard(&self) -> String {
                let h = hex::encode(self.0);
                h[..2].to_string()
            }

            /// Parse `prefix:hex`, validating both the prefix and the digest.
            pub fn parse(s: &str) -> Result<Self> {
                let s = s.trim();
                let Some(rest) = s.strip_prefix($prefix) else {
                    return Err(Error::$invalid(format!(
                        "{s:?} does not start with {prefix:?}",
                        prefix = $prefix
                    )));
                };
                if rest.len() != DIGEST_HEX_LEN {
                    return Err(Error::$invalid(format!(
                        "{s:?} has {} hex chars, expected {DIGEST_HEX_LEN}",
                        rest.len()
                    )));
                }
                let mut out = [0u8; 32];
                hex::decode_to_slice(rest, &mut out).map_err(|_| Error::$invalid(s.to_string()))?;
                Ok(Self(out))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", $prefix, hex::encode(self.0))
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self)
            }
        }

        impl FromStr for $name {
            type Err = Error;
            fn from_str(s: &str) -> Result<Self> {
                Self::parse(s)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(
                deserializer: D,
            ) -> std::result::Result<Self, D::Error> {
                let s = String::deserialize(deserializer)?;
                Self::parse(&s).map_err(de::Error::custom)
            }
        }
    };
}

define_id!(
    BlobId,
    BLOB_PREFIX,
    InvalidContentId,
    "Identifier for content-addressed blob data."
);
define_id!(
    RecordId,
    RECORD_PREFIX,
    InvalidRecordId,
    "Identifier for a signed audit record."
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashing_is_deterministic() {
        assert_eq!(BlobId::from_bytes(b"hello"), BlobId::from_bytes(b"hello"));
        assert_ne!(BlobId::from_bytes(b"hello"), BlobId::from_bytes(b"world"));
    }

    #[test]
    fn display_and_parse_round_trip() {
        let id = BlobId::from_bytes(b"payload");
        let s = id.to_string();
        assert!(s.starts_with(BLOB_PREFIX));
        assert_eq!(s.len(), BLOB_PREFIX.len() + DIGEST_HEX_LEN);
        assert_eq!(s.parse::<BlobId>().unwrap(), id);
    }

    #[test]
    fn prefixes_are_not_interchangeable() {
        let blob = BlobId::from_bytes(b"x");
        let record = RecordId::from_bytes(b"x");
        assert!(blob.to_string().parse::<RecordId>().is_err());
        assert!(record.to_string().parse::<BlobId>().is_err());
    }

    #[test]
    fn rejects_malformed_input() {
        assert!(BlobId::parse("ppb1:zz").is_err());
        assert!(BlobId::parse("ppb1:").is_err());
        assert!(BlobId::parse("").is_err());
        assert!(BlobId::parse(&format!("ppb1:{}", "0".repeat(63))).is_err());
    }

    #[test]
    fn accepts_surrounding_whitespace() {
        let id = BlobId::from_bytes(b"x");
        assert_eq!(BlobId::parse(&format!("  {id}  ")).unwrap(), id);
    }

    #[test]
    fn serializes_as_string() {
        let id = RecordId::from_bytes(b"abc");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""));
        assert_eq!(serde_json::from_str::<RecordId>(&json).unwrap(), id);
    }
}
