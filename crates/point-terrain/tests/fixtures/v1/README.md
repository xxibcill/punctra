# point-terrain Surface disk-v1 fixtures

These files freeze the rebuildable Surface disk-version 1 contract over the
six-point, supported metric-survey `MemorySource` declared in
`tests/v1_fixtures.rs`:

- `bounded-six-point.pterr` is the complete canonical file-backed Surface;
- `bounded-six-point.pterr.surface-work-v1` is the complete input checkpoint
  immediately before triangulation and artifact publication;
- `workspace/manifest.pwm` is the minimal frozen Workspace root that supplies
  the stable Snapshot Workspace and root-Revision identities; and
- `manifest.json` pins the file lengths, BLAKE3 digests, Source/Recipe binding,
  semantic hashes, counts, and bounds.

The tests build a fresh index over the deterministic Source, copy the minimal
Workspace root into an isolated directory, create its empty private
directories, and reopen it through `point_workspace::open`. They inject a
Source read fault only after indexing. The complete artifact must then
warm-open without Snapshot reads, and the work checkpoint must resume with no
Snapshot reads to bytes identical to the frozen complete artifact. Tests write
only disposable copies and verify that every checked-in fixture byte is
unchanged.

## Deliberately gated regeneration

The ignored `regenerate_v1_surface_fixtures` test is the owner-local generator:

```sh
PUNCTRA_REGENERATE_TERRAIN_V1=1 \
  cargo test -p point-terrain --test v1_fixtures \
  regenerate_v1_surface_fixtures -- --ignored --exact
```

It preserves and reopens the checked-in Workspace manifest, regenerates both
terrain files through public APIs, proves resume byte equality, and reproduces
the machine-readable manifest. It refuses to run without the explicit gate,
restores an absent generated file, and fails before overwriting any existing
disk-v1 bytes when the encoder output drifts.

Bootstrapping `workspace/manifest.pwm` is a separate, one-time action because
Workspace identities use operating-system entropy. The generator requires
both `PUNCTRA_REGENERATE_TERRAIN_V1=1` and
`PUNCTRA_BOOTSTRAP_TERRAIN_V1_WORKSPACE=1` when that anchor is absent. Never use
the bootstrap gate to replace this disk-v1 identity. Preserve these files and
add a later fixture version when the persisted contract changes.
