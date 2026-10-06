import {
  QUALIFICATION_LANE as predecessorLane,
  QUALIFICATION_RUNTIME_LANE as predecessorRuntime,
} from "./qualification-lane.js";

// The predecessor lane remains immutable evidence for v0.21.
const browser = {
  ...predecessorLane.browser,
  user_agent_version: "154.0.0.0",
  user_agent: predecessorLane.browser.user_agent.replace("Chrome/151.0.0.0", "Chrome/154.0.0.0"),
};
const id = "codex-iab-chromium-154-macos-26-apple-m5-pro";

export const QUALIFICATION_LANE = deepFreeze({
  ...structuredClone(predecessorLane),
  id,
  browser,
});

export const QUALIFICATION_RUNTIME_LANE = deepFreeze({
  ...structuredClone(predecessorRuntime),
  id,
  browser: { ...predecessorRuntime.browser, userAgent: browser.user_agent },
  capabilities: { ...predecessorRuntime.capabilities, browser_user_agent: browser.user_agent },
});

function deepFreeze(value) {
  if (value && typeof value === "object" && !Object.isFrozen(value)) {
    for (const nested of Object.values(value)) deepFreeze(nested);
    Object.freeze(value);
  }
  return value;
}
