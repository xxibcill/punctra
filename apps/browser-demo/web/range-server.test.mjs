import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { once } from "node:events";
import {
  cp,
  mkdtemp,
  readFile,
  readdir,
  realpath,
  rm,
} from "node:fs/promises";
import { createConnection } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

import { FOOTPRINT_IMPLEMENTATION_PATHS } from "./footprint-evidence.js";
import { loadStreamingProtocol } from "./module-loader.js";

const { runStreamingOperation } = await loadStreamingProtocol("range-server-test");

const serverPath = fileURLToPath(
  new URL("../../../scripts/serve-browser-demo.py", import.meta.url),
);
const webRoot = fileURLToPath(new URL("./", import.meta.url));
const visualExportFilename = "v0.21-browser-visual-evidence.tar";
const footprintExportFilename = "v0.22-browser-point-footprint-evidence.tar";
const maxVisualExportBytes = 1_243_611_136;

test("qualification pin endpoint binds the running checkout and verifier bytes", async () => {
  const { server, port } = await startServer();
  try {
    const response = await fetch(
      `http://127.0.0.1:${port}/qualification-visual-pins.json`,
    );
    const verifierPath = fileURLToPath(
      new URL("../../../scripts/verify-browser-visual-baseline.mjs", import.meta.url),
    );
    const verifierBytes = await readFile(verifierPath);
    const baseline = JSON.parse(await readFile(
      fileURLToPath(
        new URL("../../../docs/releases/v0.21-browser-visual-baseline.json", import.meta.url),
      ),
      "utf8",
    ));
    const runningPins = {
      implementation_commit: execFileSync(
        "git",
        ["rev-parse", "HEAD"],
        { cwd: fileURLToPath(new URL("../../../", import.meta.url)), encoding: "utf8" },
      ).trim(),
      verifier: {
        path: "scripts/verify-browser-visual-baseline.mjs",
        byte_length: verifierBytes.byteLength,
        sha256: createHash("sha256").update(verifierBytes).digest("hex"),
      },
    };
    assert.equal(response.status, 200);
    assert.equal(response.headers.get("cache-control"), "no-store");
    assert.deepEqual(await response.json(), {
      schema: "punctra-browser-visual-verify-pins-v1",
      accepted: {
        implementation_commit: baseline.pins.implementation_commit,
        verifier: baseline.pins.verifier,
      },
      running: runningPins,
    });
  } finally {
    await stopServer(server);
  }
});

