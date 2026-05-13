import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri convention: don't clear screen, listen on a fixed port, no minify in
// dev so the WKWebView devtools show readable code.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "esnext",
    minify: process.env.TAURI_DEBUG ? false : "esbuild",
    sourcemap: !!process.env.TAURI_DEBUG,
  },
});
