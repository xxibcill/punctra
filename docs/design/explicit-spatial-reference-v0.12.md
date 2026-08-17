# Explicit Spatial Reference and Package Publication Design (v0.12)

Status: **Implemented and locally repository-verified on 2026-08-17**

This design is authoritative for the narrow Punctra v0.12 repository slice.
The activation decision records the maintainer's request to start v0.12; it is
not evidence that a licensed production corpus, observed workflow, or accepted
deliverable established the right field profile. Those external gates remain
outstanding.

The repository contains LAS/LAZ examples with GeoTIFF projection records that
the v0.11 adapter preserves but reports as an unknown Coordinate Reference. The
same repository also contains opaque WKT and files without complete vertical
reference facts. This is sufficient to exercise a fail-closed technical
contract, but it is not permission to infer missing metadata or publish the
files as production evidence.

## Outcome

Punctra v0.12 makes one projected survey-coordinate profile explicit from
verified Source metadata through Source reopen, Workspace lineage, Terrain
Derivation, detached QA, and LandXML export. The supported workflow profile is:

- a nonzero horizontal EPSG reference identity;
- a nonzero vertical EPSG reference identity;
- X=easting, Y=northing, and Z=elevation axis meaning;
- an explicit horizontal linear unit and vertical linear unit;
- exact Source scale/offset as the coordinate precision contract; and
- provenance stating whether the complete profile came from verified Source
  metadata or an explicit caller declaration.

The contract vocabulary recognizes metres, international feet, and US survey
feet so unsupported inputs can be represented and rejected accurately. The
v0.12 Terrain, QA-tolerance, and LandXML path accepts metre/metre only and does
no transformation. No unit, axis, datum, geoid, or Coordinate Reference is
guessed.

The v0.12 adoption exit is part of the same release: every supported library
crate has complete crates.io metadata, versioned path dependencies, documented
feature behavior and dependency role, and a locally exercised package/docs.rs
path.

## Evidence boundary

Repository completion proves generated and redistributable fixtures, frozen
legacy compatibility, local packaging, documentation, and CPU/GPU checks. It
does not prove:

- that the selected profile is the recurring profile in a permitted
  production corpus;
- that a particular EPSG identity or vertical datum applies to any untracked
  example file;
- that a downstream application honors LandXML CoordinateSystem metadata;
- coordinate transformation, survey calibration, or geoid conversion;
- partner acceptance, production support, independent adoption, or a
  professional time saving; or
- crates.io publication. `cargo package` validates publishable artifacts but
  does not upload them.

## Public contract

`point-contracts` owns four small serializable values:

1. `LinearUnit` distinguishes metre, international foot, and US survey foot.
2. `SpatialAxes` fixes the accepted easting/northing/elevation order. Later
   axis orders require a new explicit variant and owning behavior.
3. `SpatialReferenceProvenance` distinguishes verified Source metadata from a
   caller declaration. It is a fact about where the complete structured
   profile came from, not proof that an external authority accepted it.
4. `SpatialReferenceProfile` contains horizontal and vertical EPSG identities,
   axes, units, and provenance. Construction rejects zero identities.

`CoordinateReference` retains its byte-compatible `Unknown` and opaque `Wkt`
wire variants and adds a `Profile` variant. Existing WKT remains preserved but
is not parsed heuristically into the new profile. A complete profile has a
canonical fixed-width byte representation for hashing at authoritative
boundaries.

This deliberately avoids a general CRS object model, PROJ pipeline, WKT
normalizer, authority database, dynamic datum epoch, local engineering
calibration, or arbitrary axis list.

## Source and reopen behavior

`SourceMetadata` continues to contain exactly one `CoordinateReference`.
Structured profiles therefore flow through the existing versioned
`SourceRecord` and adapter verification without a second source of truth. The
frozen SourceRecord-v1 `Unknown` and `Wkt` encodings must reproduce byte for
byte.

`source-las` adds one strict GeoTIFF-key path. It publishes a structured profile
only when exactly one projection key directory supplies all of the following
as direct inline values:

- projected model type;
- a non-user-defined projected EPSG identity;
- a recognized horizontal linear-unit EPSG code;
- a non-user-defined vertical EPSG identity; and
- a recognized vertical linear-unit EPSG code.

The accepted unit codes are EPSG 9001 (metre), 9002 (international foot), and
9003 (US survey foot). A WKT record, duplicate key directory, indirection,
unsupported key version, duplicate key, missing key, user-defined value,
unknown unit, malformed byte count, or WKT/GeoTIFF coexistence does not produce
a structured profile. Raw VLR/EVLR bytes remain preserved exactly, and the
Coordinate Reference remains opaque WKT or explicitly unknown as appropriate.

This is parsing, not transformation. The adapter never consults a network or
an authority database and never changes position ticks, scale, offset, or
Source order.

## Workspace and Revision binding

