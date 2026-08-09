# Architectural Decision Records

Status: deferred platform proposals

> These records describe the earlier broad platform concept. Punctra v0.1 is
> scoped to the render engine in [the accepted design](../design/render-engine-v0.1.md).

Only hard-to-reverse or surprising decisions are recorded here. Module details and routine implementation choices belong in [the architecture package](../architecture/README.md).

| ADR | Status | Decision |
|---|---|---|
| [ADR-0001](0001-in-process-library-modules.md) | Proposed | Independent modules are in-process Rust libraries, not network processes. |
| [ADR-0002](0002-immutable-sources-and-revision-overlays.md) | Proposed | Source bytes are immutable; logical Edits are revisioned sparse overlays. |
| [ADR-0003](0003-cpu-authoritative-geometry.md) | Proposed | CPU precision is authoritative; GPU values are disposable display data. |
| [ADR-0004](0004-headless-workspace-and-separate-renderer.md) | Proposed | The Workspace is headless and rendering remains a separate module. |
| [ADR-0005](0005-commit-outcomes-include-indeterminate.md) | Proposed | Commit acknowledgement can be indeterminate and is reconciled by Operation Identity. |
