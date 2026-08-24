# Browser streaming deployment v1 fixture

These immutable files are generated and verified by:

```bash
cargo run -p browser-demo --bin generate_stream_fixture
```

`representative.las` is a deterministic 70,000-Point LAS 1.2 format-3 Source.
`source-record.json` records its complete `source-las` verification.
`representative.pidx` is the compatible disk-v2 inspection index, and
`deployment.json` binds the representation and sampled root ranges for the
private v0.16 browser host.

Use `--write` only when deliberately replacing the frozen fixture. The
generator compares the four semantic fixture files; this README is maintained
separately.
