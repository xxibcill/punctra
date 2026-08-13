# Round-Trip Evidence v1 fixtures

These files pin canonical bytes for the private
`punctra.terrain-demo.landxml-round-trip-evidence.v1` schema. They are generated
from the fixed, non-secret four-Point metric TIN embedded in the owning module's
fixture test. The test reads these committed bytes and compares them with the
production encoder; it never rewrites the fixtures.

| File | Result | Bytes | BLAKE3 |
|---|---|---:|---|
| `passed.json` | semantic pass | 3,015 | `3066d88ff9ee302434b68275e93fa71b3cb4ae3f4a5ee8f53f9e3bf4c06c9ee4` |
| `topology-failed.json` | `PRT_TOPOLOGY_DRIFT`, one removed face | 3,130 | `6a5d948a8ac8a347f31659934d741d8dd78efb87a053df6900ce02e5bf226329` |

Both records pin the effective 4-GiB/10-million-Point/20-million-face
streaming ceilings, XML-token and working-memory limits, and the verifier's
accounted parser/retained peaks for these exact inputs. The topology-failure
fixture also pins the completed mapping count, candidate comparisons, and
maximum deltas that precede its face-set failure. These are deterministic
algorithm-accounting facts, not allocator or process-RSS measurements.

The caller application/version/settings values are declarations only. These
generated files are not evidence that a downstream application ran, a vendor
certified the output, or a firm accepted or paid for a deliverable.
