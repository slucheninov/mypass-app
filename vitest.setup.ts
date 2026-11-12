import { clearMocks } from "@tauri-apps/api/mocks";
import { afterEach } from "vitest";

// Testing Library unmounts after each test (svelteTesting() in vite.config.ts); the IPC mock is
// process-global, so reset it here and a test never sees the previous test's fake backend.
afterEach(() => {
  clearMocks();
});
