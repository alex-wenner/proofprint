# Development conventions

- Keep generic record handling in `proofprint-core`. Argument parsing and
  output belong in `proofprint-cli`; HTTP belongs in `proofprint-node`.
- Prefer small modules with explicit inputs and typed errors. Add abstractions
  when they remove real duplication or mark a useful boundary.
- Stream files. Do not load checkpoint-sized files into memory to import,
  export, upload, or check them.
- Say what evidence shows and no more. Keep signed claims, checked
  computations, locally stored files, and independently witnessed history apart.
- Changes to signed bytes, identifiers, or Merkle rules need a spec update and
  regenerated vectors:
  `cargo test -p proofprint-core --test vectors -- --ignored write_vectors`.
- Test at the edges: malformed input, altered signatures, corrupted files,
  interrupted writes, and separate CLI invocations.
- Explorer: React class components. The explorer never signs. Copy signed data
  from the node's canonical text rather than re-serializing parsed JSON.
- Python client: standard library only. `transformers` is imported only in
  `proofprint/trainer.py`.

Before submitting a change:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cd explorer && npm test && npm run build
cd python && python -m unittest discover -s tests -t .
```

For the Python end-to-end test, build the node and set `PROOFPRINT_NODE_BIN`
to `target/debug/proofprint-node`. CI runs all of the above, the Rust checks on
Windows and Linux. Keep data directories and signing keys out of version control.
