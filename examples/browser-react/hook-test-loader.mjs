import { registerHooks } from "node:module";

const VIEWER_TEST_DOUBLE_URL = "punctra-test:viewer";

registerHooks({
  resolve(specifier, context, nextResolve) {
    if (
      specifier === "@punctra/viewer"
      && context.parentURL?.includes("/node_modules/@punctra/react/")
    ) {
      return { shortCircuit: true, url: VIEWER_TEST_DOUBLE_URL };
    }
    return nextResolve(specifier, context);
  },
  load(url, context, nextLoad) {
    if (url === VIEWER_TEST_DOUBLE_URL) {
      return {
        format: "module",
        shortCircuit: true,
        source: `
          export function createViewer(options) {
            return globalThis.__PUNCTRA_TEST_CREATE_VIEWER__(options);
          }
        `,
      };
    }
    return nextLoad(url, context);
  },
});