test("point-footprint pin endpoint binds the running checkout and verifier", async () => {
  const { server, port } = await startServer();
  try {
    const response = await fetch(
      `http://127.0.0.1:${port}/qualification-footprint-pins.json`,
    );
    const verifierPath = fileURLToPath(
      new URL("../../../scripts/verify-browser-point-footprint.mjs", import.meta.url),
    );
    const verifierBytes = await readFile(verifierPath);
    const payload = await response.json();
    assert.equal(response.status, 200);
    assert.equal(payload.schema, "punctra-browser-point-footprint-verify-pins-v1");
    assert.equal(payload.running.implementation.commit, execFileSync(
      "git",
      ["rev-parse", "HEAD"],
      { cwd: fileURLToPath(new URL("../../../", import.meta.url)), encoding: "utf8" },
    ).trim());
    assert.deepEqual(payload.running.verifier, {
      path: "scripts/verify-browser-point-footprint.mjs",
      byte_length: verifierBytes.byteLength,
      sha256: createHash("sha256").update(verifierBytes).digest("hex"),
    });
    assert.equal(payload.running.runtime.package_name, "@punctra/viewer");
    assert.equal(payload.running.runtime.package_version, "0.22.0-alpha.1");
    assert.deepEqual(
      payload.running.runtime.artifacts.map(({ path: artifactPath }) => artifactPath),
      [
        "apps/browser-demo/web/package.json",
        "apps/browser-demo/web/pkg/browser_demo.js",
        "apps/browser-demo/web/pkg/browser_demo_bg.wasm",
      ],
    );
    assert.equal(
      payload.running.corpus.path,
      "apps/browser-demo/web/fixtures/footprint-v1/corpus.json",
    );
    assert.equal(payload.running.predecessor.release, "0.21.0-alpha.1");
    const implementationPaths = payload.running.implementation.files.map(
      ({ path: implementationPath }) => implementationPath,
    );
    assert.deepEqual(
      implementationPaths,
      FOOTPRINT_IMPLEMENTATION_PATHS,
      "server implementation pins must exactly match the closed evidence boundary",
    );
    for (const requiredPath of [
      "apps/browser-demo/src/host.rs",
      "apps/browser-demo/src/display.rs",
      "apps/browser-demo/src/lib.rs",
      "apps/browser-demo/web/footprint-corpus.test.mjs",
      "apps/browser-demo/web/footprint-evidence.test.mjs",
      "apps/browser-demo/web/footprint-export.test.mjs",
      "apps/browser-demo/web/footprint-main.js",
      "apps/browser-demo/web/footprint-evidence.js",
      "apps/browser-demo/web/footprint-runner-core.test.mjs",
      "apps/browser-demo/web/visual-capture.js",
      "apps/browser-demo/web/visual-corpus.test.mjs",
      "apps/browser-demo/web/visual-footprint-metrics.test.mjs",
      "apps/browser-demo/web/visual-provenance.js",
      "apps/browser-demo/web/visual-rubric.js",
      "apps/browser-demo/src/streaming.rs",
      "apps/renderer-demo/src/appearance.rs",
      "crates/render-wgpu/tests/offscreen.rs",
      "crates/render-wgpu/src/eye_dome.wgsl",
      "crates/render-wgpu/src/frame.rs",
      "crates/render-wgpu/src/gpu.rs",
      "crates/render-wgpu/src/pick.rs",
      "scripts/build-browser-demo.sh",
      "scripts/serve-browser-demo.py",
      "scripts/verify-browser-point-footprint.mjs",
      "crates/render-wgpu/test-support/gpu.rs",
    ]) {
      assert.equal(implementationPaths.includes(requiredPath), true, requiredPath);
    }
    assert.equal(
      implementationPaths.includes("apps/browser-demo/web/package.json"),
      false,
      "package metadata must be pinned only as a runtime artifact",
    );
    assert.equal(
      implementationPaths.includes("apps/browser-demo/web/pkg/browser_demo.js"),
      false,
      "ignored runtime JavaScript must be pinned only as a runtime artifact",
    );
    assert.equal(
      implementationPaths.includes("apps/browser-demo/web/pkg/browser_demo_bg.wasm"),
      false,
      "ignored runtime Wasm must be pinned only as a runtime artifact",
    );
    const verifierImplementationPin = payload.running.implementation.files.find(
      ({ path: implementationPath }) => (
        implementationPath === "scripts/verify-browser-point-footprint.mjs"
      ),
    );
    assert.deepEqual(verifierImplementationPin, payload.running.verifier);
    assert.equal(payload.accepted === null || typeof payload.accepted === "object", true);
  } finally {
    await stopServer(server);
  }
});

test("opt-in local server persists a bounded visual evidence TAR", async () => {
  const exportDirectory = await mkdtemp(
    path.join(tmpdir(), "punctra-visual-export-"),
  );
  const { server, port } = await startServer([
    "--visual-export-dir",
    exportDirectory,
  ]);
  const origin = `http://127.0.0.1:${port}`;
  const bytes = Uint8Array.from(
    { length: 64 * 1024 + 37 },
    (_, index) => (index * 31 + 7) & 0xff,
  );
  const sha256 = createHash("sha256").update(bytes).digest("hex");
  const resolvedExportDirectory = await realpath(exportDirectory);

  try {
    const response = await fetch(`${origin}/qualification-visual-export`, {
      method: "POST",
      headers: {
        "Content-Length": String(bytes.byteLength),
        "Content-Type": "application/x-tar",
        Origin: origin,
      },
      body: bytes,
    });
    assert.equal(response.status, 201);
    assert.equal(response.headers.get("access-control-allow-origin"), null);
    assert.deepEqual(await response.json(), {
      schema: "punctra-browser-visual-export-receipt-v1",
      filename: visualExportFilename,
      path: path.join(resolvedExportDirectory, visualExportFilename),
      byte_length: bytes.byteLength,
      sha256,
    });

    const persisted = await readFile(
      path.join(exportDirectory, visualExportFilename),
    );
    assert.deepEqual(persisted, Buffer.from(bytes));
    assert.equal(createHash("sha256").update(persisted).digest("hex"), sha256);

    const conflict = await fetch(`${origin}/qualification-visual-export`, {
      method: "POST",
      headers: {
        "Content-Length": String(bytes.byteLength),
        "Content-Type": "application/x-tar",
        Origin: origin,
      },
      body: bytes,
    });
    assert.equal(conflict.status, 409);
    assert.equal(conflict.headers.get("access-control-allow-origin"), null);
    assert.deepEqual(
      await readFile(path.join(exportDirectory, visualExportFilename)),
      persisted,
    );
    assert.deepEqual(await readdir(exportDirectory), [visualExportFilename]);
  } finally {
    await stopServer(server);
    await rm(exportDirectory, { recursive: true });
  }
});

