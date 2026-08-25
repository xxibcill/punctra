import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import {
  cpSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const artifactDirectory = path.join(repositoryRoot, "target/npm");
const viewerArtifact = onlyArtifact("punctra-viewer-");
const reactArtifact = onlyArtifact("punctra-react-");

verifyPackedFiles(viewerArtifact, [
  "package/camera-policy.js",
  "package/exact-query.d.ts",
  "package/exact-query.js",
  "package/package.json",
  "package/pkg/browser_demo.d.ts",
  "package/pkg/browser_demo.js",
  "package/pkg/browser_demo_bg.wasm.d.ts",
  "package/pkg/browser_demo_bg.wasm",
  "package/range-response.js",
  "package/sdk.d.ts",
  "package/sdk.js",
  "package/stream-ordinals.js",
  "package/stream-worker.js",
  "package/streaming-protocol.js",
  "package/viewer-api.d.ts",
  "package/viewer-api.js",
  "package/viewer-input.d.ts",
  "package/viewer-input.js",
  "package/wasm-loader.js",
  "package/worker-operation.js",
  "package/worker-protocol.js",
]);
verifyPackedFiles(reactArtifact, [
  "package/hook.js",
  "package/index.d.ts",
  "package/index.js",
  "package/lifecycle.js",
  "package/package.json",
]);

run("node", ["--test", "packages/react/lifecycle.test.mjs"], repositoryRoot);
await verifyTrial("browser-typescript", [viewerArtifact], { requireCodeSplit: true });
await verifyTrial("browser-react", [viewerArtifact, reactArtifact], { runPackageTests: true });

console.log("browser SDK packed-artifact trials passed");

function onlyArtifact(prefix) {
  const matches = readdirSync(artifactDirectory)
    .filter((name) => name.startsWith(prefix) && name.endsWith(".tgz"));
  assert.equal(matches.length, 1, `expected one ${prefix} artifact, found ${matches.join(", ")}`);
  return path.join(artifactDirectory, matches[0]);
}

function verifyPackedFiles(artifact, expectedFiles) {
  const actualFiles = run("tar", ["-tzf", artifact], repositoryRoot).stdout
    .split(/\r?\n/)
    .filter(Boolean)
    .sort();
  assert.deepEqual(actualFiles, [...expectedFiles].sort(), `${artifact} contents differ`);
}

async function verifyTrial(name, artifacts, { requireCodeSplit = false, runPackageTests = false }) {
  const temporaryRoot = mkdtempSync(path.join(tmpdir(), `punctra-${name}-`));
  const trial = path.join(temporaryRoot, name);
  try {
    cpSync(path.join(repositoryRoot, "examples", name), trial, { recursive: true });
    run("npm", ["ci", "--ignore-scripts", "--no-audit", "--no-fund"], trial);
    run("npm", ["install", "--ignore-scripts", "--no-audit", "--no-fund", "--no-save", ...artifacts], trial);
    if (runPackageTests) run("npm", ["test"], trial);
    run("npm", ["run", "typecheck"], trial);
    run("npm", ["run", "build:development"], trial);
    verifyProductionAssets(path.join(trial, "dist"), requireCodeSplit);
    run("npm", ["run", "build"], trial);
    verifyProductionAssets(path.join(trial, "dist"), requireCodeSplit);
    await verifyDevelopmentServer(trial);
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true });
  }
}

function verifyProductionAssets(distribution, requireCodeSplit) {
  const files = recursiveFiles(distribution);
  assert(files.some((file) => file.endsWith(".wasm")), "production build omitted the Wasm asset");
  assert(
    files.some((file) => /stream-worker-[A-Za-z0-9_-]+\.js$/.test(file)),
    "production build omitted the hashed module Worker",
  );
  assert(
    files.filter((file) => /-[A-Za-z0-9_-]{6,}\.(?:js|wasm)$/.test(file)).length >= 2,
    "production assets are not content-hashed",
  );
  if (requireCodeSplit) {
    assert(files.filter((file) => file.endsWith(".js")).length >= 3, "dynamic SDK import was not code-split");
  }
  const manifest = JSON.parse(readFileSync(path.join(distribution, ".vite", "manifest.json"), "utf8"));
  const copiedWorker = manifest["node_modules/@punctra/viewer/stream-worker.js"]?.file;
  assert(copiedWorker, "production manifest omitted copied-asset Worker resolution");
  verifyBundledWorker(distribution, files);
  verifyEmittedModuleGraph(distribution, files, new Set([copiedWorker]));
}

