<script lang="ts">
  // The notes of an entry: hidden behind "Reveal notes" until clicked. The text comes through
  // `entry_reveal` into a member of the entry's reveal group (so it excludes every other revealed
  // value), is rendered as sanitised markdown, and is dropped on the group's rules (selection
  // change, lock, window hide or minimise) and after 60 s of the window being blurred.
  import { onDestroy, untrack } from "svelte";
  import { api } from "../lib/api";
  import { t } from "../lib/i18n/index.svelte";
  import type { RevealGroup } from "../lib/secret.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";
  import Button from "../lib/ui/Button.svelte";
  import MarkdownView from "./MarkdownView.svelte";

  let { entryId, group }: { entryId: string; group: RevealGroup } = $props();

  const uid = $props.id();
  const BLUR_HIDE_MS = 60_000;
  const secret = untrack(() => group.create());
  let blurTimer: ReturnType<typeof setTimeout> | null = null;

  const toggle = () =>
    toasts.guard(async () => {
      if (secret.value !== null) secret.hide();
      else await secret.reveal(() => api.entryReveal(entryId, { kind: "notesBody" }));
    });

  function clearBlurTimer() {
    if (blurTimer !== null) clearTimeout(blurTimer);
    blurTimer = null;
  }
  function onblur() {
    clearBlurTimer();
    blurTimer = setTimeout(() => {
      blurTimer = null;
      secret.hide();
    }, BLUR_HIDE_MS);
  }
  onDestroy(() => {
    clearBlurTimer();
    secret.hide();
  });
</script>

<svelte:window {onblur} onfocus={clearBlurTimer} />

<section class="notes" aria-labelledby="notes-heading-{uid}">
  <div class="head">
    <h3 id="notes-heading-{uid}">{t("Protected notes")}</h3>
    <Button onclick={toggle}>{secret.value === null ? t("Reveal notes") : t("Hide notes")}</Button>
  </div>
  {#if secret.value !== null}
    <MarkdownView source={secret.value} />
  {/if}
</section>

<style>
  .notes {
    margin-top: 16px;
    padding: 12px;
    border: var(--sep);
    border-radius: var(--radius);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  h3 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--text-2);
  }
  .notes :global(.markdown) {
    margin-top: 12px;
  }
</style>
