# Verification Strategy

Status: deferred platform proposal; the implemented scopes are the
[v0.1 render engine](../design/render-engine-v0.1.md) and
[v0.2 adaptive View planner](../design/adaptive-view-planning-v0.2.md)

The interface is the test surface. Verification asks whether each module preserves its documented inputs, outputs, invariants, ordering, resource limits, errors, and effects. Tests do not lock private algorithms or file layouts unless the layout is itself a persisted contract.

## Test layers

| Layer | Purpose | Runs |
|---|---|---|
| Contract | Prove every adapter obeys the same seam | Every pull request |
| Oracle | Compare optimized behavior with a deliberately simple reference | Every pull request for small fixtures |
| Property | Explore combinations and invariants beyond hand-written examples | Every pull request with bounded cases |
| Recovery | Prove failures leave only allowed state | Every pull request for persistence modules |
| Fuzz | Reject malformed input safely | Short pull-request corpus; longer nightly runs |
| Golden semantic | Prove deterministic meaning without freezing irrelevant bytes | Every pull request |
| Benchmark | Track throughput, peak memory, latency distribution, and scaling | Scheduled and before releases |
| Hardware | Exercise graphics backends and storage behavior | Nightly or dedicated machines |
| End to end | Prove module composition through the Workspace interface | Every pull request for small fixtures |

## Shared fixture principles

Fixtures are data, not hidden assertion helpers. **point-fixtures** may generate deterministic Point Batches and malformed byte patterns, but it may not duplicate the algorithms being tested.

Required fixture classes:

- empty and one-Point Sources;
- dense grid, random uniform, clustered, corridor, and vertically stacked Points;
- very large coordinates with millimeter-scale separation;
- known and unknown Coordinate References;
- every supported LAS point format and important flag combination;
- extra-byte Attributes, missing optional Attributes, and unusual scale/offset values;
- LAS, chunked LAZ, and COPC equivalents;
- truncated headers, records, chunks, hierarchy pages, and invalid lengths;
- repeated XY positions with equal and conflicting elevations;
- collinear, crossing, overlapping, and self-intersecting Breaklines;
- Terrain Surfaces with boundaries, holes, long skinny triangles, and invalid faces;
- revision journals cut at every frame offset; and
- public-domain real Sources with recorded license and checksum.

Synthetic fixtures are generated from a seed recorded in every failure. No test depends on machine-local survey data.

## Public point-cloud datasets

Use real LAS/LAZ data progressively rather than making large downloads part of
the default test suite:

1. Generate synthetic clouds for unit and property tests.
2. Download a 1–5 million-point subset from OpenTopography or NOAA for routine
   integration and renderer development.
3. Test one USGS 3DEP LAZ tile with its original coordinates, attributes, and
   spatial reference intact.
4. Combine several tiles for streaming tests above 100 million points.
5. Use AHN or NOAA bulk data for dedicated stress testing.

Recommended sources:

