# SourceRecord v1 fixture

`source-record-v1.json` pins one schema-1 `SourceRecord` produced from the
deterministic, non-secret memory-adapter fixture in `tests/interface.rs`. The
owning interface test reads these committed bytes, compares them with current
serialization, reopens the semantic record, and verifies future-version and
truncation failures. It never rewrites the fixture.

The record uses adapter `fake` version `1`, logical order `input row order`,
eight Points, one `U16` intensity Attribute, and the opaque Fast token
`stable-fast-token`.

Exact file facts: 739 bytes and BLAKE3
`d30e27c1a6806d62a454aeabb6dba372d7e7081eef7ef70b01cfdf3e9af75b1d`.
