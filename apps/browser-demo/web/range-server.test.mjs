import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { once } from "node:events";
import { cp, mkdtemp, rm } from "node:fs/promises";
import { createConnection } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

import { loadStreamingProtocol } from "./module-loader.js";

const { runStreamingOperation } = await loadStreamingProtocol("range-server-test");

const serverPath = fileURLToPath(
  new URL("../../../scripts/serve-browser-demo.py", import.meta.url),
);
const webRoot = fileURLToPath(new URL("./", import.meta.url));

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
