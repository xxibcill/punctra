# Five-minute packed browser quickstart

Punctra `0.20.0-alpha.1` includes one clean TypeScript consumer under
`examples/browser-typescript`. It installs only the packed
`@punctra/viewer` tarball, owns its canvas and application policy, and imports
only the supported root, input, and exact-query entry points.

This is a local integration baseline, not a registry install or general browser
support promise.

## Build and serve it

From the repository root:

```bash
scripts/build-browser-sdk.sh
node scripts/verify-browser-sdk.mjs
scripts/serve-browser-demo.py --root target/browser-quickstart --port 8000
```

Open `http://127.0.0.1:8000/` in the exact browser/device lane being tested.
The verifier has already copied the immutable repository LAS deployment into
the built application's `fixtures/v1` directory. Use the strict server rather
than a generic static server because bounded Range responses, strong validators,
identity encoding, and deliberate delay/disconnect faults are part of the
Source contract.

## Follow the visible workflow

The page initializes one viewer and exposes the host-owned operations in their
safe order:

1. Cancel a delayed load, retry a disconnected-manifest failure in the retained
   viewer, and recreate after a deterministic post-publication cancellation.
2. Load the immutable Source and confirm that the visible Coverage is
   `sampled`, not complete.
3. Switch among neutral, elevation, RGB, intensity, and classification display
   mappings.
4. Switch between perspective and orthographic projection, then orbit, pan, or
   zoom with normalized input.
5. Pick a resident display Point. The result is a
   `provisional_gpu_hint`, not an exact Source record.
6. Highlight the provisional Point for presentation, then confirm it through
   the immutable-LAS bridge. Only the result labeled `exact_source_record` is
   exact authority.
7. Clear the highlight, pause and resume presentation, and dispose the viewer.

Select **Run baseline check** to execute the same deterministic path used by
the attended v0.20 browser check. A pass publishes a
`punctra-browser-quickstart-acceptance-v1` record containing the package
version, Source identity, display/projection coverage, cancellation retention,
retry/recreation outcomes, provisional and exact authority labels, and disposal
result. The packed verifier also publishes a production runtime proof with the
viewer tarball digest. The acceptance flow requires that proof from the strict
Range server, so a development build or generic static host cannot create the
checked-in packed-consumer evidence.

## What the application owns

The application owns the canvas, CSS layout, resize and visibility policy,
input-to-camera mapping, Source URL, credentials, controls, recovery UI, cache
consent, and viewer disposal. The SDK owns the validated viewer lifecycle,
generation safety, bounded streaming, provisional pick identity, presentation
highlights, and exact-query handoff.

Replace the repository manifest only with a deployment that satisfies the
[strict streaming contract](browser-streaming.md). The included exact-query
bridge supports that immutable LAS profile only; it is not arbitrary Source or
Query support.

## Continue from here

Use the [SDK deployment guide](browser-sdk.md) for asset URLs, Vite, React, CSP,
and hosting. Use the [qualification and recovery guide](browser-qualification.md)
before describing a browser/device as qualified. Review the consolidated
[known limitations](browser-known-limitations.md) before integrating the alpha.
