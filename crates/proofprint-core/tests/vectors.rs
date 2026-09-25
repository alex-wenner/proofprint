//! Frozen v1 encodings, checked against `spec/v1/vectors.json`.
//!
//! A failure here means signed bytes, ids, or Merkle rules changed. That is a
//! wire-format change: update `spec/v1/record.md`, then regenerate on purpose:
//!
//! ```text
//! cargo test -p proofprint-core --test vectors -- --ignored write_vectors
//! ```
//!
//! Other implementations sign against this file, not against this crate.

use std::fs;

use proofprint_core::log::{build_inclusion_path, leaf_hash, tree_root, verify_inclusion};
use proofprint_core::record::utc;
use proofprint_core::weights::{digest_bytes, TensorDigest, WeightsManifest};
use proofprint_core::{BlobId, BlobRef, KeyPair, Record, RecordId, SignedRecord};
use serde::{Deserialize, Serialize};
use serde_json::json;

const PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../spec/v1/vectors.json");
const SEED: [u8; 32] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
];
/// 2026-01-15T12:00:00Z
const CREATED: i64 = 1_768_478_400;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Vectors {
    version: u8,
    /// Ed25519 seed, hex. A test key; never use it for anything else.
    seed: String,
    signer: String,
    blobs: Vec<BlobVector>,
    records: Vec<RecordVector>,
    /// Root of the tree over the first `n` records, for `n = 1..=records.len()`.
    roots: Vec<String>,
    proofs: Vec<ProofVector>,
    /// Weight manifests (spec/v1/training.md): canonical bytes and sha256 digest.
    #[serde(default)]
    manifests: Vec<ManifestVector>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct ManifestVector {
    manifest: serde_json::Value,
    canonical: String,
    digest: String,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct BlobVector {
    text: String,
    id: BlobId,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct RecordVector {
    name: String,
    record: Record,
    canonical: String,
    id: RecordId,
    sig: String,
    leaf: String,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct ProofVector {
    leaf_index: u64,
    tree_size: u64,
    path: Vec<String>,
    root: String,
}

/// Builds the vectors from fixed inputs. Deterministic: Ed25519 signatures
/// have no randomness, and every timestamp is pinned.
struct Fixture {
    keys: KeyPair,
}

impl Fixture {
    fn new() -> Self {
        Self {
            keys: KeyPair::from_seed(SEED),
        }
    }

    fn record(&self, kind: &str, payload: serde_json::Value) -> Record {
        Record::new(kind, payload, self.keys.signer()).with_created(utc(CREATED))
    }

    fn records(&self) -> Vec<(&'static str, Record)> {
        let empty = self.record("example.note/v1", json!({}));
        let unicode = self.record(
            "example.note/v1",
            json!({
                "title": "Ünïcödé — 日本語 ✓",
                "message": "quotes \" backslash \\ newline \n tab \t nul \u{0}",
                "emoji": "🙂",
                "é": "sorted by UTF-8 bytes, after z",
                "z": "z",
                "a": "a"
            }),
        );
        let numbers = self.record(
            "example.note/v1",
            json!({
                "int": 42,
                "negative": -7,
                "large": 9_007_199_254_740_993_i64,
                "float": 0.0003,
                "half": 2.5,
                "one": 1.0,
                "zero": 0,
                "exp": 1e21,
                "list": [1, 2.5, -3]
            }),
        );
        let nested = self.record(
            "example.note/v1",
            json!({
                "a": {"b": {"c": [{"d": null}, {"e": true}, {"f": false}]}},
                "empty_list": [],
                "empty_object": {}
            }),
        );
        let linked = self
            .record("ml.training-step/v1", json!({"step": 1}))
            .with_parents([empty.id().unwrap(), unicode.id().unwrap()])
            .with_blob(BlobRef::from_bytes(
                "weights",
                b"weights",
                Some("application/octet-stream".into()),
            ))
            .with_blob(BlobRef::from_bytes("notes", b"notes", None));
        let kind = self.record("a.b-c.d0/v12", json!({"k": "v"}));
        vec![
            ("empty", empty),
            ("unicode", unicode),
            ("numbers", numbers),
            ("nested", nested),
            ("linked", linked),
            ("kind-grammar", kind),
        ]
    }

    fn vectors(&self) -> Vectors {
        let records: Vec<RecordVector> = self
            .records()
            .into_iter()
            .map(|(name, record)| {
                let signed = record.clone().sign(&self.keys).unwrap();
                let id = record.id().unwrap();
                RecordVector {
                    name: name.into(),
                    canonical: String::from_utf8(record.canonical_bytes().unwrap()).unwrap(),
                    id,
                    sig: signed.sig,
                    leaf: hex::encode(leaf_hash(&id)),
                    record,
                }
            })
            .collect();
        let leaves: Vec<[u8; 32]> = records.iter().map(|r| leaf_hash(&r.id)).collect();
        let roots = (1..=leaves.len())
            .map(|n| hex::encode(tree_root(&leaves[..n]).unwrap()))
            .collect();
        let proofs = [(0, 1), (2, 3), (4, 5)]
            .into_iter()
            .chain((0..leaves.len()).map(|i| (i, leaves.len())))
            .map(|(index, size)| ProofVector {
                leaf_index: index as u64,
                tree_size: size as u64,
                path: build_inclusion_path(&leaves[..size], index)
                    .unwrap()
                    .iter()
                    .map(hex::encode)
                    .collect(),
                root: hex::encode(tree_root(&leaves[..size]).unwrap()),
            })
            .collect();
        Vectors {
            version: 1,
            seed: hex::encode(SEED),
            signer: self.keys.signer(),
            blobs: ["", "hello", "weights"]
                .into_iter()
                .map(|text| BlobVector {
                    text: text.into(),
                    id: BlobId::from_bytes(text.as_bytes()),
                })
                .collect(),
            records,
            roots,
            proofs,
            manifests: vec![manifest_vector()],
        }
    }
}

fn manifest_vector() -> ManifestVector {
    let manifest = WeightsManifest::new(vec![
        TensorDigest::of_bytes(
            "layer.weight",
            "F32",
            vec![2],
            &[0, 0, 128, 63, 0, 0, 0, 64],
        ),
        TensorDigest::of_bytes("layer.bias", "F32", vec![], &[0, 0, 0, 0]),
    ])
    .unwrap();
    ManifestVector {
        manifest: serde_json::to_value(&manifest).unwrap(),
        canonical: String::from_utf8(manifest.canonical_bytes().unwrap()).unwrap(),
        digest: manifest.digest().unwrap(),
    }
}

fn frozen() -> Vectors {
    let text = fs::read_to_string(PATH).expect("spec/v1/vectors.json is present");
    serde_json::from_str(&text).expect("spec/v1/vectors.json parses")
}

#[test]
fn this_build_reproduces_the_frozen_vectors() {
    let expected = frozen();
    let actual = Fixture::new().vectors();
    assert_eq!(actual.signer, expected.signer, "signer");
    assert_eq!(actual.blobs, expected.blobs, "blob ids");
    assert_eq!(actual.records.len(), expected.records.len(), "record count");
    for (a, e) in actual.records.iter().zip(&expected.records) {
        assert_eq!(a.canonical, e.canonical, "canonical encoding of {}", e.name);
        assert_eq!(a.id, e.id, "id of {}", e.name);
        assert_eq!(a.sig, e.sig, "signature of {}", e.name);
        assert_eq!(a.leaf, e.leaf, "leaf hash of {}", e.name);
    }
    assert_eq!(actual.roots, expected.roots, "roots");
    assert_eq!(actual.proofs, expected.proofs, "inclusion proofs");
    assert_eq!(actual.manifests, expected.manifests, "weight manifests");
}

/// The file must stand on its own: every claim in it checks out when
/// recomputed from the record JSON it carries.
#[test]
fn the_frozen_vectors_are_internally_consistent() {
    let vectors = frozen();
    let keys = KeyPair::from_seed(hex::decode(&vectors.seed).unwrap().try_into().unwrap());
    assert_eq!(keys.signer(), vectors.signer);
    for blob in &vectors.blobs {
        assert_eq!(BlobId::from_bytes(blob.text.as_bytes()), blob.id);
    }

    let mut leaves = Vec::new();
    for vector in &vectors.records {
        let canonical = vector.record.canonical_bytes().unwrap();
        assert_eq!(canonical, vector.canonical.as_bytes(), "{}", vector.name);
        assert_eq!(
            RecordId::from_bytes(&canonical),
            vector.id,
            "{}",
            vector.name
        );
        let signed = SignedRecord {
            record: vector.record.clone(),
            sig: vector.sig.clone(),
        };
        assert_eq!(signed.verify().unwrap(), vector.id, "{}", vector.name);
        assert_eq!(
            hex::encode(leaf_hash(&vector.id)),
            vector.leaf,
            "{}",
            vector.name
        );
        leaves.push(leaf_hash(&vector.id));
    }

    assert!(
        !vectors.manifests.is_empty(),
        "at least one manifest vector"
    );
    for m in &vectors.manifests {
        let canonical = proofprint_core::canonical::to_vec(&m.manifest).unwrap();
        assert_eq!(
            canonical,
            m.canonical.as_bytes(),
            "manifest canonical bytes"
        );
        assert_eq!(digest_bytes(&canonical), m.digest, "manifest digest");
    }

    for (n, root) in vectors.roots.iter().enumerate() {
        assert_eq!(
            hex::encode(tree_root(&leaves[..=n]).unwrap()),
            *root,
            "root at size {}",
            n + 1
        );
    }
    for proof in &vectors.proofs {
        let path: Vec<[u8; 32]> = proof
            .path
            .iter()
            .map(|h| hex::decode(h).unwrap().try_into().unwrap())
            .collect();
        let root: [u8; 32] = hex::decode(&proof.root).unwrap().try_into().unwrap();
        let leaf = leaves[proof.leaf_index as usize];
        assert!(
            verify_inclusion(proof.leaf_index, proof.tree_size, &leaf, &path, &root),
            "proof for leaf {} in tree of {}",
            proof.leaf_index,
            proof.tree_size
        );
        assert_eq!(
            hex::encode(root),
            vectors.roots[proof.tree_size as usize - 1]
        );
    }
}

#[test]
#[ignore = "rewrites spec/v1/vectors.json; run on purpose after a spec change"]
fn write_vectors() {
    let vectors = Fixture::new().vectors();
    let mut text = serde_json::to_string_pretty(&vectors).unwrap();
    text.push('\n');
    fs::write(PATH, text).unwrap();
}
