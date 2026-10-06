import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const option = process.argv.indexOf("--artifact");
const artifact = path.resolve(root, option < 0
  ? "target/predecessors/v0.22/npm/punctra-viewer-0.22.0-alpha.1.tgz" : process.argv[option + 1]);
const frozen = JSON.parse(readFileSync(path.join(root, "docs/releases/v0.22-browser-point-footprint-baseline.json"))).pins.runtime;
const quickstart = JSON.parse(readFileSync(path.join(root, "docs/releases/v0.22-browser-quickstart.json")));
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
assert.equal(digest(readFileSync(artifact)), quickstart.acceptance.packedRuntime.viewerArtifactSha256,
  "predecessor package must be the exact frozen v0.22 packed artifact");
for (const [index, relative] of ["package.json", "pkg/browser_demo.js", "pkg/browser_demo_bg.wasm"].entries()) {
  const result = spawnSync("tar", ["-xOf", artifact, `package/${relative}`], { maxBuffer: 2_097_152 });
  assert.equal(result.status, 0, `predecessor ${relative} is unavailable`);
  assert.equal(result.stdout.byteLength, frozen.artifacts[index].byte_length);
  assert.equal(digest(result.stdout), frozen.artifacts[index].sha256, "predecessor runtime differs from accepted bytes");
  const destination = path.join(root, "target/predecessors/v0.22/node_modules/@punctra/viewer", relative);
  mkdirSync(path.dirname(destination), { recursive: true });
  writeFileSync(destination, result.stdout);
}
console.log("exact frozen v0.22 browser runtime staged for paired local timing");
