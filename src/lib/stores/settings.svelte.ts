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
 * so a locked window still follows the appearance and language. `SettingsChanged` carries only the
 * public part: an unlocked window re-reads the full settings when it arrives.
 *
 * Every answer is tagged with the lock epoch it was asked in: one that arrives after a lock (or a
 * lock and a new unlock) is dropped, and the public part is read again when the window leaves the
 * unlocked state, so an optimistic theme or language that was never confirmed does not stay.
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
      if (next !== "unlocked") this.#drop();
      void this.load().catch(() => {});
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
    const epoch = session.lockEpoch;
    if (session.status.state === "unlocked") {
      const next = await api.settingsGet();
      // A lock that landed meanwhile has the last word: nothing read before it is kept.
      if (seq !== this.#seq || !this.#current(epoch)) return;
      this.#adopt(next);
    } else {
      const next = await api.settingsGetPublic();
      if (seq !== this.#seq || epoch !== session.lockEpoch) return;
      this.#adoptPublic(next);
    }
  }

  /** Sends a patch. Theme and language show at once; a failed update puts the old ones back. */
  async update(patch: SettingsPatch_Deserialize): Promise<void> {
    const before = this.value;
    const epoch = session.lockEpoch;
    if (before && (patch.theme || patch.language)) {
      this.#applyAppearance(patch.theme ?? before.theme, patch.language ?? before.language);
    }
    try {
      const next = await api.settingsUpdate(patch);
      // An answer from before a lock is not this session's: the lock already re-read the public part.
      if (!this.#current(epoch)) return;
      this.#seq++;
      this.#adopt(next);
    } catch (e) {
      // Revert only when nothing newer arrived meanwhile; after a lock the public re-read decides.
      if (before && this.value === before && this.#current(epoch)) this.#applyAppearance(before.theme, before.language);
      throw e;
    }
  }

  #current(epoch: number): boolean {
    return session.status.state === "unlocked" && session.lockEpoch === epoch;
  }

  /** A broadcast carries the public part; an unlocked window re-reads the full settings. */
  #adoptEvent(next: PublicSettings): void {
    this.#seq++;
    this.#adoptPublic(next);
    if (session.status.state === "unlocked") void this.load().catch(() => {});
  }

  #adopt(next: Settings_Serialize): void {
    this.value = next;
    this.#adoptPublic(publicPart(next));
  }

  #adoptPublic(next: PublicSettings): void {
    this.public = next;
    this.#applyAppearance(next.theme, next.language);
  }

  /** Forgets the full settings (lock, vault closed); the caller re-reads the public part. */
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
