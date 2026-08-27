import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import {
  QUALIFICATION_LIMITS,
  QUALIFICATION_WORKLOAD,
} from "../apps/browser-demo/web/qualification.js";
import { QUALIFICATION_LANE } from "../apps/browser-demo/web/qualification-lane.js";
import { verifyBrowserQualificationMatrix } from "./verify-browser-qualification.mjs";

const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));
const baselineUrl = new URL("../docs/releases/v0.20-browser-baseline.json", import.meta.url);

export async function verifyBrowserIntegrationBaseline(baseline, qualificationMatrix) {
  assert.equal(baseline.schema, "punctra-browser-integration-baseline-v1");

  const packageVersion = await verifyPackages(baseline.packages);
  assert.equal(baseline.release, packageVersion);
  await verifyDigestRecord(baseline.generated_api_reference);
  await verifyDeployment(baseline.immutable_deployment);
  await verifyGeneratedScene(baseline.generated_scene);
  await verifyPresentationPolicy(baseline.presentation_policy);
  await verifyQuickstart(baseline.quickstart, baseline);

  assert.equal(baseline.qualification.matrix_schema, "punctra-browser-qualification-matrix-v1");
  assert.equal(baseline.qualification.qualified_lane, QUALIFICATION_LANE.id);
  assert.deepEqual(baseline.qualification.limits, QUALIFICATION_LIMITS);
  const matrix = qualificationMatrix ?? await readJson(baseline.qualification.matrix_path);
  assert.equal(matrix.schema, baseline.qualification.matrix_schema);
  assert.equal(matrix.release, baseline.release);
  assert.equal(matrix.implementation_commit, baseline.qualification.implementation_commit);
  assert.equal(matrix.qualified_entries[0].id, baseline.qualification.qualified_lane);
  verifyBrowserQualificationMatrix(matrix, matrix.implementation_commit);

  assert.deepEqual(baseline.recovery, {
    prepublication_worker_or_network: "retry_in_place_after_correction",
    cancelled_load: "retain_viewer_and_last_frame",
    invalid_resize: "retain_prior_viewport",
    partial_publication: "dispose_and_recreate_viewer",
    device_loss: "dispose_and_recreate_viewer_and_device",
    stale_generation: "reject",
    unsupported_initialization: "report_structured_failure_without_fallback",
  });
  assert.deepEqual(baseline.external_evidence, {
    independent_adopter: false,
    registry_or_cdn_publication: false,
    api_stable: false,
    visual_quality_complete: false,
    support_qualified: false,
    beta: false,
    release_candidate: false,
  });
  return true;
}

async function verifyPackages(packages) {
  const viewerManifest = await readJson("apps/browser-demo/web/package.json");
  const reactManifest = await readJson("packages/react/package.json");
  assert.equal(viewerManifest.name, packages.viewer.name);
  assert.equal(viewerManifest.version, packages.viewer.version);
  assert.deepEqual(Object.keys(viewerManifest.exports), Object.keys(packages.viewer.supported_entry_points));
  assert.equal(reactManifest.name, packages.react.name);
  assert.equal(reactManifest.version, packages.react.version);
  assert.equal(reactManifest.peerDependencies["@punctra/viewer"], packages.react.viewer_peer);

  const modules = {
    ".": "apps/browser-demo/web/sdk.js",
    "./input": "apps/browser-demo/web/viewer-input.js",
    "./exact-query": "apps/browser-demo/web/exact-query.js",
  };
  for (const [entryPoint, modulePath] of Object.entries(modules)) {
    const module = await import(`${pathToFileURL(resolvePath(modulePath)).href}?baseline=1`);
    assert.deepEqual(Object.keys(module).sort(), [...packages.viewer.supported_entry_points[entryPoint]].sort());
  }
  const declarations = {
    ".": "apps/browser-demo/web/sdk.d.ts",
    "./input": "apps/browser-demo/web/viewer-input.d.ts",
    "./exact-query": "apps/browser-demo/web/exact-query.d.ts",
  };
  for (const [entryPoint, declarationPath] of Object.entries(declarations)) {
    const declaration = await readText(declarationPath);
    for (const exportName of packages.viewer.supported_entry_points[entryPoint]) {
      assert.match(declaration, new RegExp(`\\b${escapeRegExp(exportName)}\\b`));
    }
  }
  const exactDeclaration = await readText(declarations["./exact-query"]);
  assert.doesNotMatch(exactDeclaration, /decodeLasLayout|decodeLasPointRecord|LasExactQueryLayout/);
  for (const asset of packages.viewer.required_deployable_assets) {
    assert.equal(viewerManifest.files.includes(asset), true, `${asset} must ship in the viewer package`);
  }
  for (const privateModule of packages.viewer.package_private_modules) {
    assert.equal(viewerManifest.files.includes(privateModule), true, `${privateModule} must remain an internal shipped module`);
    assert.equal(Object.hasOwn(viewerManifest.exports, `./${privateModule.replace(/\.js$/, "")}`), false);
  }
  return viewerManifest.version;
}