function verifyBundledWorker(distribution, files) {
  const emitted = new Set(files.map((file) => file.split(path.sep).join("/")));
  const match = files
    .filter((file) => file.endsWith(".js"))
    .map((file) => readFileSync(path.join(distribution, file), "utf8"))
    .map((source) => source.match(/new Worker\(new URL\(["'`]([^"'`]*stream-worker-[^"'`]+\.js)["'`]/))
    .find(Boolean);
  assert(match, "production SDK does not construct the bundled module Worker");
  const bundledWorker = match[1].replace(/^\//, "");
  assert(emitted.has(bundledWorker), `production SDK references missing Worker ${bundledWorker}`);
}

function verifyEmittedModuleGraph(distribution, files, ignoredFiles) {
  const emitted = new Set(files.map((file) => file.split(path.sep).join("/")));
  for (const file of emitted) {
    if (!file.endsWith(".js") || ignoredFiles.has(file)) continue;
    const source = readFileSync(path.join(distribution, file), "utf8");
    assert.doesNotMatch(
      source,
      /import\s*\(\s*`\.\/[^`]*\$\{/,
      `${file} retains a computed relative module import`,
    );
    for (const specifier of literalRelativeImports(source)) {
      const target = path.posix.normalize(path.posix.join(path.posix.dirname(file), specifier));
      assert(emitted.has(target), `${file} imports missing production module ${target}`);
    }
  }
}

function literalRelativeImports(source) {
  const imports = [];
  const patterns = [
    /\bfrom\s*["'](\.\/[^"']+)["']/g,
    /\bimport\s*\(\s*["'`](\.\/[^"'`$]+)["'`]\s*\)/g,
  ];
  for (const pattern of patterns) {
    for (const match of source.matchAll(pattern)) {
      imports.push(match[1].split(/[?#]/, 1)[0]);
    }
  }
  return imports;
}

async function verifyDevelopmentServer(directory) {
  const port = 41_000 + (process.pid % 1_000);
  const child = spawn(
    "npm",
    ["exec", "vite", "--", "--host", "127.0.0.1", "--port", String(port), "--strictPort"],
    { cwd: directory, stdio: ["ignore", "pipe", "pipe"] },
  );
  try {
    const root = await fetchUntilReady(`http://127.0.0.1:${port}/`);
    assert.match(root, /<script type="module"/);
    const source = await fetchUntilReady(
      `http://127.0.0.1:${port}/${directory.endsWith("browser-react") ? "src/main.tsx" : "src/main.ts"}`,
    );
    assert.match(source, /@punctra|punctra/i);
    const sdk = await fetchUntilReady(
      `http://127.0.0.1:${port}/node_modules/@punctra/viewer/sdk.js`,
    );
    assert.match(sdk, /createViewer/);
    const worker = await fetchUntilReady(
      `http://127.0.0.1:${port}/node_modules/@punctra/viewer/stream-worker.js?punctra-v=development-trial`,
    );
    assert.match(worker, /development-trial|WORKER_CACHE_TOKEN/);
  } finally {
    child.kill("SIGTERM");
    await new Promise((resolve) => child.once("exit", resolve));
  }
}

async function fetchUntilReady(url) {
  let lastError;
  for (let attempt = 0; attempt < 100; attempt += 1) {
    try {
      const response = await fetch(url);
      if (response.ok) return response.text();
      lastError = new Error(`${url} returned ${response.status}`);
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw lastError;
}

function recursiveFiles(directory, prefix = "") {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const relative = path.join(prefix, entry.name);
    return entry.isDirectory()
      ? recursiveFiles(path.join(directory, entry.name), relative)
      : [relative];
  });
}

function run(command, arguments_, cwd) {
  const result = spawnSync(command, arguments_, { cwd, encoding: "utf8" });
  if (result.status !== 0) {
    process.stderr.write(result.stdout);
    process.stderr.write(result.stderr);
    throw new Error(`${command} ${arguments_.join(" ")} failed with ${result.status}`);
  }
  return result;
}
