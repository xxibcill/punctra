# Repository Interoperability Qualification Design (v0.8)

Status: **Accepted but incomplete alpha — bounded file-comparison slices
implemented; remaining repository scope inherited by active v0.9; external
product evidence remains outstanding**

This design is authoritative for the narrow Punctra v0.8 repository slice. It
starts from the completed `0.7.0-alpha.1` technical-readiness work and adds one
post-Run interoperability qualification path. It does not complete the
design-partner MVP, claim that a named downstream application was exercised,
or turn `terrain-demo` into a supported product.

The accepted repository outcome was deliberately smaller than the product
milestone: privately parse a caller-returned LandXML 1.2 TIN, compare it with
the exact LandXML produced by one Complete v0.7 Workflow Run, and publish a
bounded canonical Round-Trip Evidence record. The caller, not Punctra,
declares the downstream application, version, settings, and comparison
tolerances.

The v0.8 alpha stopped after delivery slices 1 and 2. It is not relabeled
Complete. Delivery slices 3 and 4 are inherited as prerequisite closure work
by the active [v0.9 Trust and v1 Candidate design](trust-v1-candidate-v0.9.md).

## Why this is the next slice

v0.7 made the narrow LAS/LAZ correction-to-terrain path restartable and
auditable. Its eight-frame journal binds caller intent, Workspace Revision,
Terrain, QA, LandXML, and `audit.json` without claiming anything about what
happens after the deliverable leaves Punctra.

The next repository risk is semantic drift during an external import/export
round trip. Exact XML bytes are not a useful downstream criterion because an
application may reorder Points and faces, replace local identifiers, reverse a
triangle winding, or rewrite harmless document metadata while preserving the
same TIN. Conversely, a file that still looks plausible may change units,
round coordinates, drop a face, flip a diagonal, or merge nearby vertices.

v0.8 therefore qualifies one semantic boundary without adding general
LandXML import, downstream automation, or a public interoperability framework.

## Accepted outcome

One private `terrain-demo` command will:

1. read one Complete v0.7 Run without repairing or mutating it;
2. revalidate `run.pwf`, `terrain.xml`, and `audit.json` against the existing
   v0.7 checkpoint hashes and identities;
3. read one caller-supplied returned LandXML file and one complete caller
   declaration;
4. parse the original and returned files into the narrow semantic TIN model
   under cumulative hard limits;
5. compare metric units, vertices, tolerances, ambiguity, and face topology;
   and
6. create or exactly reconcile one canonical evidence target outside the Run
   root without overwriting different caller data.

The implemented first-slice CLI shape is:

```text
terrain-demo compare-landxml \
  --application APPLICATION \
  --application-version VERSION \
  --settings-profile PROFILE \
  --horizontal-tolerance-metres H \
  --vertical-tolerance-metres V \
  REFERENCE_LANDXML RETURNED_LANDXML
```

It emits an explicitly non-Run-bound comparison summary. The summary says
`run bound false`, `canonical evidence published false`, and `external
application execution verified false`. It is technical comparison output, not
Round-Trip Evidence.

The planned Run-bound CLI shape for the next delivery slice is:

```text
terrain-demo verify-round-trip \
  --downstream-app APPLICATION \
  --downstream-version VERSION \
  --downstream-setting KEY=VALUE ... \
  --horizontal-tolerance-metres H \
  --vertical-tolerance-metres V \
  RUN_ROOT RETURNED_LANDXML EVIDENCE_TARGET
```

Both commands remain thin private application callers. They do not add a
public function or trait to `point-terrain`, add a new crate, or expose an XML
document model as reusable API. The final command name, required arguments,
and declaration meanings above remain the accepted private application
contract.

## Existing v0.7 state remains unchanged

Interoperability qualification is post-Run evidence, not another Workflow
checkpoint.

- `run.pwf` retains disk version 1, semantic version 1, frame version 1, and
  exactly eight frames.
- `audit.json` retains schema `punctra.terrain-workflow.audit.v1` and its exact
  canonical encoding.
- `terrain.xml`, `audit.json`, the Source, Spatial Index, and Workspace are
  read-only inputs to qualification.
- `start`, `resume`, and `inspect` retain their v0.7 behavior and acceptance
  claims.
- `EVIDENCE_TARGET` must be outside `RUN_ROOT`; qualification never creates a
  fifth fixed Run-root child.
- A torn or non-Complete journal is rejected. The verifier does not invoke the
  repair behavior of `inspect` and cannot make a Run Complete.

This separation permits a v0.7 Run and report to remain byte-identical while
several independently declared downstream attempts produce separate evidence
records.

## Caller declaration

The caller must provide all of the following before parsing begins:

