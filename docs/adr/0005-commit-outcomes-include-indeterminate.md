# ADR-0005: Commit acknowledgement can be indeterminate

## Status

Proposed

## Context

A local durable commit crosses filesystem write, flush, metadata-update, and acknowledgement steps. An I/O error or process failure near the durable commit point can make it impossible for the caller to know immediately whether the old or new head will appear after recovery. Reporting an ordinary failure would invite an unsafe duplicate retry.

## Decision

Every commit receives a caller-chosen Operation Identity and returns one of Committed, Rejected, or Indeterminate. The caller must durably retain the identity with the Workspace Identity needed for reconciliation before starting the Job. The Revision store—not the caller—canonicalizes and hashes the request and must durably stage the complete Edit payload before crossing the logical commit point.

Committed identifies a durable new Revision. Rejected guarantees the previous head is unchanged. Indeterminate means the commit point may have been crossed; the caller reopens or calls operation reconciliation to learn whether that Operation Identity committed. The same recorded identity and canonical request are idempotent. Reusing the identity with different content is Rejected. NotRecorded reconciliation guarantees that no operation record or Revision exists; an interrupted ephemeral Point Set does not need to be reconstructed.

Recovery always exposes a complete old or new Revision, never partial logical state.

## Consequences

- The interface describes real filesystem failure modes instead of claiming impossible certainty.
- Retrying after Indeterminate is safe only through Operation Identity reconciliation.
- A crash before the return value is delivered remains recoverable because the caller retained the identity first.
- Process-scoped Point Sets are copied into journal-owned staging before a commit can become ambiguous.
- The Revision journal must retain an operation-status record and idempotence information.
- Application adapters need a small recovery state instead of treating every error as rejection.
- Tests must inject failures around every durability and acknowledgement step.
