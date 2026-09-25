import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// `npm run dev` forwards API calls to a node on its default address.
// `npm run build` writes dist/, which the node serves on its own origin.
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: { "/api": "http://127.0.0.1:4780" },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
  test: {
    environment: "node",
  },
});
