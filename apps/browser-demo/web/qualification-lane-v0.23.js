// v0.23 uses the same exact local hardware/browser lane as v0.22.
// Fresh v0.23 observations are required; these declarations confer no qualification.
import { QUALIFICATION_LANE, QUALIFICATION_RUNTIME_LANE as predecessorRuntime } from "./qualification-lane-v0.22.js";

export { QUALIFICATION_LANE };
export const QUALIFICATION_RUNTIME_LANE = Object.freeze({
  ...predecessorRuntime,
  host: Object.freeze({ ...predecessorRuntime.host,
    package: Object.freeze({ ...predecessorRuntime.host.package, version: "0.23.0-alpha.1" }) }),
});
