# Real Sources v0.3

Status: complete; accepted scope implemented and locally verified

Punctra v0.3 adds a headless Source path for reading authoritative Point data
without constructing a Workspace, Spatial Index, View, or renderer. The release
establishes stable Source and Point Identity, lossless canonical Point values,
runtime-neutral bounded execution, and interchangeable in-memory and LAS/LAZ
adapters. `source-las` supports LAS point-data record formats 0–10 and LAZ
formats 0–8. LAZ formats 9 and 10 fail with `UnsupportedFormat` before Source
publication until an exact layered WavePacket14 codec is available; v0.3 does
not publish waveform values that its pinned codec cannot preserve exactly.

The Source seam is deliberately narrower than the earlier platform proposal.
Ordinary callers receive one opaque, already verified `Source`; adapter traits,
verification witnesses, decoder blocks, and batch validation remain behind that
interface.

## Primary interface

Adapter crates own convenient opening functions. They return the same verified
Source type:

```rust,ignore
let source = source_memory::open(input).blocking_wait()?;
// The same consumer can use:
// let source = source_las::open("survey.laz").blocking_wait()?;

println!("{} Points", source.metadata().point_count());

let mut batches = source.points()?;
while let Some(batch) = batches.next()? {
    consume_authoritative_points(&batch)?;
}

let summary = batches
    .summary()
    .expect("successful end of stream has an exact summary");
assert_eq!(summary.source(), source.identity());
```

Advanced callers use `Source::read` to select logical ordinal spans,
Attributes, and hard read limits. Input spans may be unordered or overlap; the
Source module validates, sorts, and merges them before an adapter sees the
request. Output remains in ascending logical ordinal order without duplicate
Points.

Opening is a runtime-neutral `Job`. A read is a concrete, backpressured pull
stream. Its cloneable `OperationHandle` may be moved to another host task to
observe monotonic progress or request cancellation; it cannot publish producer
progress. Progress is not mixed into the Point Batch stream, so the common
consumer loop handles only authoritative data and terminal errors.

## Canonical contracts

### Source and Point Identity

`SourceId` is an opaque 256-bit identity derived from:

- a complete content fingerprint;
- the adapter kind and canonicalization version; and
- the adapter's declared logical-order rule.

Paths, allocation choices, requested spans, Point Batch boundaries, worker
count, and completion timing do not affect Source Identity. Replacing or
re-encoding a Source creates a new identity even when its visible Points are
equivalent.

`PointId` is `(SourceId, logical ordinal)`. LAS and LAZ use point-record order.
The in-memory adapter uses the caller's immutable input row order. Repeated
reads and differently partitioned reads therefore preserve Point Identity.

### Point values

A canonical `PointBatch` contains:

- one Source Identity and one non-empty contiguous ordinal range;
- exact signed integer position ticks with finite source scale and offset; and
- zero or more typed Attribute columns keyed by the verified Source schema.

Every column has the same row count. Attribute types, integer widths, floating
bit widths, flags, and fixed-width opaque values remain representable without
coercion. A requested Attribute is returned exactly or the read fails with an
unsupported-schema error. Coordinate Reference may be explicitly unknown and
is never guessed or transformed.

Point Batch boundaries are not stable interface data. Different valid budgets
may choose different boundaries, but flattening successful reads yields the
same Point Identities, ticks, Attributes, and order.

### Metadata and provenance

`SourceMetadata` records the Point count, quantization, finite bounds when the
Source is non-empty, Coordinate Reference, Attribute schema, format name, and
bounded format metadata. Unknown LAS VLR and EVLR payloads remain preserved as
ordered namespaced metadata rather than being silently discarded.

Every verified Source carries immutable provenance containing its Source
Identity, full content fingerprint, logical-order rule, and contract version.
The exact successful read summary copies that provenance and the normalized
request facts.

## Verification

A Source is never provisional. No Point Identity or Point Batch is exposed
before opening has established or matched Source Identity.

Opening supports three caller intents:

- identify a new Source, which always performs Full verification;
- reopen a recorded Source with Full verification; or
- reopen with Fast verification, optionally falling back to Full when the Fast
  evidence is inconclusive.

`SourceRecord` is a versioned, serializable verification record. It binds the
Source Identity and full fingerprint to the adapter kind and version,
logical-order rule, schema facts, and bounded adapter-owned Fast evidence.
Reopening never silently assigns a replacement identity.

