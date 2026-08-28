import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  cpSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { captureChildExit } from "./child-process.mjs";
import {
  BROWSER_SDK_REFERENCE_SECTIONS,
  publicDeclaration,
} from "./generate-browser-sdk-reference.mjs";

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const artifactDirectory = path.join(repositoryRoot, "target/npm");
const sourceViewerManifest = JSON.parse(readFileSync(
  path.join(repositoryRoot, "apps/browser-demo/web/package.json"),
  "utf8",
));
let developmentServerSequence = 0;
const viewerArtifact = onlyArtifact("punctra-viewer-");
const reactArtifact = onlyArtifact("punctra-react-");

verifyPackedFiles(viewerArtifact, [
  "package/camera-policy.js",
  "package/exact-query.d.ts",
  "package/exact-query-error.js",
  "package/exact-query.js",
  "package/las-exact-decoder.js",
  "package/module-loader.js",
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
verifyQualificationConsumer();
verifyPackedFiles(reactArtifact, [
  "package/hook.js",
  "package/index.d.ts",
  "package/index.js",
  "package/lifecycle.js",
  "package/package.json",
]);
verifyGeneratedApiReference(viewerArtifact, reactArtifact);

run("node", ["--test", "packages/react/lifecycle.test.mjs"], repositoryRoot);
await verifyTrial("browser-typescript", [viewerArtifact], {
  publishDistribution: true,
  requireCodeSplit: true,
  runPackageTests: true,
});
await verifyTrial("browser-react", [viewerArtifact, reactArtifact], { runPackageTests: true });

console.log("browser SDK packed-artifact trials passed");

function verifyQualificationConsumer() {
  const qualificationRoot = path.join(repositoryRoot, "apps/browser-demo/web");
  const viewerPackage = path.join(qualificationRoot, "node_modules/@punctra/viewer");
  const packageManifest = JSON.parse(readFileSync(path.join(viewerPackage, "package.json"), "utf8"));
  assert.equal(packageManifest.name, sourceViewerManifest.name);
  assert.equal(packageManifest.version, sourceViewerManifest.version);
  const index = readFileSync(path.join(qualificationRoot, "index.html"), "utf8");
  assert.match(index, /"@punctra\/viewer":\s*"\.\/node_modules\/\@punctra\/viewer\/sdk\.js"/);
  assert.match(index, /"@punctra\/viewer\/input":\s*"\.\/node_modules\/\@punctra\/viewer\/viewer-input\.js"/);
  assert.match(index, /"@punctra\/viewer\/exact-query":\s*"\.\/node_modules\/\@punctra\/viewer\/exact-query\.js"/);
  const worker = readFileSync(path.join(qualificationRoot, "qualification-worker.js"), "utf8");
  assert.match(worker, /node_modules\/\@punctra\/viewer\/stream-worker\.js/);
}

function verifyGeneratedApiReference(viewer, react) {
  const reference = readFileSync(path.join(repositoryRoot, "docs/api/browser-sdk.md"), "utf8");
  const viewerManifest = packedJson(viewer, "package/package.json");
  const reactManifest = packedJson(react, "package/package.json");
  assert.ok(reference.includes(`packed in Punctra \`${viewerManifest.version}\``));
  const packedPackages = {
    viewer: { artifact: viewer, packageName: viewerManifest.name },
    react: { artifact: react, packageName: reactManifest.name },
  };
  for (const { title, packageKey, declarationPath } of BROWSER_SDK_REFERENCE_SECTIONS) {
    const { artifact, packageName } = packedPackages[packageKey];
    const expectedDeclaration = publicDeclaration(
      declarationPath,
      packedText(artifact, `package/${declarationPath}`),
    ).trim();
    const section = readReferenceSection(reference, title);
    assert.equal(
      section.packedDeclaration,
      `${packageName}/${declarationPath}`,
      `${title} names the wrong packed declaration`,
    );
    assert.equal(
      section.declaration,
      expectedDeclaration,
      `${title} differs from packed ${packageName}/${declarationPath}`,
    );
  }
}

function readReferenceSection(reference, title) {
  const heading = `## ${title}\n\n`;
  const headingStart = reference.indexOf(heading);
  assert.notEqual(headingStart, -1, `generated API reference omitted ${title}`);
  const bodyStart = headingStart + heading.length;
  const nextHeading = reference.indexOf("\n\n## ", bodyStart);
  const body = reference.slice(
    bodyStart,
    nextHeading === -1 ? reference.length : nextHeading,
  ).trimEnd();
  const parsed = /^Packed declaration: `([^`]+)`\n\n```ts\n([\s\S]*?)\n```$/.exec(body);
  assert(parsed, `generated API reference has a malformed ${title} section`);
  return { packedDeclaration: parsed[1], declaration: parsed[2] };
}

function packedJson(artifact, entry) {
  return JSON.parse(packedText(artifact, entry));
}

function packedText(artifact, entry) {
  return run("tar", ["-xOzf", artifact, entry], repositoryRoot).stdout;
}

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

async function verifyTrial(
  name,
  artifacts,
  {
    publishDistribution = false,
    requireCodeSplit = false,
    runPackageTests = false,
  },
) {
  const temporaryRoot = mkdtempSync(path.join(tmpdir(), `punctra-${name}-`));
  const trial = path.join(temporaryRoot, name);
  try {
    cpSync(path.join(repositoryRoot, "examples", name), trial, { recursive: true });
    if (publishDistribution) prepareQuickstartFixture(trial);
    run("npm", ["ci", "--ignore-scripts", "--no-audit", "--no-fund"], trial);
    run("npm", ["install", "--ignore-scripts", "--no-audit", "--no-fund", "--no-save", ...artifacts], trial);
    if (runPackageTests) run("npm", ["test"], trial);
    run("npm", ["run", "typecheck"], trial);
    run("npm", ["run", "build:development"], trial);
    verifyProductionAssets(path.join(trial, "dist"), requireCodeSplit);
    run("npm", ["run", "build"], trial);
    verifyProductionAssets(path.join(trial, "dist"), requireCodeSplit);
    if (publishDistribution) {
      const packedViewerArtifact = artifacts.find((artifact) => path.basename(artifact).startsWith("punctra-viewer-"));
      assert(packedViewerArtifact, "packed quickstart is missing its viewer artifact");
      publishQuickstart(path.join(trial, "dist"), packedViewerArtifact);
    }
    await verifyDevelopmentServer(trial);
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true });
  }
}

function prepareQuickstartFixture(trial) {
  const fixtureSource = path.join(repositoryRoot, "apps/browser-demo/web/fixtures/v1");
  const fixtureTarget = path.join(trial, "public/fixtures/v1");
  cpSync(fixtureSource, fixtureTarget, { recursive: true });
}

function publishQuickstart(distribution, viewerArtifact) {
  const target = path.join(repositoryRoot, "target/browser-quickstart");
  rmSync(target, { recursive: true, force: true });
  cpSync(distribution, target, { recursive: true });
  writeFileSync(
    path.join(target, "punctra-packed-runtime.json"),
    `${JSON.stringify({
      schema: "punctra-browser-packed-runtime-v1",
      build: "production",
      serverContract: "punctra-strict-range-v1",
      viewerPackage: sourceViewerManifest.name,
      viewerVersion: sourceViewerManifest.version,
      viewerArtifactSha256: createHash("sha256").update(readFileSync(viewerArtifact)).digest("hex"),
    }, null, 2)}\n`,
    "utf8",
  );
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
  const manifest = JSON.parse(readFileSync(path.join(distribution, ".vite", "manifest.json"), "utf8"));
  if (requireCodeSplit) verifyCodeSplitSdk(manifest);
  const copiedWorker = manifest["node_modules/@punctra/viewer/stream-worker.js"]?.file;
  assert(copiedWorker, "production manifest omitted copied-asset Worker resolution");
  const bundledWorker = verifyBundledWorker(distribution, files);
  verifyResolvedWorker(distribution, manifest, bundledWorker);
  verifyEmittedModuleGraph(distribution, files, new Set([copiedWorker]));
}

function verifyResolvedWorker(distribution, manifest, bundledWorker) {
  const resolvedWorkerModule = manifest[
    "node_modules/@punctra/viewer/stream-worker.js?worker&url"
  ]?.file;
  assert(
    resolvedWorkerModule,
    "production manifest omitted the public resolved Worker URL",
  );
  const resolvedWorkerSource = readFileSync(path.join(distribution, resolvedWorkerModule), "utf8");
  assert(
    resolvedWorkerSource.includes(bundledWorker),
    "public Worker resolution differs from the bundled default Worker",
  );
}

function verifyCodeSplitSdk(manifest) {
  const sdkEntry = "node_modules/@punctra/viewer/sdk.js";
  assert(
    manifest["index.html"]?.dynamicImports?.includes(sdkEntry),
    "application entry does not retain the SDK dynamic-import boundary",
  );
  assert.equal(manifest[sdkEntry]?.isDynamicEntry, true, "SDK is not a production dynamic entry");
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
  return bundledWorker;
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
  const port = 41_000 + ((process.pid + developmentServerSequence) % 1_000);
  developmentServerSequence += 1;
  const viteEntry = path.join(directory, "node_modules/vite/bin/vite.js");
  const child = spawn(
    process.execPath,
    [viteEntry, "--force", "--host", "127.0.0.1", "--port", String(port), "--strictPort"],
    { cwd: directory, stdio: ["ignore", "pipe", "pipe"] },
  );
  const output = collectChildOutput(child);
  const serverExit = captureChildExit(child);
  try {
    await Promise.race([
      verifyDevelopmentResponses(directory, port),
      serverExit.then((settlement) => {
        throw developmentServerExitError(settlement, output);
      }),
    ]);
  } finally {
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGTERM");
    await serverExit.catch(() => {});
  }
}

async function verifyDevelopmentResponses(directory, port) {
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
}

function collectChildOutput(child) {
  const output = { stdout: "", stderr: "" };
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk) => { output.stdout += chunk; });
  child.stderr.on("data", (chunk) => { output.stderr += chunk; });
  return output;
}

function developmentServerExitError(settlement, output) {
  const reason = settlement.signal === null
    ? `code ${settlement.code}`
    : `signal ${settlement.signal}`;
  const diagnostics = `${output.stdout}\n${output.stderr}`.trim();
  return new Error(`Vite development server exited with ${reason}${diagnostics ? `:\n${diagnostics}` : ""}`);
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
