import { loadFootprintCorpus } from "./footprint-corpus.js";
import {
  FOOTPRINT_EXPORT_ARCHIVE_FILENAME,
  exportFootprintArchiveToLocalServer,
  footprintArchiveTransportFromUrl,
} from "./footprint-export.js";
import { runPointFootprintQualification } from "./footprint-qualification.js";
import { loadVisualCorpus } from "./visual-corpus.js";
import { createVisualValidator, errorMessage } from "./visual-validation.js";

const FOOTPRINT_CORPUS_URL = new URL("./fixtures/footprint-v1/corpus.json", import.meta.url);
const { requireCondition } = createVisualValidator("Point-footprint runner failed");

const canvas = document.querySelector("#footprint-canvas");
const runButton = document.querySelector("#run-footprint");
const modeSelect = document.querySelector("#footprint-mode");
const sessionInput = document.querySelector("#footprint-session");
const statusOutput = document.querySelector("#footprint-status");
const progressOutput = document.querySelector("#footprint-progress");
const progressCount = document.querySelector("#footprint-progress-count");
const evidenceOutput = document.querySelector("#footprint-evidence");
const artifactOutput = document.querySelector("#footprint-artifacts");
const transportOutput = document.querySelector("#footprint-transport-status");
const downloadArchiveButton = document.querySelector("#download-footprint-archive");
const requestedOutput = document.querySelector("#requested-footprint");
const selectedOutput = document.querySelector("#resolved-footprint");
const displaySizeOutput = document.querySelector("#display-diameter");
const pickSizeOutput = document.querySelector("#pick-diameter");
const transientOutput = document.querySelector("#transient-bytes");

let loadedContext;
let activeRun = false;
let latestArchive;
let latestEvidence;

async function initializePage() {
  try {
    const footprint = await loadFootprintCorpus(FOOTPRINT_CORPUS_URL);
    const visualUrl = new URL(footprint.corpus.predecessor.corpus.path, footprint.url);
    const visual = await loadVisualCorpus(visualUrl);
    loadedContext = { footprint, visual };
    buildProgress(footprint.corpus.canonical_trials);
    updateState("ready", "Ready. Choose record or verify, then start the bounded attended run.");
    runButton.disabled = false;
    configureTransportLabel();
  } catch (error) {
    updateState("failed", errorMessage(error));
    evidenceOutput.textContent = JSON.stringify(errorRecord(error), null, 2);
  }
}

async function startRun(options = {}, activation) {
  requireCondition(!activeRun, "a qualification run is already active");
  requireCondition(loadedContext !== undefined, "closed corpora are not loaded");
  const activationFacts = validateTrustedActivation(activation);
  const mode = validateMode(options.mode ?? modeSelect.value);
  const sessionLabel = validateSessionLabel(options.sessionLabel ?? sessionInput.value);
  activeRun = true;
  latestArchive = undefined;
  latestEvidence = undefined;
  runButton.disabled = true;
  modeSelect.disabled = true;
  sessionInput.disabled = true;
  downloadArchiveButton.disabled = true;
  artifactOutput.replaceChildren();
  resetProgress();
  updateState("running", "Binding implementation, verifier, runtime, and predecessor artifacts…");

  try {
    const result = await runPointFootprintQualification({
      mode,
      sessionLabel,
      activationFacts,
      inputs: loadedContext,
      canvas,
      observer: {
        artifactAdded: appendArtifactDownload,
        diagnostics: updateReadouts,
        progress: (completed, total) => {
          progressCount.textContent = `${completed} / ${total}`;
        },
        state: updateState,
        trial: markProgress,
      },
      isPageVisible: () => document.visibilityState === "visible",
      browser: {
        userAgent: navigator.userAgent,
        platform: navigator.platform || "unreported browser platform",
      },
      publishArchive,
    });
    latestEvidence = result.evidence;
    latestArchive = result.archive;
    const recordMode = result.evidence === null;
    const displayedRecord = recordMode ? result.baseline : result.evidence;
    evidenceOutput.textContent = JSON.stringify(displayedRecord, null, 2);
    downloadArchiveButton.disabled = false;
    const state = recordMode
      ? "passed"
      : result.evidence.summary.passed ? "passed" : "failed";
    const message = recordMode
      ? "RECORD COMPLETE — baseline manifest and PNGs are ready for the pinning stage."
      : result.evidence.summary.passed
        ? `PASS — ${result.evidence.summary.canonical_trials}/9 canonical trials, focused DPR checks, and resource fallback are bound.`
        : `FAIL — ${result.evidence.summary.failures.join("; ")}`;
    updateState(state, message);
    if (result.transportReceipt !== null) {
      transportOutput.textContent = `Archive persisted: ${result.transportReceipt.path}`;
    }
    return structuredClone(displayedRecord);
  } catch (error) {
    const failure = errorRecord(error);
    evidenceOutput.textContent = JSON.stringify(failure, null, 2);
    updateState("failed", failure.message);
    throw error;
  } finally {
    resetCanvasProfile(loadedContext.footprint.corpus.canonical_profile);
    resetReadouts();
    activeRun = false;
    runButton.disabled = false;
    modeSelect.disabled = false;
    sessionInput.disabled = false;
  }
}

