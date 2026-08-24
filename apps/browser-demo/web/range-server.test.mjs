import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { once } from "node:events";
import { fileURLToPath } from "node:url";
import test from "node:test";

const serverPath = fileURLToPath(
  new URL("../../../scripts/serve-browser-demo.py", import.meta.url),
);

test("strict local server enforces the v0.16 Range and CORS contract", async () => {
  const server = spawn("python3", [serverPath, "--host", "127.0.0.1", "--port", "0"], {
    stdio: ["ignore", "pipe", "pipe"],
  });
  server.stderr.on("data", () => {});
  const port = await listeningPort(server);

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
    const exited = once(server, "exit");
    server.kill("SIGINT");
    await exited;
  }
});

async function listeningPort(server) {
  let output = "";
  for await (const chunk of server.stdout) {
    output += chunk;
    const match = /http:\/\/127\.0\.0\.1:(\d+)\//.exec(output);
    if (match) return Number(match[1]);
  }
  throw new Error("local Range server exited before reporting its port");
}
