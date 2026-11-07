import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Tauri expects a fixed port and reads TAURI_ENV_* at build time.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    outDir: "dist",
    target: ["es2022", "safari16"],
    sourcemap: false,
  },
  test: {
    environment: "node",
    include: ["src/__checks__/**/*.test.ts"],
  },
});
