import { defineConfig } from "vite";

export default defineConfig({
  optimizeDeps: {
    exclude: ["@punctra/viewer"],
  },
  worker: {
    format: "es",
  },
  build: {
    assetsInlineLimit: 0,
    manifest: true,
  },
});
