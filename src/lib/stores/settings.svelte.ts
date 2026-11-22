import { api } from "../api";
import { events, type Language, type PublicSettings, type SettingsPatch_Deserialize, type Settings_Serialize, type Theme } from "../bindings";
import { setLanguage } from "../i18n/index.svelte";
import { session } from "./session.svelte";

/** Applies the theme setting: `system` leaves the attribute off so `prefers-color-scheme` decides. */
export function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  if (theme === "system") delete root.dataset.theme;
  else root.dataset.theme = theme;
}

function publicPart(full: Settings_Serialize): PublicSettings {
  const { language, theme, shortcutLock, shortcutShow, updateCheckEnabled } = full;
  return { language, theme, shortcutLock, shortcutShow, updateCheckEnabled };
}

/**
 * The settings as one window sees them. The full settings (`value`) are read and kept only while
 * the vault is unlocked, as the backend serves them only then; a lock drops them. `public` (theme,
 * language, shortcut hints, the update flag) is what every window renders in any session state,
 * so a locked window still follows the appearance and language.
 */
class SettingsStore {
  value = $state<Settings_Serialize | null>(null);
  public = $state<PublicSettings | null>(null);
  #unlisten: (() => void) | null = null;
  #offSession: (() => void) | null = null;
  #followTheme = true;
  #subscribed: Promise<void> | null = null;
  /** Bumped by every read and event, so a slower answer never overwrites a newer one. */
  #seq = 0;

  /** Listens for `SettingsChanged` and the session, then loads. Call once per window. */
  async start({ followTheme = true } = {}): Promise<void> {
    this.#followTheme = followTheme;
    // Registered before the first await, so the session's first answer is never missed.
    this.#offSession?.();
    this.#offSession = session.onChange((next) => {
      if (next === "unlocked") void this.load().catch(() => {});
      else this.#drop();
    });
    this.#subscribed ??= events.settingsChanged.listen((e) => this.#adoptEvent(e.payload)).then((off) => {
      this.#unlisten = off;
    });
    await this.#subscribed;
    await this.load();
  }

  stop(): void {
    this.#subscribed = null;
    this.#unlisten?.();
    this.#unlisten = null;
    this.#offSession?.();
    this.#offSession = null;
    this.#seq++;
    this.value = null;
    this.public = null;
  }

  /** The full settings while unlocked, otherwise the public part only. */
  async load(): Promise<void> {
    const seq = ++this.#seq;
    if (session.status.state === "unlocked") {
      const epoch = session.lockEpoch;
      const next = await api.settingsGet();
      // A lock that landed meanwhile has the last word: nothing read before it is kept.
      if (seq !== this.#seq || session.status.state !== "unlocked" || session.lockEpoch !== epoch) return;
      this.#adopt(next);
    } else {
      const next = await api.settingsGetPublic();
      if (seq !== this.#seq) return;
      this.#adoptPublic(next);
    }
  }

  /** Sends a patch. Theme and language show at once; a failed update puts the old ones back. */
  async update(patch: SettingsPatch_Deserialize): Promise<void> {
    const before = this.value;
    if (before && (patch.theme || patch.language)) {
      this.#applyAppearance(patch.theme ?? before.theme, patch.language ?? before.language);
    }
    try {
      const next = await api.settingsUpdate(patch);
      this.#seq++;
      if (session.status.state === "unlocked") this.#adopt(next);
      else this.#adoptPublic(publicPart(next));
    } catch (e) {
      // Revert only when nothing newer arrived meanwhile.
      if (before && this.value === before) this.#applyAppearance(before.theme, before.language);
      throw e;
    }
  }

  /** A broadcast carries the full settings; a window that is not unlocked keeps only the public part. */
  #adoptEvent(next: Settings_Serialize): void {
    this.#seq++;
    if (session.status.state === "unlocked") this.#adopt(next);
    else this.#adoptPublic(publicPart(next));
  }

  #adopt(next: Settings_Serialize): void {
    this.value = next;
    this.#adoptPublic(publicPart(next));
  }

  #adoptPublic(next: PublicSettings): void {
    this.public = next;
    this.#applyAppearance(next.theme, next.language);
  }

  /** Forgets the full settings (lock, vault closed); the appearance stays as it is. */
  #drop(): void {
    this.#seq++;
    this.value = null;
  }

  #applyAppearance(theme: Theme, language: Language): void {
    if (this.#followTheme) applyTheme(theme);
    setLanguage(language);
  }
}

export const settings = new SettingsStore();