test("opt-in local server keeps the v0.22 footprint export separate", async () => {
  const exportDirectory = await mkdtemp(
    path.join(tmpdir(), "punctra-footprint-export-"),
  );
  const { server, port } = await startServer([
    "--footprint-export-dir",
    exportDirectory,
  ]);
  const origin = `http://127.0.0.1:${port}`;
  const bytes = Uint8Array.of(0x75, 0x73, 0x74, 0x61, 0x72, 0x32, 0x32);

  try {
    const response = await fetch(`${origin}/qualification-footprint-export`, {
      method: "POST",
      headers: {
        "Content-Length": String(bytes.byteLength),
        "Content-Type": "application/x-tar",
        Origin: origin,
      },
      body: bytes,
    });
    assert.equal(response.status, 201);
    assert.deepEqual(await response.json(), {
      schema: "punctra-browser-point-footprint-export-receipt-v1",
      filename: footprintExportFilename,
      path: path.join(await realpath(exportDirectory), footprintExportFilename),
      byte_length: bytes.byteLength,
      sha256: createHash("sha256").update(bytes).digest("hex"),
    });
    assert.deepEqual(await readFile(path.join(exportDirectory, footprintExportFilename)), Buffer.from(bytes));
    assert.deepEqual(await readdir(exportDirectory), [footprintExportFilename]);
  } finally {
    await stopServer(server);
    await rm(exportDirectory, { recursive: true });
  }
});

test("visual evidence export is absent unless explicitly enabled", async () => {
  const { server, port } = await startServer();
  const origin = `http://127.0.0.1:${port}`;
  const bytes = new Uint8Array([0x75, 0x73, 0x74, 0x61, 0x72]);

  try {
    const response = await fetch(`${origin}/qualification-visual-export`, {
      method: "POST",
      headers: {
        "Content-Length": String(bytes.byteLength),
        "Content-Type": "application/x-tar",
        Origin: origin,
      },
      body: bytes,
    });
    assert.equal(response.status, 404);
    assert.equal(response.headers.get("access-control-allow-origin"), null);
  } finally {
    await stopServer(server);
  }
});

test("visual evidence export rejects missing or mismatched origins", async () => {
  const exportDirectory = await mkdtemp(
    path.join(tmpdir(), "punctra-visual-export-"),
  );
  const { server, port } = await startServer([
    "--visual-export-dir",
    exportDirectory,
  ]);
  const origin = `http://127.0.0.1:${port}`;
  const exportUrl = `${origin}/qualification-visual-export`;
  const bytes = new Uint8Array([0x75, 0x73, 0x74, 0x61, 0x72]);

  try {
    for (const requestOrigin of [undefined, "http://localhost:65535"]) {
      const headers = {
        "Content-Length": String(bytes.byteLength),
        "Content-Type": "application/x-tar",
      };
      if (requestOrigin !== undefined) headers.Origin = requestOrigin;
      const response = await fetch(exportUrl, {
        method: "POST",
        headers,
        body: bytes,
      });
      assert.equal(response.status, 403);
      assert.equal(response.headers.get("access-control-allow-origin"), null);
    }

    const wrongType = await fetch(exportUrl, {
      method: "POST",
      headers: {
        "Content-Length": String(bytes.byteLength),
        "Content-Type": "application/x-tar; charset=binary",
        Origin: origin,
      },
      body: bytes,
    });
    assert.equal(wrongType.status, 415);
    assert.equal(wrongType.headers.get("access-control-allow-origin"), null);

    assert.equal(
      await rawVisualExportStatus(port, {
        host: `rebound.invalid:${port}`,
        origin: `http://rebound.invalid:${port}`,
        contentLength: bytes.byteLength,
        body: bytes,
      }),
      403,
    );
    assert.equal(
      await rawVisualExportStatus(port, { origin, contentLength: 0 }),
      400,
    );
    assert.equal(
      await rawVisualExportStatus(port, {
        origin,
        contentLength: maxVisualExportBytes + 1,
      }),
      413,
    );
    assert.equal(
      await rawVisualExportStatus(port, {
        origin,
        contentLength: maxVisualExportBytes,
      }),
      400,
      "the inclusive upper bound must reach exact-length validation",
    );
    assert.equal(
      await rawVisualExportStatus(port, { origin }),
      411,
    );

    const options = await fetch(exportUrl, { method: "OPTIONS" });
    assert.equal(options.status, 204);
    assert.equal(
      options.headers.get("access-control-allow-methods"),
      "GET, HEAD, OPTIONS",
    );
    assert.deepEqual(await readdir(exportDirectory), []);
  } finally {
    await stopServer(server);
    await rm(exportDirectory, { recursive: true });
  }
});

