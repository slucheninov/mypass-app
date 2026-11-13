import { api } from "./api";

export type Os = "macos" | "windows" | "linux";

/** Capability state from `app_info`; the defaults keep capability-gated UI hidden until loaded. */
export const platform: { os: Os | null; deviceUnlockAvailable: boolean } = $state({
  os: null,
  deviceUnlockAvailable: false,
});

function narrowOs(os: string): Os | null {
  return os === "macos" || os === "windows" || os === "linux" ? os : null;
}

/** Fills `platform` from `app_info`; a failure leaves the safe defaults. */
export async function loadPlatform(): Promise<void> {
  try {
    const info = await api.appInfo();
    platform.os = narrowOs(info.os);
    platform.deviceUnlockAvailable = info.deviceUnlockAvailable;
  } catch {
    // Keep the defaults.
  }
}
