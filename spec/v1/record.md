# ProofPrint record format — experimental v1

## Envelope

A signed record is a JSON object containing `record` and `sig`:

```json
{
  "record": {
    "v": 1,
    "kind": "example.note/v1",
    "payload": { "message": "hello" },
    "created": "2026-09-18T00:00:00Z",
    "signer": "ed25519:<64 lowercase hex characters>"
  },
  "sig": "<128 lowercase hex characters>"
}
```

Angle-bracket values above are placeholders, not a valid signed test vector.

`parents` is an ordered array of record IDs. `blobs` is an array of references
with `name`, `blob`, `size` (unsigned byte count), and optional `media` fields.
Empty parent and blob arrays are omitted by the reference encoder.

The core treats `payload` as an opaque JSON object. Binary and large inputs live
in attachments; their IDs and declared sizes are part of the signed record.

## Identifiers and signatures

- Blob ID: `ppb1:` + lowercase hex BLAKE3-256 of the exact file bytes.
- Record ID: `ppr1:` + lowercase hex BLAKE3-256 of the canonical unsigned record.
- Signer: `ed25519:` + lowercase hex of the 32-byte public key.
- Signature: Ed25519 over the canonical unsigned record bytes, encoded as hex.

The signature is not part of the record ID. It is checked before acceptance even
when the ID already exists. Re-publishing an existing valid record is idempotent.
The signer field is inside the signed content.

## Canonical encoding

The reference implementation serializes the typed `Record` into a JSON value:

1. Recursively sort object keys by UTF-8 byte order.
2. Preserve array order.
3. Emit no insignificant whitespace.
4. Encode JSON strings and numbers using the locked `serde_json` implementation.
5. Emit `created` in UTC RFC 3339 with seconds precision and a `Z` suffix.
6. Omit empty `parents`/`blobs` and absent optional blob media types.

This is a project-specific experimental encoding, **not RFC 8785 JCS**. The
implementation parses into typed records before verification; alternate input
spellings may normalize to the same signed representation. Raw input JSON
bytes are not signed.

## Test vectors

`vectors.json` in this directory freezes the encoding. It holds a test seed
and its signer, blob ids for fixed byte strings, and signed records chosen to
exercise the rules above: an empty payload, non-ASCII keys and strings with
escapes, integers above 2^53 and floats, nested and empty containers, parents
and attachments, and the kind grammar. For each record it gives the canonical
text, the id, the signature, and the leaf hash. It also gives the root after
each record and several inclusion proofs.

Another implementation must reproduce every value before it signs records.
Ed25519 signatures are deterministic, so the signatures must match byte for
byte. The Rust tests fail if this build stops reproducing the file. Changing
the file is a wire-format change.

## Validation

- Protocol version must be 1.
- Kind is dot-separated lowercase ASCII alphanumeric/hyphen segments, followed
  by `/v` and one or more decimal digits. Segments cannot start or end with `-`.
- Kind length: at most 128 bytes.
- Payload: a JSON object.
- At most 64 parents and 64 blob references, with no duplicate parents or names.
- Attachment names: 1–256 UTF-8 bytes.
- Canonical record size: at most 1 MiB; nesting depth is bounded.
- Public key and signature must parse and verify.
- The current implementation rejects timestamps over 300 seconds ahead of its
  local clock. This is local admission policy, not a distributed consensus rule.

By default, parents must already exist in the local log. The import override
allows references to unavailable parents; it does not prove their existence.

## Merkle log

Leaves are hashes of record digests, in append order:

```text
leaf = BLAKE3(0x00 || record_digest)
node = BLAKE3(0x01 || left_hash || right_hash)
```

Pair adjacent nodes and promote an unpaired last node unchanged. Repeat until
one root remains. An empty tree has no root. A single-leaf root is that leaf.
The tree uses the shape and domain-separation pattern of RFC 6962, with BLAKE3
and ProofPrint-specific leaf data; it is not an RFC 6962 wire implementation.

An inclusion proof contains the leaf index, tree size, leaf hash, ordered sibling
path, and root. Verification skips levels at which the target was promoted and
rejects missing or surplus siblings. A consumer must also check that the leaf
matches the intended record ID and that the root/size come from a trusted or
independently witnessed checkpoint. A proof supplied with its own root alone
does not establish a globally accepted history.

## Persistence

Signed records are appended to `log/records.ndjson`, one per line. Each line
and its newline are written from one buffer and synced before the append
returns. An exclusive lock on `log/writer.lock` allows one open log per
directory.

Every record is verified again when the log is opened. If the file ends in a
line without a newline, that line is the remains of an interrupted write:
when it parses and verifies it is kept and terminated; otherwise it is
truncated and the number of dropped bytes is reported. Any other line that
fails to parse or verify stops the log from opening.

Artifacts are placed under `blobs/<first-two-hex-digits>/<full-digest>.bin`
through unique staging files on the same volume.

Files are content-addressed, but their availability is a separate concern.
Independent checkpoint witnessing, consistency proofs, a public API, replication,
and consensus remain future protocol work.