test("strict local server enforces the v0.16 Range and CORS contract", async () => {
  const { server, port } = await startServer();

  try {
    assert.ok(port > 0, "server must report its assigned ephemeral port");
    const fixtureUrl = `http://127.0.0.1:${port}/fixtures/v1/representative.las`;
    const response = await fetch(fixtureUrl, {
      headers: { Range: "bytes=0-255" },
    });
    const bytes = new Uint8Array(await response.arrayBuffer());
    assert.equal(response.status, 206);
    assert.equal(response.headers.get("accept-ranges"), "bytes");
    assert.equal(response.headers.get("content-encoding"), "identity");
    assert.equal(response.headers.get("content-length"), "256");
    assert.equal(response.headers.get("content-range"), "bytes 0-255/2380227");
    assert.equal(
      response.headers.get("etag"),
      '"sha256-bc2b1cb9077505425e051e26920edba152a67f83c2b0b00dc26fc08ed8198697"',
    );
    assert.match(response.headers.get("access-control-expose-headers"), /Content-Range/);
    assert.equal(bytes.byteLength, 256);
    assert.equal(
      createHash("sha256").update(bytes).digest("hex"),
      "d4e3653b70a2199f0658a8c4f77689cb87ff58069c7a73ea03d0e16e55c6dc36",
    );

    const options = await fetch(fixtureUrl, { method: "OPTIONS" });
    assert.equal(options.status, 204);
    assert.equal(options.headers.get("access-control-allow-headers"), "Range");
    assert.equal(options.headers.get("access-control-allow-methods"), "GET, HEAD, OPTIONS");

    const invalid = await fetch(fixtureUrl, {
      headers: { Range: "bytes=0-99999999" },
    });
    assert.equal(invalid.status, 416);

    const moduleResponse = await fetch(
      `http://127.0.0.1:${port}/worker-protocol.js`,
    );
    assert.equal(moduleResponse.status, 200);
    assert.equal(moduleResponse.headers.get("cache-control"), "no-store, no-transform");
  } finally {
    await stopServer(server);
  }
});

test("real local server exposes bounded protocol fault routes", async () => {
  const { server, port } = await startServer();
  const manifestUrl = `http://127.0.0.1:${port}/fixtures/v1/deployment.json`;

  try {
    const manifestResponse = await fetch(manifestUrl);
    const manifest = await manifestResponse.json();
    for (const [fault, expectedCode] of [
      ["disconnect", "offline"],
      ["redirect", "range_unsupported"],
      ["retry", "retry_exhausted"],
      ["truncated", "range_truncated"],
      ["corrupt", "range_corrupt"],
      ["validator_drift", "source_changed"],
    ]) {
      const faultManifest = structuredClone(manifest);
      faultManifest.source.url = `./representative.las?fault=${fault}`;
      const fetchImplementation = async (input, init) => {
        if (String(input) === manifestUrl) {
          return new Response(JSON.stringify(faultManifest), {
            status: 200,
            headers: { "Content-Type": "application/json" },
          });
        }
        return fetch(input, init);
      };

      await assert.rejects(
        runStreamingOperation({
          manifestUrl,
          cacheMode: "none",
          credentials: "omit",
          delay: async () => {},
          fetchImplementation,
        }),
        (error) => {
          assert.equal(error.code, expectedCode, `${fault} fault classification`);
          return true;
        },
      );
    }
  } finally {
    await stopServer(server);
  }
});

