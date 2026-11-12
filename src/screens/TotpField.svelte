<script lang="ts">
  // The one-time code row: the code (meant to be shown; the secret stays in Rust), the ring and a
  // refetch at every period boundary. The countdown follows the clock, not a counter, so a
  // throttled timer cannot leave a stale code on screen past its boundary.
  import { onDestroy, onMount } from "svelte";
  import { api } from "../lib/api";
  import type { FieldRef } from "../lib/bindings";
  import { t } from "../lib/i18n/index.svelte";
  import { lockGuard } from "../lib/lock-guard";
  import { toasts } from "../lib/stores/toasts.svelte";
  import DetailField from "./DetailField.svelte";
  import TotpRing from "./TotpRing.svelte";

  let { entryId, onlargetype }: { entryId: string; onlargetype: (field: FieldRef) => void } = $props();

  let code = $state<string | null>(null);
  let remaining = $state(0);
  let period = $state(30);
  let boundary = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let generation = 0;

  /** `123 456` for six digits, `1234 5678` for eight; anything else as it comes. */
  function grouped(value: string): string {
    if (value.length === 6) return `${value.slice(0, 3)} ${value.slice(3)}`;
    if (value.length === 8) return `${value.slice(0, 4)} ${value.slice(4)}`;
    return value;
  }

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
      // At least one second, so a backend answering "0 left" cannot spin the refetch.
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

<DetailField label={t("One-time code")} {entryId} field={{ kind: "totp" }} {onlargetype}>
  {#if code !== null}
    <span class="totp">
      <TotpRing {remaining} {period} />
      <span class="code">{grouped(code)}</span>
    </span>
  {/if}
</DetailField>

<style>
  .totp {
    display: inline-flex;
    align-items: center;
    gap: 10px;
  }
  .code {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 18px;
    letter-spacing: 0.04em;
  }
</style>
