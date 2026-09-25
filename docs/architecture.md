# Architecture

ProofPrint moves signed records around and checks them. What a record means
belongs to its schema and to whoever reads it.

```text
Python client / CLI / explorer
          |
          v
proofprint-node (HTTP, holds the signing key)
          |
          v
Ledger ---- Log: signed records in append order, Merkle root, inclusion proofs
   |
   +------- BlobStore: files by BLAKE3 hash
```

## Pieces

`proofprint-core` is the Rust library. A `Record` carries any JSON object as
its payload, names its parents by id, and references files by `BlobRef`. A
`SignedRecord` adds an Ed25519 signature over the record's canonical bytes.
`Ledger` combines the `Log` and the `BlobStore` for one data directory. An OS
file lock allows one open `Log` per directory. On open, every stored record is
verified again. An unfinished last line from an interrupted write is dropped;
any other damage refuses the open.

`Schema` gives typed payloads to known kinds: datasets, models, training runs,
training steps, evaluations, and verifications by another party. The log
accepts any kind that follows `name/vN`. It does not enforce what a schema says.

`proofprint-cli` works directly on a data directory.

`proofprint-node` wraps one `Ledger` behind a mutex and serves it over HTTP.
It accepts records signed elsewhere and signs drafts with its own key. It
streams uploads to a staging file on the store's volume and hashes them into
place. It serves the built explorer from `explorer/dist` for any path outside
`/api/`. The router is tested in-process, without a socket.

`explorer/` is a React app made of class components. It reads the API only
and never signs. Record ids, the exact signed bytes, and the envelope are
copied from the node's canonical text, because `JSON.parse` rounds integers
above 2^53.

`python/` is a client for the HTTP API with no dependencies. The node holds the
key; Python never signs. `copy_to` sends a record and its ancestors to another
node, oldest first, with their original signatures.

## What each check establishes

| Check | Shows | Does not show |
| --- | --- | --- |
| Signature | The stated key signed these canonical bytes | Who controls the key; whether the payload is true |
| File hash | Stored bytes match the id in the record | That anyone else still has the file |
| Parents | The record names these earlier records | That the earlier records were used as described |
| Inclusion proof | This log, at this size and root, contains the record | That the log was never rewritten |
| Verification record | A second key published a check of the parent record | That the second party checked carefully |

## Toward a public service

1. **Witnessed history:** signed `(tree_size, root)` checkpoints, consistency
   proofs, and outside parties that keep and compare checkpoints.
2. **Durable storage:** a persistent index, incremental roots, streamed listings.
3. **Access control:** authenticated write routes before non-local use.
4. **Mirrors:** copies of selected files, with availability reported per copy
   and bytes checked on download.
5. **Several operators:** agreement rules only if several organizations need to
   run one log together. A cryptocurrency or mining is not part of this.

## Resources

Record handling runs on ordinary CPUs. Files dominate storage and bandwidth;
import, export, upload, and checks stream through fixed buffers. The record
index is held in memory.

Checking a signature is cheap. Re-running an evaluation or a training segment
may need the same GPUs the original run used. The log records such checks and
their outputs; it does not run them.