async function verifyDeployment(deploymentBaseline) {
  for (const record of [
    deploymentBaseline.manifest,
    deploymentBaseline.source,
    deploymentBaseline.index,
    deploymentBaseline.source_record,
  ]) {
    await verifyDigestRecord(record);
  }
  const deployment = await readJson(deploymentBaseline.manifest.path);
  assert.equal(deployment.source.byte_length, deploymentBaseline.source.byte_length);
  assert.equal(deployment.source.sha256, deploymentBaseline.source.sha256);
  assert.equal(deployment.source.source_identity, deploymentBaseline.source.source_identity);
  assert.equal(deployment.source.point_count, deploymentBaseline.source.point_count);
  assert.equal(deployment.index.byte_length, deploymentBaseline.index.byte_length);
  assert.equal(deployment.index.sha256, deploymentBaseline.index.sha256);
  assert.equal(deployment.index.disk_version, deploymentBaseline.index.disk_version);
  assert.equal(deployment.index.recipe_version, deploymentBaseline.index.recipe_version);
  assert.equal(deployment.index.display_sample_schema, deploymentBaseline.index.display_sample_schema);
  assert.equal(deployment.index.root.coverage, deploymentBaseline.root.coverage);
  assert.equal(deployment.index.root.covered_point_count, deploymentBaseline.root.covered_point_count);
  assert.equal(deployment.index.root.display_point_count, deploymentBaseline.root.display_point_count);
  assert.deepEqual(deployment.index.root.world_origin, deploymentBaseline.root.world_origin);
  assert.equal(deploymentBaseline.root.displayed_batches, QUALIFICATION_WORKLOAD.publishedBatches);
}

async function verifyGeneratedScene(scene) {
  const source = await readText("apps/browser-demo/src/scene.rs");
  for (const fixedDeclaration of [
    "ViewGenerationKey::new(ViewId::new(15), 1)",
    "BatchKey::new(1)",
    "BatchVersion::new(1)",
    "SCENE_SIDE: u64 = 33",
    "const SOURCE_ID: SourceId = SourceId::new([0x15; 32])",
    "const WORLD_ORIGIN: [f64; 3] = [500_000.0, 4_600_000.0, 100.0]",
  ]) assert.match(source, new RegExp(escapeRegExp(fixedDeclaration)));
  assert.deepEqual(scene, {
    source_identity: "15".repeat(32),
    point_count: 1089,
    estimated_gpu_bytes: 26136,
    world_origin: [500000, 4600000, 100],
    view_id: 15,
    generation: 1,
    batch_key: 1,
    batch_version: 1,
    centre_point_ordinal: 544,
  });
}

async function verifyPresentationPolicy(policy) {
  const sdk = await import(`${pathToFileURL(resolvePath("apps/browser-demo/web/sdk.js")).href}?presentation=1`);
  const deployment = await readJson("apps/browser-demo/web/fixtures/v1/deployment.json");
  assert.deepEqual(policy.display_modes, sdk.DISPLAY_MODES);
  assert.deepEqual(policy.projections, ["orthographic", "perspective"]);
  assert.equal(policy.deployment_display_mapping, deployment.display_mapping);
  assert.equal(policy.provisional_authority, "provisional_gpu_hint");
  assert.equal(policy.exact_authority, "exact_source_record");
  assert.equal(policy.highlight_authority, "presentation_only");
}