A Fast mismatch reports `VerificationRequired`; a subsequent Full check either
confirms the recorded Source or reports `SourceChanged`. Malformed bytes that
match the expected immutable content are `CorruptSource`. Changed bytes or
storage facts are `SourceChanged`. Errors retain bounded phase and ordinal or
byte context when known, and malformed input never panics.

Verified readers check their adapter's change witness before publishing an
affected batch. The in-memory adapter exposes deterministic mutation and fault
controls only through its opt-in `test-support` feature for conformance tests.
Local LAS tests use private file and decoder fault seams rather than expanding
the public Source interface.

## Bounded execution

`foundation-runtime` owns runtime-neutral Jobs, operation controls, progress,
cancellation, and the common batch-stream contract. It starts no process-global
executor and requires no async runtime.

Every read declares hard limits for:

- normalized Source spans;
- output Points;
- Point Batch Points;
- canonical Point Batch payload bytes; and
- adapter working bytes where decoding requires a separate block.

The producer does not require the consumer to retain prior Point Batches. A
single unavoidable record or decoder block that cannot fit its declared limit
fails explicitly instead of allocating past the limit. Checked arithmetic
precedes allocation.

Successful exhaustion records exactly one complete summary and then fuses.
Cancellation or failure records no complete summary and then fuses. Previously
returned Point Batches are partial input and cannot be mistaken for complete
Coverage.

## Module ownership

The implementation order keeps each module directly usable:

1. **point-contracts** defines and validates canonical Source, Point,
   Attribute, coordinate, metadata, and provenance values.
2. **foundation-runtime** provides runtime-neutral Jobs, progress,
   cancellation, and bounded pull-stream semantics.
3. **point-source** owns verified Source construction, Source Records, identity
   derivation, span normalization, read validation, and the caller-facing
   Source interface.
4. **source-memory** supplies the first concrete adapter, deterministic
   fixtures, and corruption/change fault injection.
5. **source-las** adds bounded LAS 0–10 reads and LAZ 0–8 decoding through the
   same interface and conformance suite. It explicitly rejects LAZ 9/10 at the
   format boundary because their layered WavePacket14 path does not yet meet
   the exact-value contract.

The adapter-author seam is public only so workspace adapter crates can satisfy
it. It is version-coupled to Punctra's official adapters and is not a stable
plugin promise in v0.3. Only `point-source` publishes a caller-visible Source
or accepts a Point Batch as valid.

`render-protocol` reuses canonical Point Identity for display picking and
highlighting, but Source modules never depend on View or GPU modules. Display
positions remain disposable origin-relative `f32` values; Source positions
remain CPU-authoritative ticks plus `f64` scale and offset.

## Delivery slices

Implementation proceeds as vertical evidence rather than crate scaffolding:

1. canonical contracts, Jobs, the verified Source seam, in-memory reads,
   conformance faults, and canonical Point Identity in the renderer;
2. bounded LAS opening and reads with metadata and Attribute preservation;
3. bounded LAZ 0–8 decoding, explicit LAZ 9/10 rejection, source-scale
   benchmarks, memory ceilings, and a directly usable file-inspection example.

All three delivery slices are implemented. The conformance, benchmark,
documentation, workspace, and required GPU gates below are the release record.

## Acceptance

Punctra v0.3 is complete only when:

- memory, LAS, and LAZ adapters pass one caller-facing Source conformance
  suite;
- repeated reads and differently partitioned equivalent reads produce the same
  Point Identities, ticks, Attributes, and order;
- overlapping spans produce no duplicate Point;
- every requested supported Attribute and bounded metadata record round-trips
  without silent loss;
- every unsupported compressed point format is rejected before a Source or
  Point Batch is published;
- corrupt, truncated, unsupported, cancelled, and changed inputs fail
  explicitly and leave their streams fused without a completion summary;
- Point Batch point and byte limits and decoder working limits are enforced
  before publication;
- source-scale LAS and LAZ benchmarks report throughput and enforce a declared
  peak-memory ceiling;
- each new module has direct interface tests and a real caller; and
- all local verification in `CONTRIBUTING.md`, including required GPU
  acceptance, passes.

## Out of scope

Punctra v0.3 does not add:

- Spatial Index construction, persistence, or spatial lookup;
- exact spatial or Attribute predicates beyond ordinal spans and field
  projection;
- Workspace, Snapshot, Query, Edit, or Revision behavior;
- remote range reads, retry policy, caches, or networking;
- reprojection, vertical-datum transformation, or Coordinate Reference
  guessing;
- COPC hierarchy import;
- automatic renderer materialization or View policy; or
- a general third-party format/plugin registry.
