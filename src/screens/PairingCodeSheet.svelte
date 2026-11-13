<script lang="ts">
  // The pairing code of the browser extension, shown while the backend waits for the browser to
  // type it (main window only). The code is shown, never announced: no live region reads it out.
  // It lives in `browser.pairing` alone and leaves the DOM with the sheet. The countdown runs from
  // the backend's own deadline; at zero the sheet asks the backend what is active now, so a sheet
  // never outlives its code (the clear event arrives about a second after the deadline).
  import { onMount } from "svelte";
  import { t } from "../lib/i18n/index.svelte";
  import { browser } from "../lib/stores/browser.svelte";
  import Button from "../lib/ui/Button.svelte";
  import Modal from "../lib/ui/Modal.svelte";

  onMount(() => browser.watchPairing());

  let now = $state(Date.now());
  const challenge = $derived(browser.pairing);
  const expiresAt = $derived(challenge?.expiresAt ?? null);

  $effect(() => {
    if (expiresAt === null) return;
    now = Date.now();
    const timer = setInterval(() => (now = Date.now()), 500);
    return () => clearInterval(timer);
  });

  const remaining = $derived(expiresAt === null ? null : Math.max(0, Math.ceil((expiresAt - now) / 1000)));

  $effect(() => {
    if (remaining === 0) void browser.requeryPairing();
  });

  // "123456" reads as "123 456".
  const grouped = $derived((challenge?.code ?? "").replace(/(.{3})(?=.)/g, "$1 "));
</script>

{#if challenge}
  <Modal open={true} title={t("Pairing code")} onclose={() => void browser.cancelPairing()}>
    <p class="code" data-testid="pairing-code">{grouped}</p>
    <p class="browser">{challenge.browser}</p>
    <p class="hint">
      {#if remaining === null}
        {t("Enter this code in the MyPass browser extension.")}
      {:else}
        {t("Enter this code in the MyPass extension. It expires in %lld seconds.", remaining)}
      {/if}
    </p>
    {#snippet footer()}
      <Button onclick={() => void browser.cancelPairing()}>{t("Cancel")}</Button>
    {/snippet}
  </Modal>
{/if}

<style>
  .code {
    margin: 4px 0 8px;
    font-family: var(--mono, ui-monospace, monospace);
    font-size: 40px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-align: center;
    user-select: text;
  }
  .browser {
    margin: 0 0 8px;
    text-align: center;
    font-weight: 600;
  }
  .hint {
    margin: 0;
    text-align: center;
    color: var(--text-2);
  }
</style>
