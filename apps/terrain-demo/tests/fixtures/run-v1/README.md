# Terrain Demo owner-local Run v1 fixture corpus

This corpus contains generated technical facts only. It has no field data,
labels, or preference evidence. The Complete Run was captured from the
terrain-demo process fixture after a full workflow over its generated 64-point
LAS source. Its canonical `terrain.xml` and `audit.json` bytes, lengths, and
workflow hashes are bound by the Complete journal. The report explicitly marks
partner acceptance, downstream round-trip, and human workflow acceptance as
unevaluated.

`prefixes` captures the durable journal after each of the eight ordered v1
checkpoints. `complete` holds the eight-frame `run.pwf`, both exact artifacts,
and the required empty `run.lock`. `manifest.json` records format versions,
fixed identities, and the exact byte length and BLAKE3 digest of every payload.

Normal tests consume the payloads with `include_bytes!` and never regenerate
the expected side. The owner-only capture helper accepts an already-published
Complete Run root, validates all eight journal frames and both artifact
witnesses, and refuses to overwrite an existing target:

```text
cargo run -p terrain-demo --example run_fixture_capture -- \
  SOURCE_COMPLETE_RUN_ROOT NEW_CORPUS_ROOT
```

Compatibility changes require a new fixture generation rather than replacement
of v1 evidence.

The v1 journal contract binds raw owner-platform path bytes. Therefore journal
tests prove byte compatibility, ordered recovery, and artifact hashes after a
clone or worktree move, while Complete-Run qualification must reject that
relocated path binding without mutation. The corpus does not claim portable
qualification success for a different checkout path.
