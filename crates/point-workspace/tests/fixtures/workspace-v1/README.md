# Point Workspace owner-local v1 fixture corpus

This corpus contains generated technical data only: 16 synthetic quantized
positions and one synthetic `U8` classification column. It contains no user
paths, field data, labels, or preference evidence.

`manifest.json` records the v1 disk and semantic versions, fixed Workspace,
Source, Revision, and Operation identities, and the exact byte length and
BLAKE3 digest of every payload. Each state also requires empty `operations`,
`revisions`, and `scratch` directories where no payload is listed; the tests
materialize those structural directories without changing the checked bytes.

The four states are:

- `root`: a newly committed Workspace root;
- `committed`: one classification Revision and its durable ready intent;
- `retryable-ready`: the same ready intent with its Revision link absent;
- `recorded-rejection`: a durable no-change rejection.

Normal tests consume every payload with `include_bytes!` and never regenerate
the expected side. The ignored `capture_workspace_v1_fixture_corpus` test is an
owner-only capture tool that refuses to overwrite this directory. Compatibility
changes require an intentional new fixture generation rather than replacement
of v1 evidence.