- **[USGS 3DEP](https://www.usgs.gov/the-national-map-data-delivery/gis-data-download)**
  is the preferred starting point for classified US point clouds. Select one
  tile in LidarExplorer. USGS describes 3DEP products as public domain; see the
  [dataset record](https://data.usgs.gov/datacatalog/data/USGS%3Ab7e353d2-325f-4fc6-8d95-01254705638a).
- **[NOAA Digital Coast](https://coast.noaa.gov/htdata/lidar1_z/index.html)**
  provides LAZ bulk downloads, custom subsets, COPC, EPT, and browser previews.
  Use the [Data Access Viewer](https://coast.noaa.gov/dataviewer/) to select a
  small custom area.
- **[OpenTopography](https://opentopography.org/start)** supports cropped LAS
  or LAZ downloads, individual tiles, and bulk access across varied
  landscapes. Licensing and attribution vary by dataset, so record the
  selected dataset's metadata. See its
  [download documentation](https://opentopography.org/faq-page).
- **[AHN Netherlands](https://www.ahn.nl/open-data)** provides dense nationwide
  aerial lidar that is particularly useful for buildings and urban scenes.
  Older releases are unrestricted open data; AHN5 requires source attribution
  for derivative products.

Keep large point-cloud files out of Git. Commit a download script or retrieval
instructions instead, together with the source URL, dataset/version, license
and required attribution, expected checksum, and relevant metadata such as
bounds, point count, CRS, and LAS point format.

LAS/LAZ decoding remains outside the renderer interface. Tests and applications
feed the renderer bounded Point Batches, regardless of whether those batches
came from a local LAZ file, a network source, or a deterministic generator.

## Module verification matrix

### point-contracts

Test:

- construction rejects invalid row counts, non-finite required values, and invalid units;
- slicing and concatenating Point Batches preserve Point Identity and Attributes;
- quantized ticks decode deterministically to 64-bit world values;
- persisted value round trips preserve semantics;
- unknown persisted fields follow the documented compatibility rule; and
- schema examples compile as documentation tests.

Property examples:

- slicing then concatenating at any valid row returns an equivalent Point Batch;
- encode then decode returns the same contract value;
- Point Identity equality is independent of batch partitioning.

### foundation-runtime

Test:

- Job implements Future and blocking_wait with the same terminal result;
- cancellation before and after a declared commit point follows the documented outcome;
- progress phases and counters never move backward;
- every successfully exhausted stream makes exactly one summary available and then remains fused at `None`;
- failed and cancelled streams return one terminal error and then fused None;
- empty data batches are rejected;
- hard batch, memory, and temporary-storage budgets are enforced; and
- no test requires a particular async runtime.

### Source adapters

Run one shared Source conformance suite against **source-memory**, **source-las**, and **source-copc**:

- repeated reads return identical Point Batches;
- arbitrary valid span partitioning returns the same ordered Points;
- every Point Identity follows the adapter's logical ordinal rule;
- requested Attributes are exact or rejected, never silently dropped;
- batch size and transient memory remain bounded;
- cancellation stops without persistent effects;
- Fast and Full verification follow their documented threat models;
- Full verification returns a serializable SourceRecord whose Recorded form reopens the same Source;
- successful opening returns one opaque verified Source and never exposes it before verification completes;
- a changed Source is rejected before affected Point values are returned; and
- malformed input yields a structured error without panic or unbounded allocation.

Cross-adapter semantic fixtures should compare LAS, LAZ, and COPC encodings of the same logical Points while accepting that re-encoding creates a different Source Identity.

### point-index

The reference oracle is a brute-force scan over small canonical Point Batches.

For randomly generated Regions:

- every exact Point is included in the candidate plan;
- false positives are allowed and measured;
- no false negatives are accepted;
- candidate Source spans arrive in hard-bounded batches;
- incomplete indexes return IndexIncomplete rather than a partial exact plan;
- open_index reports Ready, Missing, or Incompatible without exposing partial persisted state;
- hierarchy nodes expose stable bounds, counts, children, spans, and error facts within the hard request limit;
- IndexDescriptor reports Artifact Identity, Source Identity, point count, build options, and schema version used to validate composition;
- rebuilds produce equivalent exact plans and hierarchy facts;
- Source Identity mismatch rejects the index;
- build interruption at every checkpoint resumes to the same final index; and
- corrupt pages are detected before use.

Scaling tests record:

- build throughput;
- write amplification;
- peak resident memory;
- index bytes per Point;
- candidate amplification by Region shape; and
- resume work after an interrupted build.

### point-set

Test:

- canonical identity ordering and duplicate removal;
- exact count and content hash across different Point Batch partitioning;
- forced spills under tiny memory budgets;
- hard temporary-storage exhaustion;
- bounded repeatable identity iteration;
- provenance derived from the terminal summary rather than a caller label;
- Point Batch Source mismatch with terminal Snapshot provenance fails without returning a handle;
- cleanup after the last ephemeral handle is released;
- expired, missing, and corrupt spill detection; and
- Source or Revision provenance mismatch rejection at commit.

### point-revisions

Use model-based tests with a simple in-memory map as the oracle:

- sequential Edit Batches produce the same Snapshot overlays;
- create and open_and_recover work directly without a Workspace;
- reopen with a different Revision Source Contract fails before exposing state;
- independently created Revision stores over the same Source produce disjoint Revision Identities;
- later operations in one batch win only for their named fields;
- stale expected heads are Rejected and write nothing;
- concurrent commits produce one winner per expected head;
- Snapshots remain immutable after later commits;
- reopen and replay produce identical visible state;
- every committed Revision remains addressable after reopen;
- compaction preserves every Revision and every live view;
- failure after the possible commit point returns Indeterminate and resolves by Operation Identity;
- a crash before the caller receives an outcome is recoverable from its previously retained Operation Identity;
- repeating one Operation Identity with the same canonical payload creates at most one Revision and returns the same resolution;
- repeating one Operation Identity with different content is Rejected as OperationIdentityConflict;
- persisted operation digest versions remain comparable across every supported Revision-journal version;
- crash injection during Point Set staging never creates a Revision and resolves as Rejected or NotRecorded;
- NotRecorded guarantees that the Operation Identity created no Revision;
- Point Sets from another Source or Revision are Rejected;
- Point ordinals outside `0..point_count` and patches outside the editable Attribute schema are Rejected;
- position patches are rejected in v0.1;
- unknown Point Identities and invalid Breaklines reject the entire batch; and
- sparse storage growth is proportional to changed Points and features.

Fault injection cuts, corrupts, or fails every write, flush, sync, and rename point. Recovery must expose either the previous head or the complete next head.

### point-query

The reference oracle sequentially scans the Source, applies the Snapshot overlay, and evaluates predicates.

QueryEngine construction rejects Source Identity, point-count, editable-schema, Coordinate Reference, or Spatial Index binding mismatch before producing a Snapshot.
Snapshot pinning resolves the Revision Identity through that validated RevisionStore and rejects an unknown Revision.

Generate combinations of:

- Region shape and edge inclusion;
- Classification and flag filters;
- requested Attribute columns;
- empty, dense, and sparse overlays;
- batch partitioning;
- cancellation offsets; and
- concurrent commits after Query submission.

Assert exact equality of Point Identity and requested values, stable ordinal order, no duplicates, a matching ExactPointSummary, and bounded memory. A Query can be slow without an index, but it cannot be partial. Screen-through cases include polygon edges and deliberately ignore occlusion.

### render-protocol

Generate Reset, Upsert, replacement, conditional Remove, and SetHighlights
sequences:

- a generation begins with exactly one Reset;
- stale View identities and generations are rejected;
- Upserts must strictly increase the last batch version;
- conditional Remove rejects a mismatched resident version;
- point, estimated-byte, and batch limits are enforced independently;
- rejected updates leave state unchanged; and
- the CPU reference state exposes the expected aggregate residency and
  deterministic highlight set.

### point-view

For fixed camera, viewport, hierarchy, and hard budgets:

- nodes outside any frustum plane are culled conservatively;
- screen-space error selects the expected hierarchy cut;
- point, estimated-byte, and batch limits independently block new requests;
- requested nodes reserve budget and are not requested twice;
- a parent remains retained until every selected visible replacement is
  resident;
- resident descendants remain retained while a selected coarse parent loads;
- hysteresis prevents refinement from oscillating inside the configured dead
  band;
- shuffled node input produces byte-for-byte equal request, retention, and
  retirement order;
- malformed, cyclic, or spatially inconsistent hierarchies fail before planner
  history changes;
- a generation change resets hysteresis history; and
- every retirement carries the exact generation, batch key, and observed
  version.

Run those contracts entirely through the planner interface without a GPU. A
separate headless acceptance test materializes planned synthetic batches,
applies them to **render-wgpu**, and proves that coarse Coverage is retired only
after its replacements render within the same fixed limits. The optimized
planner benchmark uses the demo-scale 5,461-node hierarchy representing more
than 10 million logical Points.

### terrain-model

Validate the Terrain Surface through public output:

- every face references three existing, distinct vertices;
- face area is nonzero within the documented numeric model;
- orientation is consistent;
- manifold expectations and boundary loops hold;
- every normalized Breakline is represented by constrained edges;
- boundaries and holes exclude the correct faces;
- duplicate and coincident-XY policies are honored;
- crossing constraints are noded or rejected according to the Recipe;
- Point and Breakline terminal provenance mismatch is rejected;
- detached input cannot claim a Workspace Revision;
- signed-zero, tie-to-even grid rounding, exact intersection, and near-grid cases follow the recorded numeric model;
- each TerrainLimit, including vertices created by constraint noding, fails predictably;
- diagnostics and temporary storage remain bounded;
- input batch partitioning and worker count do not change topology;
- repeated runs produce the same canonical topology hash; and
- Artifact Identity is stable across batch partitioning and changes when provenance or canonical topology changes.

Compare small unconstrained cases with a separate simple Delaunay oracle. Use exact or adaptive predicate stress cases around collinearity and cocircularity.

Do not assert private triangulation insertion order.

### landxml

Verification has three levels:

1. parse emitted XML with an independent XML parser;
2. validate the selected LandXML schema and module semantic rules; and
3. compare reconstructed vertices, faces, Breaklines, boundaries, units, and Coordinate Reference with the input Terrain Surface.

Also assert hard byte-chunk limits, one terminal LandXmlReport, cancellation behavior, and no destination-file claim inside the encoder tests.

Golden tests compare canonical semantic trees first and exact bytes only where deterministic byte output is promised.

Maintain fixtures produced by independent consumer tools when licensing permits. A real CAD round trip is a release qualification test once automation is available; XML validity alone is not treated as interoperability proof.

### render-wgpu

Use synthetic render-protocol point updates with offscreen targets:

- render one Point, dense overlapping Points, classifications, highlights, and empty input;
- exercise very large world origins with small relative offsets;
- enforce logical residency and device buffer limits;
- return a resource error rather than silently evicting active batches;
- verify that command recording performs no Source-scale I/O and submission
  remains host-owned;
- preserve exact cameras across multiple frames recorded before one submission;
- keep replaced batch resources and identity metadata alive through
  `RecordedFrame`;
- reject frames from a different View generation and recorded frames from a
  different renderer;
- verify explicit coarse-batch replacement and conditional removal;
- test asynchronous one-pixel pick hits only as provisional candidates; and
- compare tolerant image statistics or perceptual hashes, not fragile byte-identical pixels across GPU vendors.

Qualify available Vulkan, Metal, Direct3D 12, and software fallback adapters
locally. Rendering is not required to be pixel-identical across GPUs.

### point-workspace

Composition tests use real lower modules with small fixtures and temporary directories:

- open a new and existing Workspace;
- preserve Workspace Identity across reopen and directory moves;
- bind exactly one Source and reject attempts to attach another;
- expose no Snapshot before full Source registration;
- reject a missing, changed, or rebound Source;
- recover the Revision journal before exposing the head;
- report Ready or Missing IndexStatus and run the explicit preparation Job;
- refuse ViewInput while the index is missing or building;
- pin Snapshots across commits;
- run exact Queries through the Snapshot interface;
- enforce one writer and supported concurrent readers;
- delete disposable caches and reopen to the same logical state;
- cancel every exposed Job;
- reconcile every injected Indeterminate commit by Operation Identity; and
- surface lower-module errors without losing structured context.

Do not replace every lower module with mocks. The memory Source and fault-injection filesystem adapters exist to make real composition deterministic.

## Cross-module invariant tests

These tests are more valuable than screenshot-only application tests:

1. **Identity chain:** decode → index → Query → View → pick hint retains the original Point Identity.
2. **Edit chain:** exact Query → Point Set → commit → reopen → Query returns the patched Attribute while Source bytes remain unchanged.
3. **Determinism chain:** Source + Revision + Recipe → Terrain Surface → LandXML produces identical topology and semantic export across repeat runs.
4. **Precision chain:** quantized Source value → CPU world value → origin-relative View value never causes the display value to become analytical input.
5. **Recovery chain:** kill during index, commit, Derivation, and atomic file output; reopen exposes only states allowed by [contracts.md](contracts.md).
6. **Deletion chain:** remove the index and every cache; stable Point Identity, Revision state, Query results, and independently recorded Artifact provenance remain valid after rebuilding.

## Fuzzing

Fuzz seams where untrusted bytes or combinatorial geometry enter:

- LAS/LAZ headers, VLRs, EVLRs, records, chunk tables, and extra-byte schemas;
- COPC hierarchy pages and local byte-range slices;
- index and Revision persisted frames;
- Point Set spill headers, segments, and checksums;
- Point Batch column lengths and Attribute descriptors;
- render-protocol delta sequences and batch lengths;
- Region and filter expression decoding;
- Breakline normalization and terrain predicates;
- LandXML input options and validation; and
- Workspace manifests.

Fuzz targets must set strict allocation, recursion, and elapsed-work guards. “No panic” is insufficient: malformed inputs must also avoid unbounded memory and persistent partial state.

Every minimized crash becomes a permanent fixture at the owning module's interface.

## Benchmark scenarios

Start with generated, reproducible scenarios; add public real Sources as licensing permits.

| Scenario | Sizes | Primary measurements |
|---|---|---|
| Sequential decode | 1M, 10M, 100M Points | Points/s, bytes/s, peak memory |
| Index build and resume | 10M, 100M, 1B generated Points | time, bytes/Point, write amplification, resume work |
| Spatial Query | tiny box, corridor, polygon, full extent | first-batch time, total time, candidate amplification, memory |
| Point Set materialization | 1K, 1M, 100M identities | memory, spill bytes, first/second iteration time |
| Sparse Edits | 1K, 1M, 10M changed Points | commit time, journal growth, reopen time |
| View preparation | fixed cameras and budgets | first View delta, refinement time, emitted Points, memory |
| Rendering | 1M, 5M, 20M resident Points plus mesh fixtures | frame percentiles, upload time, GPU memory |
| Terrain | 10K, 100K, 1M candidate vertices | time, peak memory, topology hash |
| LandXML | 100K, 1M faces | encode time, validation time, output size |

Do not claim universal latency targets. Establish a named reference workstation and SSD, commit baselines, and fail performance CI only on statistically meaningful regressions after the benchmark stabilizes.

Memory ceilings are correctness tests, not optional benchmark notes.

## CI lanes

### Pull request

- format and lint;
- dependency-allowlist check;
- documentation examples;
- all small unit, contract, oracle, property, and recovery tests;
- short fuzz corpus;
- headless offscreen renderer test where available; and
- Workspace composition tests.

### Nightly

- long property runs;
- long fuzzing;
- large generated Sources;
- crash matrices;
- multiple worker counts;
- graphics backend matrix;
- persisted-version fixture matrix; and
- benchmark recording.

### Release qualification

- all nightly checks;
- public real-Source corpus;
- install and open on supported operating systems;
- LandXML semantic round trips with available external consumers;
- disk-full, permission, path-length, and device-loss exercises;
- migration from every supported persisted version; and
- license and fixture-provenance audit.

## Definition of verified

A module is not complete because its happy path works. It is verified when:

- every interface invariant has a named test or an explicit reason it cannot be automated;
- the public interface alone is sufficient to run its conformance suite;
- ordering and cancellation behavior are tested;
- failure effects are tested;
- persisted state survives fault injection where applicable;
- Source-scale work has an enforced memory ceiling;
- deterministic results remain deterministic across repeat runs and worker counts;
- optimized output matches a simple oracle for small cases; and
- at least one direct-use example proves the module works outside a Workspace or viewer.
