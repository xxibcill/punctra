import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  verifyBrowserIntegrationBaseline,
  verifyQuickstartEvidence,
} from "./verify-browser-integration-baseline.mjs";

const baseline = JSON.parse(await readFile(
  new URL("../docs/releases/v0.20-browser-baseline.json", import.meta.url),
  "utf8",
));
const quickstartEvidence = JSON.parse(await readFile(
  new URL("../docs/releases/v0.20-browser-quickstart.json", import.meta.url),
  "utf8",
));
const qualificationMatrix = JSON.parse(await readFile(
  new URL("../docs/releases/v0.20-browser-matrix.json", import.meta.url),
  "utf8",
));
const operationalReleaseSources = [
  "build-browser-sdk.sh",
  "generate-browser-sdk-reference.mjs",
  "serve-browser-demo.py",
  "verify-browser-integration-baseline.mjs",
  "verify-browser-qualification.mjs",
  "verify-browser-sdk.mjs",
];

test("the checked-in browser integration baseline matches its source inputs", async () => {
  assert.equal(await verifyBrowserIntegrationBaseline(baseline), true);
});

test("an accidental exact-decoder export fails the supported surface", async () => {
  const tampered = structuredClone(baseline);
  tampered.packages.viewer.supported_entry_points["./exact-query"].push("decodeLasLayout");
  await assert.rejects(() => verifyBrowserIntegrationBaseline(tampered));
});

test("declaration drift fails even when required names remain present", async () => {
  const tampered = structuredClone(baseline);
  tampered.packages.viewer.declaration_digests["."].sha256 = "00".repeat(32);
  await assert.rejects(
    () => verifyBrowserIntegrationBaseline(tampered),
    /apps\/browser-demo\/web\/sdk\.d\.ts SHA-256 drifted/,
  );
});

test("export target drift fails the supported package surface", async () => {
  const tampered = structuredClone(baseline);
  tampered.packages.viewer.export_targets["./input"].import = "./sdk.js";
  await assert.rejects(() => verifyBrowserIntegrationBaseline(tampered));
});

test("fixture digest drift fails even when semantic facts are unchanged", async () => {
  const tampered = structuredClone(baseline);
  tampered.immutable_deployment.source.sha256 = "00".repeat(32);
  await assert.rejects(
    () => verifyBrowserIntegrationBaseline(tampered),
    /representative\.las SHA-256 drifted/,
  );
});

test("qualification observations are byte-bound even when values remain in limits", async () => {
  const tampered = structuredClone(baseline);
  tampered.qualification.matrix_digest.sha256 = "00".repeat(32);
  await assert.rejects(
    () => verifyBrowserIntegrationBaseline(tampered),
    /v0\.20-browser-matrix\.json SHA-256 drifted/,
  );
});

test("the integration baseline binds the qualification observations", async () => {
  const tampered = structuredClone(qualificationMatrix);
  tampered.qualified_entries[0].observations.render.drawn_points = 1;
  await assert.rejects(
    () => verifyBrowserIntegrationBaseline(baseline, tampered),
    /observed render output must match the qualified workload/,
  );
});

test("generated-scene facts come from the executable scene preparation", async () => {
  const tampered = structuredClone(baseline);
  tampered.generated_scene.point_count = 1;
  await assert.rejects(
    () => verifyBrowserIntegrationBaseline(tampered),
    /baseline generated-scene facts must match PreparedScene output/,
  );
});

test("quickstart evidence is byte-bound and semantically verified", async () => {
  const digestTampered = structuredClone(baseline);
  digestTampered.quickstart.evidence.sha256 = "00".repeat(32);
  await assert.rejects(
    () => verifyBrowserIntegrationBaseline(digestTampered),
    /v0\.20-browser-quickstart\.json SHA-256 drifted/,
  );

  const semanticTampered = structuredClone(quickstartEvidence);
  semanticTampered.acceptance.recreationSucceeded = false;
  assert.throws(() => verifyQuickstartEvidence(semanticTampered, baseline));
});

test("quickstart evidence requires packed runtime provenance", () => {
  const tampered = structuredClone(quickstartEvidence);
  delete tampered.acceptance.packedRuntime;
  assert.throws(
    () => verifyQuickstartEvidence(tampered, baseline),
    /packedRuntime/,
  );
});

test("the external-evidence boundary cannot be promoted by editing the record", async () => {
  const tampered = structuredClone(baseline);
  tampered.external_evidence.independent_adopter = true;
  await assert.rejects(() => verifyBrowserIntegrationBaseline(tampered));
});

test("operational tooling derives the release from authoritative package manifests", async () => {
  for (const relativePath of operationalReleaseSources) {
    const source = await readFile(new URL(relativePath, import.meta.url), "utf8");
    assert.doesNotMatch(source, /0\.20\.0-alpha\.1/, relativePath);
  }
});
