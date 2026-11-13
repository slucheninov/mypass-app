import { createRawSnippet } from "svelte";

/** A text-only snippet for tests that pass `children`, `footer`, `actions` or `trigger`. */
export function textSnippet(text: string) {
  return createRawSnippet(() => ({ render: () => `<span>${text}</span>` }));
}
