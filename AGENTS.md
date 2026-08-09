# Repository Instructions

## Verification

- Run formatting, linting, tests, documentation checks, and GPU acceptance tests only on the local machine.
- Do not add or enable GitHub Actions or another hosted CI service unless the user explicitly requests it.
- Before handing off code changes, run the relevant commands documented in `CONTRIBUTING.md`.
- When a local GPU adapter is expected, set `PUNCTRA_REQUIRE_GPU=1` so a missing adapter fails the GPU acceptance tests.
