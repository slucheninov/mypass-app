<script lang="ts">
  // The value of one field at 48 px, for reading it out or typing it elsewhere. The value is
  // requested only after this component exists (the user chose "Show Large Type"), lives in one
  // `createRevealed()` and is gone with the component. It closes on Escape, a click outside, the
  // window hiding or minimising, a lock, and (the owner unmounts it) a selection change.
  import { onDestroy, onMount } from "svelte";
  import { api } from "../lib/api";
  import type { FieldRef } from "../lib/bindings";
  import { t } from "../lib/i18n/index.svelte";
  import { createRevealed, onWindowHidden } from "../lib/secret.svelte";
  import { session } from "../lib/stores/session.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
  import Button from "../lib/ui/Button.svelte";
  import Modal from "../lib/ui/Modal.svelte";

  let { entryId, field, onclose }: { entryId: string; field: FieldRef; onclose: () => void } = $props();

  const secret = createRevealed();
  const openedAtEpoch = session.lockEpoch;

  $effect(() => {
    if (session.lockEpoch !== openedAtEpoch) onclose();
  });

  onMount(() => {
    secret.reveal(() => api.entryReveal(entryId, field)).catch((e) => {
      toasts.reportError(e);
      onclose();
    });
    return onWindowHidden(onclose);
  });
  onDestroy(() => secret.hide());

  type Kind = "letter" | "digit" | "symbol";
  const kindOf = (ch: string): Kind => (/\p{N}/u.test(ch) ? "digit" : /\p{L}/u.test(ch) ? "letter" : "symbol");

  // Consecutive characters of one class share a span; the colours are decorative only.
  const runs = $derived.by(() => {
    const out: { kind: Kind; text: string }[] = [];
    for (const ch of secret.value ?? "") {
      const kind = kindOf(ch);
      const last = out[out.length - 1];
      if (last && last.kind === kind) last.text += ch;
      else out.push({ kind, text: ch });
    }
    return out;
  });
</script>

<Modal open={true} title={t("Large Type")} {onclose}>
  <div class="value" data-part="value" style="font-size: 48px; font-family: ui-monospace, SFMono-Regular, Menlo, monospace">
    {#each runs as run, i (i)}<span data-class={run.kind}>{run.text}</span>{/each}
  </div>
  {#snippet footer()}
    <Button onclick={onclose}>{t("Close")}</Button>
  {/snippet}
</Modal>

<style>
  .value {
    line-height: 1.25;
    overflow-wrap: anywhere;
    word-break: break-all;
    user-select: text;
    -webkit-user-select: text;
  }
  [data-class="digit"] {
    color: var(--accent);
  }
  [data-class="symbol"] {
    color: var(--danger);
  }
</style>