- a non-empty downstream application label;
- a non-empty downstream version string;
- one non-empty opaque settings-profile label;
- a finite, non-negative horizontal tolerance in metres; and
- a finite, non-negative vertical tolerance in metres.

Application, version, and settings profile are opaque caller labels. Empty,
surrounding-whitespace, control-character, or over-limit values are invalid
requests. Punctra neither invents defaults nor interprets vendor-specific
settings. Credentials, license keys, customer names, and other secrets must
not be placed in the profile because later evidence repeats it exactly.

The declaration means only “the caller associates this returned file with
these labels and settings.” It is not proof that the application ran, that the
settings were applied, that the application vendor endorses the result, or
that a firm accepted the deliverable.

## Input identity and race boundary

Qualification first captures regular-file and directory witnesses for the Run
root, v0.7 files, returned file, and evidence parent. Symlinks and non-regular
inputs are rejected. Each input is opened once, read under its captured
identity and length, rechecked after the read, and then parsed from the captured
immutable bytes. Replacement,
truncation, or growth during the operation is a non-evaluated operational
failure rather than a semantic pass or fail.

The implemented witness uses device/inode identity on Unix and volume-serial/
file-index identity on Windows. It fails closed on a filesystem or platform
that does not expose one of those stable identities.

The implemented `compare-landxml` slice applies this boundary only to its two
explicit XML inputs. It neither opens a Run root nor claims the Complete
checkpoint binding described below.

The Complete checkpoint's LandXML and report byte hashes bind the original
files. The verifier also checks the Run Identity and request hash recorded by
the journal and report. The returned file is bound by its BLAKE3 content hash
and exact byte length. Filesystem paths are operational inputs, not semantic
identity, and are not treated as evidence that a downstream application wrote
the file.

## Narrow LandXML semantic model

Both XML inputs must be UTF-8 LandXML 1.2 documents in the exact namespace
`http://www.landxml.org/schema/LandXML-1.2`. XML declarations and bounded
attributes are allowed. DTDs, entity declarations, external entities,
XInclude, and foreign child elements in qualified semantic containers are
rejected. Unrecognized attributes are bounded uninterpreted metadata and do
not affect comparison.

The qualified semantic surface is exactly one TIN:

- one effective `Units/Metric` declaration with `linearUnit="meter"`;
- exactly one `Surfaces/Surface/Definition` with `surfType="TIN"`;
- exactly one `Pnts` collection containing unique positive Point identifiers;
- exactly one `Faces` collection whose faces reference those identifiers;
- three finite decimal coordinates per Point in LandXML northing, easting,
  elevation order; and
- three distinct existing Point references per triangular face.

Whitespace, XML attribute order, Point order, face order, Point identifier
renumbering, surface-name changes, root date/time changes, and triangle winding
are not semantic drift. Signed zero is canonicalized to zero. Duplicate Points,
duplicate faces, non-finite values, empty tokens, integer overflow, degenerate
faces, missing references, a second TIN, Breaklines, boundaries, or another
surface definition fail qualification.

Face orientation uses exact power-of-two scaling independently on easting and
northing before the fast robust predicate. If scaling is not lossless or the
predicate returns zero or a non-finite result, a bounded exact dyadic-integer
fallback decides collinearity across the full finite binary64 range.

At the root, only optional `Project` and `Application` elements may accompany
`Units` and `Surfaces`; they are bounded uninterpreted metadata and are not
reported as supported semantics. A `CoordinateSystem` section is rejected:
v0.8 performs no Coordinate Reference interpretation and does not silently
accept reference drift.

## Bounded fail-closed parsing

The implemented comparison core is independent of the v0.6 encoder. It reads
each regular, non-symlink input under a byte ceiling, parses it with
`roxmltree` under a node ceiling, then builds only the narrow semantic TIN
model. `RoundTripLimits` applies the same per-input ceilings to the reference
and returned files and one cumulative vertex-comparison ceiling. The initial
implemented defaults are:

| Limit | Default |
|---|---:|
| bytes per XML input | 256 MiB |
| XML nodes per input | 8,000,000 |
| XML text/attribute bytes per input | 256 MiB |
| Points per semantic surface | 2,000,000 |
| faces per semantic surface | 4,000,000 |
| candidate vertex comparisons | 32,000,000 |
| application label | 128 B |
| version label | 128 B |
| settings-profile label | 1 KiB |

The file, XML-node, XML-text, Point, face, and comparison ceilings are hard
semantic gates. The DOM and parser allocations are proportional to bounded
input but are not independently measured or fallibly accounted; v0.8 does not
label these ceilings as a measured peak-heap guarantee. The first slice also
does not accept the full 4-GiB v0.7 export ceiling. A later streaming reader is
required before Run-bound qualification can cover that entire range.

