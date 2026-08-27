import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { verifyBrowserIntegrationBaseline } from "./verify-browser-integration-baseline.mjs";

const baseline = JSON.parse(await readFile(
  new URL("../docs/releases/v0.20-browser-baseline.json", import.meta.url),
  "utf8",
));

test("the checked-in browser integration baseline matches its source inputs", async () => {
  assert.equal(await verifyBrowserIntegrationBaseline(baseline), true);
});

test("an accidental exact-decoder export fails the supported surface", async () => {
  const tampered = structuredClone(baseline);
  tampered.packages.viewer.supported_entry_points["./exact-query"].push("decodeLasLayout");
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

test("the external-evidence boundary cannot be promoted by editing the record", async () => {
  const tampered = structuredClone(baseline);
  tampered.external_evidence.independent_adopter = true;
  await assert.rejects(() => verifyBrowserIntegrationBaseline(tampered));
});
