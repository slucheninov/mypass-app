import { api } from "../api";
import { events, type Language, type SettingsPatch_Deserialize, type Settings_Serialize, type Theme } from "../bindings";
import { setLanguage } from "../i18n/index.svelte";

/** Applies the theme setting: `system` leaves the attribute off so `prefers-color-scheme` decides. */
export function applyTheme(theme: "system" | "light" | "dark"): void {
  const root = document.documentElement;
  if (theme === "system") delete root.dataset.theme;
  else root.dataset.theme = theme;
}

function applyAppearance(theme: Theme, language: Language): void {
  applyTheme(theme);
  setLanguage(language);
}

class SettingsStore {
  value = $state<Settings_Serialize | null>(null);
  #unlisten: (() => void) | null = null;

  /** Listens for `SettingsChanged`, then loads. Call once per window. */
  async start(): Promise<void> {
    this.#unlisten ??= await events.settingsChanged.listen((e) => this.#adopt(e.payload));
    await this.load();
  }

  stop(): void {
    this.#unlisten?.();
    this.#unlisten = null;
    this.value = null;
  }

  async load(): Promise<void> {
    this.#adopt(await api.settingsGet());
  }

  /** Sends a patch. Theme and language show at once; a failed update puts the old ones back. */
  async update(patch: SettingsPatch_Deserialize): Promise<void> {
    const before = this.value;
    if (before && (patch.theme || patch.language)) {
      applyAppearance(patch.theme ?? before.theme, patch.language ?? before.language);
    }
    try {
      this.#adopt(await api.settingsUpdate(patch));
    } catch (e) {
      if (before) applyAppearance(before.theme, before.language);
      throw e;
    }
  }

  #adopt(next: Settings_Serialize): void {
    this.value = next;
    applyAppearance(next.theme, next.language);
  }
}

export const settings = new SettingsStore();