async function verifyQuickstart(quickstart, baseline) {
  const [manifest, controllerSource, mainSource, evidence, packedRuntime] = await Promise.all([
    readJson(`${quickstart.path}/package.json`),
    readText(`${quickstart.path}/src/quickstart.ts`),
    readText(`${quickstart.path}/src/main.ts`),
    readJson(quickstart.evidence.path),
    readJson(quickstart.packed_runtime.path),
  ]);
  await verifyDigestRecord(quickstart.evidence);
  await verifyPackedRuntime(packedRuntime, quickstart.packed_runtime.record, baseline.release);
  assert.equal(manifest.name, "punctra-browser-quickstart");
  assert.equal(quickstart.acceptance_schema, "punctra-browser-quickstart-acceptance-v1");
  assert.match(mainSource, /punctra-packed-runtime\.json/);
  const consumerSource = `${controllerSource}\n${mainSource}`;
  for (const packageName of quickstart.imports) {
    assert.match(consumerSource, new RegExp(escapeRegExp(packageName)));
  }
  assert.deepEqual(quickstart.required_workflow, [
    "cancelled_load_retains_viewer",
    "recoverable_failure_retries_in_place",
    "postpublication_failure_recreates_viewer",
    "strict_source_load",
    "five_display_modes",
    "two_projections",
    "host_navigation",
    "provisional_pick",
    "presentation_highlight",
    "exact_confirmation",
    "highlight_clear",
    "pause_resume",
    "dispose",
  ]);
  verifyQuickstartEvidence(evidence, baseline);
}

async function verifyPackedRuntime(packedRuntime, expected, release) {
  assert.deepEqual(packedRuntime, expected);
  assert.deepEqual(packedRuntime, {
    schema: "punctra-browser-packed-runtime-v1",
    build: "production",
    serverContract: "punctra-strict-range-v1",
    viewerPackage: "@punctra/viewer",
    viewerVersion: release,
    viewerArtifactSha256: packedRuntime.viewerArtifactSha256,
  });
  const artifact = await readFile(resolvePath(`target/npm/punctra-viewer-${release}.tgz`));
  assert.equal(
    packedRuntime.viewerArtifactSha256,
    sha256(artifact),
    "packed runtime proof must name the exact viewer artifact",
  );
}

export function verifyQuickstartEvidence(evidence, baseline) {
  assert.equal(evidence.schema, "punctra-browser-quickstart-evidence-v1");
  assert.match(evidence.observed_on, /^\d{4}-\d{2}-\d{2}$/);
  assert.equal(evidence.lane_id, baseline.qualification.qualified_lane);
  assert.deepEqual(evidence.acceptance, {
    schema: baseline.quickstart.acceptance_schema,
    packageVersion: baseline.release,
    sourceIdentity: baseline.immutable_deployment.source.source_identity,
    generation: 1,
    displayedPoints: baseline.immutable_deployment.root.display_point_count,
    displayModes: baseline.presentation_policy.display_modes,
    projections: baseline.presentation_policy.projections,
    cancellationRetainedViewer: true,
    recoverableFailureCode: "offline",
    retryRetainedViewer: true,
    retrySucceeded: true,
    recreationFailureCode: "cancelled",
    recreationRequired: true,
    recreationSucceeded: true,
    provisionalAuthority: baseline.presentation_policy.provisional_authority,
    exactAuthority: baseline.presentation_policy.exact_authority,
    disposed: true,
    packedRuntime: baseline.quickstart.packed_runtime.record,
  });
}

async function verifyDigestRecord(record) {
  const bytes = await readFile(resolvePath(record.path));
  assert.equal(bytes.byteLength, record.byte_length, `${record.path} byte length drifted`);
  assert.equal(sha256(bytes), record.sha256, `${record.path} SHA-256 drifted`);
}

function resolvePath(relativePath) {
  return path.join(repositoryRoot, relativePath);
}

async function readText(relativePath) {
  return readFile(resolvePath(relativePath), "utf8");
}

async function readJson(relativePath) {
  return JSON.parse(await readText(relativePath));
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

if (process.argv[1] && pathToFileURL(process.argv[1]).href === import.meta.url) {
  const baseline = JSON.parse(await readFile(baselineUrl, "utf8"));
  await verifyBrowserIntegrationBaseline(baseline);
  console.log("browser integration baseline passed");
}
