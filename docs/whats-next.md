# What's next

ProofPrint keeps signed, linked records of datasets, training runs,
checkpoints, and evaluations in an append-only log. The goal is real use:
people who train and evaluate models record what they did in a form that
someone else can check and contradict.

## Current state

| Piece | Status |
| --- | --- |
| `proofprint-core` | Working. Records, Ed25519, file store, Merkle log, example schemas. One writer per data directory. Interrupted writes are dropped and reported on open. |
| `proofprint-cli` | Working. `init`, `keygen`, `whoami`, `publish`, `get`, `verify`, `log`, `root`, `prove`, `tree`, `blob`, `schemas`. |
| `proofprint-node` | Working and tested. Records, trees, proofs, checks, uploads, downloads, and signing drafts with the node's key. Loopback only unless `--allow-remote`; no authentication. |
| Explorer | Working. React. History with kind and signer filters, lineage graph, record page with checks, inclusion proof, and exact signed bytes. |
| Python client | Working. `NodeClient`, `Publisher`, `TrainingRecorder`, a `transformers.Trainer` callback, and `copy_to` for handing history to another node. |
| Test vectors | Frozen in `spec/v1/vectors.json`. Another language must reproduce them before it signs anything. |
| Witnessed roots, public hosting, replication | Not started. |

What a signature shows: this key committed these bytes. It does not show that
a model was trained, that a dataset was used, or that a score is right.

## Next, in order

1. **Use it on a real fine-tune.** Run the `Trainer` callback through an actual
   training run and a real evaluation, then fix what gets in the way. Measure
   the time each step record adds, upload time for multi-gigabyte checkpoints
   (uploads are capped at 4 GiB by default), and how big the log gets.

2. **Re-run evaluations.** A command that takes an evaluation record, fetches
   the named checkpoint, re-runs the same `lm-eval-harness` task, and publishes
   an `attestation.verification/v1` with the output attached. A check by a
   second party is the first thing here that goes beyond a signed claim.

3. **Witnessed roots.** The node signs `(tree_size, root)` checkpoints and
   sends them somewhere it does not control: an existing transparency log such
   as Rekor, or independent witnesses. Add consistency proofs between
   checkpoints. Without this, whoever runs a log can quietly rewrite it.

4. **Identities people already use.** Accept Sigstore keyless signing (OIDC,
   such as a GitHub or Google account) as a signer type next to raw Ed25519
   keys. This changes the `signer` field, so it needs a spec update and new
   test vectors.

5. **Read and write existing formats.** Export records as in-toto statements so
   SLSA and model-signing tools can read them. Accept an OpenSSF model-signing
   signature as evidence on a model record.

6. **Hugging Face Hub.** Put record ids in model cards, and add a checker that
   downloads a Hub model, hashes its files, and finds the matching record.

7. **Durable internals.** A SQLite index instead of holding the whole log in
   memory, incremental Merkle roots, and streamed listings. A run with a record
   per step reaches 100,000 records quickly.

8. **Authentication.** Tokens on the write routes before any node listens on
   more than one machine.

Not planned: tokens, mining, or consensus. Several organizations jointly
running one log is a separate design, needed only if someone asks for it.

## Order of effort

```text
1. Real fine-tune through the Trainer callback
2. Evaluation re-run command
3. Signed checkpoints sent to an outside log
4. Sigstore signer type (spec change + vectors)
5. in-toto export
6. Hub integration
7. SQLite index
8. Write-route authentication
```

## Try what exists today

From the repository root:

```bash
cargo test --workspace
cd explorer && npm ci && npm run build && cd ..
cargo run -p proofprint-node -- --demo --data target/demo-data
```

Open <http://127.0.0.1:4780>. For your own records, run the node without
`--demo` and follow [python/README.md](../python/README.md).
