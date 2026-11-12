<script lang="ts">
  // The current keyboard layout next to the password field, so a wrong layout is noticed before a
  // failed unlock. The short code comes from the backend and is shown as is. It is read on mount,
  // on window focus and on keys that can switch the layout (anything but a printable character),
  // at most once per 500 ms; `null` (unsupported platform) hides the badge.
  import { api } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import { createThrottle } from "../lib/throttle";
  import Icon from "../lib/ui/Icon.svelte";

  let layout = $state<string | null>(null);
  let latest = 0;
  const throttle = createThrottle(500);

  async function read(): Promise<void> {
    const mine = ++latest;
    let value: string | null = null;
    try {
      value = await api.platformKeyboardLayout();
    } catch {
      // Treated like an unsupported platform.
    }
    if (mine === latest) layout = value;
  }

  const refresh = () => void throttle.run(() => void read());

  // The OS switches the layout after the key event, so a read inside the handler would still see
  // the old one. A key that can switch it schedules one delayed read instead; it is not subject to
  // the throttle (that only limits the immediate reads), and only one is pending at a time.
  const SWITCH_DELAY_MS = 120;

  $effect(() => {
    let pending: ReturnType<typeof setTimeout> | null = null;
    const onKey = (event: KeyboardEvent) => {
      if (event.key.length === 1 && !event.ctrlKey && !event.metaKey) return;
      refresh();
      if (pending !== null) return;
      pending = setTimeout(() => {
        pending = null;
        void read();
      }, SWITCH_DELAY_MS);
    };
    refresh();
    window.addEventListener("focus", refresh);
    window.addEventListener("keydown", onKey);
    window.addEventListener("keyup", onKey);
    return () => {
      window.removeEventListener("focus", refresh);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("keyup", onKey);
      if (pending !== null) clearTimeout(pending);
    };
  });
</script>

<div class="region" aria-live="polite">
  {#if layout}
    <span class="badge" data-testid="keyboard-layout">
      <Icon name="keyboard" size={14} />
      <span aria-hidden="true">{layout}</span>
      <span class="sr-only">{t("Keyboard layout: %@", [layout])}</span>
    </span>
  {/if}
</div>

<style>
  .region {
    min-height: 20px;
  }
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--text-2);
    font-size: 12px;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
