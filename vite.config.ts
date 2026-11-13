import { svelte } from "@sveltejs/vite-plugin-svelte";
import { svelteTesting } from "@testing-library/svelte/vite";
import { defineConfig } from "vitest/config";

// One config for the dev server, the production build and the unit tests. The dev server port is
// the `devUrl` of src-tauri/tauri.conf.json; `strictPort` makes a clash fail instead of drifting.
export default defineConfig({
  plugins: [svelte(), svelteTesting()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // The Vite root is the Cargo workspace root: keep the watcher off the Rust trees and build output.
    watch: { ignored: ["**/target/**", "**/src-tauri/**", "**/crates/**"] },
  },
  build: {
    // The baseline engines are WKWebView on macOS 14 (Safari 17) and WebView2; Safari 16 is the
    // floor the output is transpiled for.
    target: "safari16",
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts"],
    setupFiles: ["./vitest.setup.ts"],
  },
});