The Workspace Source-contract digest includes the canonical structured spatial
profile, or the exact bounded opaque WKT bytes when WKT is present. Existing
`Unknown` Source contracts retain their v1 digest so frozen Workspace-v1
fixtures and Revision identities remain unchanged. A Workspace created from a
profiled or WKT Source cannot reopen against the same point bytes with a
different Coordinate Reference.

No Workspace file layout, Point Set layout, Revision row layout, or Operation
record version changes. Snapshot and Revision lineage retain spatial identity
through the verified Source identity and Source-contract digest; they do not
copy mutable display metadata into every provenance value.

## Terrain, QA, and export

Terrain Derivation already copies the Source Coordinate Reference and exact
position transform into `TerrainDescriptor`. v0.12 additionally exposes the
structured profile directly and includes its canonical bytes in Artifact
hashing. The Source contract already binds the same profile before the Recipe
is evaluated. Derived vertices remain exact Source ticks.

Detached Check Point QA uses the same Terrain descriptor. The current public
residual and tolerance path is newly accepted for the metre/metre,
easting/northing/elevation profile. A non-metre structured profile fails before
evaluation; no numeric result is published under an incorrect metre label. The
frozen legacy unknown-reference fact remains readable only for compatibility
as described below.

The new LandXML path likewise requires the Surface's complete supported
profile. It emits exactly one LandXML 1.2 `CoordinateSystem` before `Units`,
using stable EPSG labels for the horizontal coordinate-system and vertical
datum attributes, plus the already explicit Metric `linearUnit="meter"`.
LandXML point text remains Northing, Easting, Elevation, so the axis swap is
declared and tested rather than inferred.

The v0.6-v0.11 boolean metric assertion remains in the frozen journal wire
format only for reading and exactly reconciling legacy generated-v1 workflow
fixtures whose Coordinate Reference was unstructured. No current CLI or public
writer can set it, and every new v0.12 QA/export workflow uses the profile path.

The private round-trip verifier accepts either both legacy files without a
CoordinateSystem or both v0.12 files with exactly the same supported
CoordinateSystem facts. Missing, duplicated, changed, foreign, partially
specified, or unsupported reference metadata is a Coordinate Reference
failure before coordinate tolerances are evaluated.

## Packaging and compatibility

The workspace Cargo version becomes `0.12.0-alpha.1`; persisted schema and
algorithm versions remain independent. Publishable library crates declare:

- repository, homepage, docs.rs URL, README, dual license, MSRV, description,
  keywords, and categories;
- `publish = true` explicitly;
- exact `=0.12.0-alpha.1` registry versions alongside local paths for every
  Punctra dependency;
- documented default/optional feature behavior; and
- package content that carries the root README and explicit dual-license
  metadata without private corpus or build artifacts.

The two application crates remain `publish = false`. The dependency-role guide
identifies foundation values, Source adapters, Workspace, View/rendering,
review, and terrain modules, and states pre-v1 compatibility expectations:
Cargo/API versions may break between alpha minors with migration notes, while
frozen persisted versions retain their documented compatibility or fail
closed.

The local packaging gate runs `cargo package --list` and `cargo package`
without upload, in dependency order, and builds documentation with the same
features docs.rs will use. Package contents must not include private or
untracked field data.

## Explicit non-goals

v0.12 does not add:

- coordinate reprojection or any PROJ/GDAL runtime dependency;
- datum shifts, geoid models, epochs, grid files, localization, or calibration;
- automatic WKT interpretation, authority lookup, CRS guessing, or unit
  inference from numeric ranges;
- support for angular/geographic Terrain coordinates;
- a foot-to-metre Terrain, QA, tolerance, or export conversion;
- Workspace schema migration, persisted Terrain, or a new Edit grammar;
- general LandXML CoordinateSystem import;
- crates.io publication, hosted CI, signing, release tagging, or a stability
  promise beyond the documented alpha policy; or
- any field, partner, downstream, adoption, or production-support claim.

## Verification gates

Repository completion requires all applicable local commands in
`CONTRIBUTING.md`, plus:

- public contract construction, bounded deserialization, exact wire, and
  canonical-byte tests for every profile value;
- strict LAS/LAZ GeoTIFF complete, missing, duplicate, indirect, unsupported-
  unit, WKT-conflict, and malformed fixtures;
- SourceRecord reopen and unchanged frozen-v1 reproduction;
- Workspace rejection when only reference semantics change, with frozen-v1
  Workspace identities unchanged;
- Terrain/QA/LandXML success for the supported profile and focused failures for
  missing, unit, axis, horizontal-reference, and vertical-reference drift;
- round-trip reference equality before coordinate-tolerance evaluation;
- package metadata assertions, package-content inspection, clean extracted-
  package builds, and rustdoc with warnings denied; and
- required GPU acceptance with `PUNCTRA_REQUIRE_GPU=1`.

No hosted CI is added. Completion is recorded only from one exact local commit
and continues to name field activation, a complete permitted corpus,
independent adoption, downstream observation, partner validation, and support
qualification as outstanding.
