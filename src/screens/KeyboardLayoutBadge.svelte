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

  $effect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key.length !== 1 || event.ctrlKey || event.metaKey) refresh();
    };
    refresh();
    window.addEventListener("focus", refresh);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("focus", refresh);
      window.removeEventListener("keydown", onKey);
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