test("expected client cancellation does not emit a server traceback", async () => {
  const { server, port, stderr } = await startServer();
  const requestPath = "/fixtures/v1/representative.las?delay_ms=500";
  const socket = createConnection({ host: "127.0.0.1", port });

  try {
    await once(socket, "connect");
    socket.write([
      `GET ${requestPath} HTTP/1.1`,
      `Host: 127.0.0.1:${port}`,
      "Connection: close",
      "",
      "",
    ].join("\r\n"));
    await new Promise((resolve) => setTimeout(resolve, 100));
    socket.resetAndDestroy();
    await waitForStderr(stderr, /delay_ms=500 HTTP\/1\.1" 200/);
    await new Promise((resolve) => setTimeout(resolve, 50));
  } finally {
    socket.destroy();
    await stopServer(server);
  }
  assert.doesNotMatch(stderr(), /Traceback|BrokenPipeError|ConnectionResetError/);
});

test("an alternate verified root retains immutable fixture cache policy", async () => {
  const alternateRoot = await mkdtemp(path.join(tmpdir(), "punctra-browser-root-"));
  await cp(webRoot, alternateRoot, { recursive: true });
  const { server, port } = await startServer(["--root", alternateRoot]);

  try {
    const response = await fetch(
      `http://127.0.0.1:${port}/fixtures/v1/representative.las`,
      { headers: { Range: "bytes=0-255" } },
    );
    assert.equal(response.status, 206);
    assert.equal(
      response.headers.get("cache-control"),
      "public, max-age=31536000, immutable, no-transform",
    );
  } finally {
    await stopServer(server);
    await rm(alternateRoot, { recursive: true });
  }
});

async function startServer(additionalArguments = []) {
  const server = spawn("python3", [
    serverPath,
    "--host",
    "127.0.0.1",
    "--port",
    "0",
    ...additionalArguments,
  ], {
    stdio: ["ignore", "pipe", "pipe"],
  });
  let stderr = "";
  server.stderr.setEncoding("utf8");
  server.stderr.on("data", (chunk) => { stderr += chunk; });
  return { server, port: await listeningPort(server), stderr: () => stderr };
}

async function rawVisualExportStatus(port, {
  host = `127.0.0.1:${port}`,
  origin,
  contentLength,
  body = new Uint8Array(),
}) {
  const socket = createConnection({ host: "127.0.0.1", port });
  await once(socket, "connect");
  const headers = [
    "POST /qualification-visual-export HTTP/1.1",
    `Host: ${host}`,
    "Content-Type: application/x-tar",
    "Connection: close",
  ];
  if (origin !== undefined) headers.push(`Origin: ${origin}`);
  if (contentLength !== undefined) {
    headers.push(`Content-Length: ${contentLength}`);
  }
  socket.end(
    Buffer.concat([
      Buffer.from(`${headers.join("\r\n")}\r\n\r\n`),
      Buffer.from(body),
    ]),
  );

  let response = "";
  for await (const chunk of socket) response += chunk.toString("latin1");
  const match = /^HTTP\/1\.[01] (\d{3})/.exec(response);
  assert.ok(match, `server must return one HTTP status, received ${response}`);
  return Number(match[1]);
}

async function stopServer(server) {
  const exited = once(server, "exit");
  server.kill("SIGINT");
  await exited;
}

async function listeningPort(server) {
  let output = "";
  for await (const chunk of server.stdout) {
    output += chunk;
    const match = /http:\/\/127\.0\.0\.1:(\d+)\//.exec(output);
    if (match) return Number(match[1]);
  }
  throw new Error("local Range server exited before reporting its port");
}

async function waitForStderr(stderr, expected) {
  const deadline = performance.now() + 2_000;
  while (performance.now() < deadline) {
    if (expected.test(stderr())) return;
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  assert.match(stderr(), expected);
}
