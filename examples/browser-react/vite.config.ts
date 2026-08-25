import { defineConfig } from "vite";

export default defineConfig({
  worker: {
    format: "es",
  },
  build: {
    assetsInlineLimit: 0,
    manifest: true,
  },
});
