import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

// Runes mode only: no Svelte 4 stores, no `$:`, no `export let` (pinned by scaffold.test.ts).
export default {
  preprocess: vitePreprocess(),
  compilerOptions: { runes: true },
};
