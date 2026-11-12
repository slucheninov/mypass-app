import { clearMocks } from "@tauri-apps/api/mocks";
import { afterEach } from "vitest";

// Testing Library unmounts after each test (svelteTesting() in vite.config.ts); the IPC mock is
// process-global, so reset it here and a test never sees the previous test's fake backend.
afterEach(() => {
  clearMocks();
});

// Node 26 ships its own `localStorage` global, which shadows jsdom's and is `undefined` without a
// backing file. The UI keeps layout preferences there (src/lib/prefs.ts), so tests get a plain
// in-memory Storage; a test that wants "storage unavailable" overrides it itself.
if (typeof window.localStorage === "undefined") {
  const data = new Map<string, string>();
  const storage = {
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => void data.set(key, String(value)),
    removeItem: (key: string) => void data.delete(key),
    clear: () => data.clear(),
    key: (index: number) => [...data.keys()][index] ?? null,
    get length() {
      return data.size;
    },
  } as Storage;
  Object.defineProperty(window, "localStorage", { value: storage, configurable: true });
}