An unsupported construct, malformed XML, limit excess, invalid number, unknown
semantic child, or incomplete document never yields a partial semantic model.
No parser recovery, unit inference, axis swap, nearest-neighbor guess, or
partial-coverage result is permitted.

## Comparison rules

### Units and axes

The original and returned documents must both explicitly declare metric metres.
Missing units, Imperial units, another metric linear unit, multiple effective
unit declarations, or a numeric scale that merely appears convertible is
`unit_drift`. v0.8 never rescales coordinates.

Coordinate tuples always mean northing, easting, elevation. The verifier maps
them to world Y, X, Z consistently and never tests swapped axes as a fallback.

### Vertex tolerance

For an original Point `o` and returned Point `r`, a candidate match exists only
when both inclusive tests pass:

```text
hypot(r.easting - o.easting, r.northing - o.northing) <= H
abs(r.elevation - o.elevation) <= V
```

`H` and `V` are the caller-declared metre tolerances recorded in evidence.
Comparisons reject non-finite intermediate results and use a deterministic
bounded spatial lookup; they do not round coordinates to a display precision.
The record includes the maximum observed horizontal and vertical delta for the
accepted mapping.

Every original Point and every returned Point must have exactly one candidate.
Zero candidates are coordinate or count drift. More than one candidate on
either side is `ambiguous_vertex`; the verifier does not choose the closest,
use Point identifiers as a tie-breaker, or use face topology to resolve it.
Thus the accepted mapping is a unique bijection, not an optimizer-selected
correspondence.

### Topology

After the vertex bijection is complete, each returned face is mapped to the
original Point identities and reduced to its three sorted vertex identities.
Triangle winding and order are ignored. The complete sorted sets must be equal.
A missing face, added face, duplicate face, changed diagonal, split/merged
triangle, or reference to an unmatched Point is `topology_drift` even when all
coordinates lie inside tolerance.

Evidence records complete added/removed face counts and hashes plus only a
bounded diagnostic sample. A bounded sample is never presented as the complete
difference set.

### Qualification result

The result is `passed` only when provenance, parsing, units, unique vertex
mapping, tolerances, and topology all pass. A completely evaluated semantic
mismatch produces `failed` evidence and a distinct nonzero CLI result. Checks
that depend on an earlier failed check are explicitly `not_evaluated`.

Invalid caller declarations, I/O failures, cancellation, resource-limit
failures, input races, and indeterminate publication are operational failures;
they publish no final evidence and cannot be counted as either a pass or a
semantic failure.

## Canonical Round-Trip Evidence

The evidence schema is
`punctra.terrain-demo.landxml-round-trip-evidence.v1`. One linear canonical
JSON encoder emits keys in this order:

1. `schema` and `result`;
2. `run` — Run Identity, request hash, Complete journal hash, original
   `terrain.xml` hash/bytes, and v0.7 `audit.json` hash/bytes;
3. `downstream_declaration` — exact application, version, and settings profile;
4. `comparison_policy` — horizontal/vertical tolerances and matcher version;
5. `returned_landxml` — content hash, bytes, namespace, declared units,
   surface name, Point/face counts, and ignored top-level section names;
6. `checks` — provenance, parse, units, unique mapping, tolerance, and topology
   status with stable reason codes;
7. `comparison` — mapped/unmatched/ambiguous Point counts, maximum deltas, and
   added/removed face counts and hashes with bounded samples;
8. `limits` — every effective parser, comparison, and publication limit; and
9. `nonclaims` — explicit false values for Punctra-observed downstream
   execution, vendor certification, firm acceptance, paid use, conversion, and
   measured labor savings.

The record contains no clock-derived field. Identical verified inputs,
declaration, policy, and limits produce identical bytes. Publication uses a
synced staging file, no-replace link, target read-back, parent-directory sync,
and exact-existing reconciliation. A different existing target is a conflict
and is never overwritten. The CLI prints the evidence BLAKE3 hash and byte
length only after publication is known complete; a post-link uncertainty is
reported as indeterminate, not success.

The implemented comparison command currently reports these stable diagnostic
classes:

- `PRT_INVALID_INPUT`;
- `PRT_RESOURCE_LIMIT`; and
- `PRT_SEMANTIC_MISMATCH`.

Slice 3 will refine semantic evidence with stable reason codes beginning with:

- `PRT_XML_INVALID`;
- `PRT_SUBSET_UNSUPPORTED`;
- `PRT_COORDINATE_REFERENCE_UNSUPPORTED`;
- `PRT_UNIT_DRIFT`;
- `PRT_POINT_COUNT_DRIFT`;
- `PRT_VERTEX_UNMATCHED`;
- `PRT_VERTEX_AMBIGUOUS`;
- `PRT_TOLERANCE_DRIFT`; and
- `PRT_TOPOLOGY_DRIFT`.

