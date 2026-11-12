<script lang="ts">
  // The tray's one-time code row: the code (meant to be shown; the secret stays in Rust), the ring
  // and a refetch at every period boundary. The countdown follows the clock, not a counter.
  import { onDestroy, onMount } from "svelte";
  import { api } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import { lockGuard } from "../lib/lock-guard";
  import { toasts } from "../lib/stores/toasts.svelte";
  import TotpRing from "../screens/TotpRing.svelte";
  import TrayRow from "./TrayRow.svelte";

  let { entryId }: { entryId: string } = $props();

  let code = $state<string | null>(null);
  let remaining = $state(0);
  let period = $state(30);
  let boundary = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let generation = 0;

  const grouped = (value: string): string =>
    value.length === 6 ? `${value.slice(0, 3)} ${value.slice(3)}` : value.length === 8 ? `${value.slice(0, 4)} ${value.slice(4)}` : value;

  function stopTimer() {
    if (timer !== null) clearTimeout(timer);
    timer = null;
  }

  function tick() {
    stopTimer();
    const left = boundary - Date.now();
    if (left <= 0) {
      void fetchCode();
      return;
    }
    remaining = Math.ceil(left / 1000);
    timer = setTimeout(tick, Math.min(1000, left));
  }

  async function fetchCode() {
    const mine = ++generation;
    const unchanged = lockGuard();
    try {
      const next = await api.entryTotp(entryId);
      if (mine !== generation || !unchanged()) return;
      code = next.code;
      period = next.period;
      boundary = Date.now() + Math.max(1, next.secondsRemaining) * 1000;
      tick();
    } catch (e) {
      if (mine === generation && unchanged()) toasts.reportError(e);
    }
  }

  onMount(() => void fetchCode());
  onDestroy(() => {
    generation++;
    stopTimer();
  });
</script>

<TrayRow label={t("One-time code")} {entryId} field={{ kind: "totp" }}>
  {#if code !== null}
    <span class="totp">
      <TotpRing {remaining} {period} />
      <span class="code">{grouped(code)}</span>
    </span>
  {/if}
</TrayRow>

<style>
  .totp {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .code {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 16px;
    letter-spacing: 0.05em;
  }
</style>
