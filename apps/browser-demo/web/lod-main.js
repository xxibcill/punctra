import { loadLodCorpus } from "./lod-corpus.js";
import { exportLodArchiveToLocalServer, LOD_EXPORT_ARCHIVE_FILENAME } from "./lod-export.js";
import { runLodQualification } from "./lod-qualification.js";
import { loadVisualCorpus } from "./visual-corpus.js";

const canvas = document.querySelector("#lod-canvas");
const button = document.querySelector("#run-lod");
const mode = document.querySelector("#lod-mode");
const session = document.querySelector("#lod-session");
const status = document.querySelector("#lod-status");
const evidence = document.querySelector("#lod-evidence");
const transport = document.querySelector("#lod-transport");
let inputs;

function setState(value, message) {
  document.body.dataset.lodRunner = value;
  status.textContent = message;
}

async function initialize() {
  try {
    const [lod, visual] = await Promise.all([
      loadLodCorpus(new URL("./fixtures/lod-v1/corpus.json", import.meta.url)),
      loadVisualCorpus(new URL("./fixtures/visual-v1/corpus.json", import.meta.url)),
    ]);
    inputs = { lod, visual };
    button.disabled = false;
    setState("ready", "Ready. Choose record or verify and start the attended run.");
  } catch (error) { setState("failed", String(error)); }
}

button.addEventListener("click", async (event) => {
  if (!event.isTrusted || document.visibilityState !== "visible" || !inputs) return;
  const label = session.value.trim();
  if (!/^[A-Za-z0-9._-]{1,64}$/.test(label)) {
    setState("failed", "Use a 1–64 character anonymous session label.");
    return;
  }
  button.disabled = true;
  mode.disabled = true;
  session.disabled = true;
  try {
    setState("running", "Binding the clean implementation and actual Wasm runtime…");
    const result = await runLodQualification({ mode: mode.value, sessionLabel: label,
      activation: { trusted_user_activation: event.isTrusted, page_visibility: document.visibilityState,
        independent_human: false, agent_operated: true },
      inputs, canvas, state: (message) => setState("running", message),
      publishArchive: (bytes, sha256) => exportLodArchiveToLocalServer({ archiveBytes: bytes,
        filename: LOD_EXPORT_ARCHIVE_FILENAME, sha256, pageUrl: location.href }),
    });
    evidence.textContent = JSON.stringify({ pins: result.record.pins.implementation.commit,
      mode: result.record.mode, summary: result.record.summary, resources: result.record.resources,
      external_evidence: result.record.external_evidence }, null, 2);
    transport.textContent = `Archive persisted: ${result.receipt.path}`;
    setState("passed", `${mode.value === "record" ? "RECORD COMPLETE" : "VERIFY PASS"} — ${result.record.summary.transition_frames} transition frames and 27 canonical recreations passed.`);
  } catch (error) {
    evidence.textContent = String(error?.stack ?? error);
    setState("failed", String(error));
  } finally {
    button.disabled = false;
    mode.disabled = false;
    session.disabled = false;
  }
});

await initialize();