Operational diagnostics remain distinct and name one safe recovery action.

## Exact external evidence boundary

Repository acceptance and product acceptance are intentionally separate.
Completing the parser, matcher, fixtures, and evidence writer completes only
the v0.8 repository interoperability-qualification slice.

The design-partner MVP product milestone remains outstanding until all three
external gates are documented outside generated repository fixtures:

1. **Three-firm pipeline gate:** at least three distinct firms run the same
   supported Punctra export path through their actual declared downstream
   pipeline on their own production workflow, obtain a passing evidence record,
   and accept the deliverable without bespoke code repair. Repeated teams,
   offices, files, or settings at one firm count as one firm.
2. **Three-paid-pilot gate:** at least three distinct paid pilot engagements
   have payment evidence and production-use evidence. A free evaluation,
   letter of intent, synthetic run, internal demo, or unpaid design-partner
   session does not count.
3. **Two-conversion-or-savings gate:** at least two distinct pilot firms either
   convert to continuing paid use or document measured labor savings large
   enough to justify overlapping incumbent software. Multiple workflows at one
   firm count once; projected savings or unmeasured preference does not count.

The same firm may contribute to all three gates when it independently satisfies
each definition, but the cardinalities remain three distinct firms, three
distinct paid pilots, and two distinct conversion-or-measured-savings firms.
Confidential customer and payment records need not be committed to the
repository, but a release claim must cite an authorized external evidence
ledger that permits the counts to be audited.

A caller declaration or passing Round-Trip Evidence file alone satisfies none
of these gates. Repository-generated or hand-edited returned XML is technical
test evidence only.

## Verification strategy

Repository acceptance will require local tests for:

- a byte-identical return and presentation-only rewrites of whitespace, order,
  identifiers, metadata, and triangle winding;
- inclusive zero and nonzero tolerance boundaries plus just-outside failures;
- unique bijection, zero-candidate, many-candidate, duplicated-Point, and
  adversarial near-neighbor cases;
- missing/added/duplicated faces, diagonal flips, winding changes, degenerate
  triangles, and unknown Point references;
- missing, duplicated, Imperial, and non-metre unit declarations without
  conversion or inference;
- malformed, truncated, deeply nested, oversized-token, entity, extension,
  count, allocation, cancellation, and file-replacement cases;
- Complete/non-Complete Run binding, v0.7 report/LandXML conflict, and strict
  read-only treatment of the Run root;
- canonical pass and fail evidence bytes, exact reconciliation, conflicting
  targets, publication faults, and indeterminate acknowledgement; and
- generated LandXML variants explicitly labeled as generated, not output from
  Civil 3D, Bentley software, or any other named application.

All formatting, linting, tests, documentation checks, and applicable GPU
acceptance remain local as documented in `CONTRIBUTING.md`. Activating this
design does not claim those implementation gates have passed.

## Explicit exclusions

v0.8 does not add:

- a public LandXML parser, importer, document model, exporter registry, plugin
  system, or interoperability crate;
- automation, installation, scripting, licensing, certification, or API
  integration for Civil 3D, Bentley software, or another downstream product;
- a claim that any named downstream application/version/settings combination
  has been run, passed, or become supported;
- unit conversion, unit inference, axis guessing, CRS transformation,
  vertical-reference conversion, geoid handling, or precision repair;
- multiple Surfaces, Breaklines, boundaries, constrained triangulation,
  Profiles, non-TIN definitions, or general LandXML semantics;
- topology repair, nearest-match tie-breaking, best-effort parsing, partial
  Coverage, or tolerance selection on the caller's behalf;
- any v0.7 journal/report schema change, extra Workflow checkpoint, mutable Run
  state, or evidence written inside the Run root;
- product UI, installer, updater, licensing system, support service, telemetry,
  networking, or cloud execution; or
- licensed production datasets, firm acceptance, a paid pilot, conversion, or
  a labor-savings claim supplied by repository tests.

## Delivery slices

Implementation is divided into four independently reviewable slices:

1. **Implemented:** private bounded DOM-backed LandXML reader, semantic model,
   limits, regular-file witnesses, and malformed/resource test matrix;
2. **Implemented:** metric/tolerance/ambiguity/topology comparison with input
   hashes, caller declaration, generated semantic-drift fixtures, and the
   explicitly non-evidence `compare-landxml` CLI;
3. **Pending:** strict Complete-Run binding, canonical pass/fail evidence, and
   no-replace publication/reconciliation faults; and
4. **Pending:** end-to-end generated round-trip matrix, streaming coverage for
   the full v0.7 export ceiling, documentation, independent review, and the
   complete local release gates.

No delivery slice may be described as downstream-application, partner, pilot,
conversion, or labor-savings evidence unless the corresponding external gate
is separately and actually satisfied.