async function publishArchive(bytes, sha256) {
  const transport = footprintArchiveTransportFromUrl(window.location.href);
  if (transport === "same-origin-local-server") {
    return exportFootprintArchiveToLocalServer({
      archiveBytes: bytes,
      filename: FOOTPRINT_EXPORT_ARCHIVE_FILENAME,
      sha256,
      pageUrl: window.location.href,
    });
  }
  downloadBytes(bytes, FOOTPRINT_EXPORT_ARCHIVE_FILENAME, "application/x-tar");
  return null;
}

function downloadLatestArchive() {
  requireCondition(latestArchive !== undefined, "no point-footprint archive is staged");
  downloadBytes(latestArchive.bytes, FOOTPRINT_EXPORT_ARCHIVE_FILENAME, "application/x-tar");
}

function downloadBytes(bytes, filename, type) {
  const url = URL.createObjectURL(new Blob([bytes], { type }));
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 300_000);
}

function appendArtifactDownload({ path, bytes, metadata }) {
  const item = document.createElement("li");
  const link = document.createElement("a");
  const url = URL.createObjectURL(new Blob([bytes], { type: metadata.mime_type }));
  link.href = url;
  link.download = path.split("/").at(-1);
  link.textContent = path;
  item.append(link);
  artifactOutput.append(item);
}

function updateReadouts(diagnostics) {
  const facts = diagnostics.point_footprint;
  requestedOutput.textContent = facts.requested;
  selectedOutput.textContent = facts.selected;
  displaySizeOutput.textContent = `${facts.display_size_physical_pixels.toFixed(3)} px`;
  pickSizeOutput.textContent = `${facts.nominal_pick_size_physical_pixels.toFixed(0)} px`;
  transientOutput.textContent = `${diagnostics.frame?.transient_texture_bytes?.toLocaleString() ?? "—"} B`;
}

function resetReadouts() {
  requestedOutput.textContent = "—";
  selectedOutput.textContent = "—";
  displaySizeOutput.textContent = "—";
  pickSizeOutput.textContent = "—";
  transientOutput.textContent = "—";
}

function resetCanvasProfile(profile) {
  canvas.style.width = `${profile.css_width}px`;
  canvas.style.height = `${profile.css_height}px`;
  canvas.width = profile.physical_width;
  canvas.height = profile.physical_height;
}

function buildProgress(trials) {
  progressOutput.replaceChildren();
  for (const trial of trials) {
    const item = document.createElement("li");
    item.dataset.trialId = trial.id;
    item.dataset.state = "pending";
    item.textContent = trial.id;
    progressOutput.append(item);
  }
  progressCount.textContent = `0 / ${trials.length}`;
}

function resetProgress() {
  for (const item of progressOutput.children) {
    item.dataset.state = "pending";
    item.textContent = item.dataset.trialId;
  }
  progressCount.textContent = `0 / ${progressOutput.children.length}`;
}

function markProgress(trialId, state, detail) {
  const item = [...progressOutput.children].find((candidate) => (
    candidate.dataset.trialId === trialId
  ));
  requireCondition(item !== undefined, `progress item ${trialId} is absent`);
  item.dataset.state = state;
  item.textContent = `${trialId} — ${detail}`;
}

function updateState(state, message) {
  document.body.dataset.footprintRunner = state;
  statusOutput.textContent = message;
}

function configureTransportLabel() {
  const transport = footprintArchiveTransportFromUrl(window.location.href);
  transportOutput.textContent = transport === "same-origin-local-server"
    ? "Opt-in same-origin local TAR export is active."
    : "Standard attended browser TAR download is active.";
}

function validateMode(mode) {
  requireCondition(mode === "record" || mode === "verify", "mode must be record or verify");
  return mode;
}

function validateSessionLabel(value) {
  requireCondition(
    typeof value === "string" && /^[A-Za-z0-9._-]{1,64}$/.test(value),
    "session label is invalid",
  );
  return value;
}

function validateTrustedActivation(event) {
  requireCondition(event?.isTrusted === true, "attended run requires a trusted click");
  requireCondition(
    navigator.userActivation?.isActive === true,
    "attended run requires active browser user activation",
  );
  requireCondition(document.visibilityState === "visible",
    "attended run requires a visible qualification page");
  return {
    trusted_user_activation: true,
    browser_user_activation_active: true,
    control_id: runButton.id,
    event_type: event.type,
  };
}

function errorRecord(error) {
  return {
    schema: "punctra-browser-point-footprint-runner-error-v1",
    name: error?.name ?? "Error",
    message: errorMessage(error),
  };
}

runButton.addEventListener("click", (event) => {
  startRun({}, event).catch(() => {});
});
downloadArchiveButton.addEventListener("click", downloadLatestArchive);

window.__punctraFootprintRunner = Object.freeze({
  getState: () => document.body.dataset.footprintRunner,
  getEvidence: () => latestEvidence === undefined ? null : structuredClone(latestEvidence),
});

initializePage();
