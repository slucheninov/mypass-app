<script lang="ts">
  // One open-guard warning (D6/D7). Cancel, Escape and a click on the backdrop decline; the dialog
  // never shows a path or a process list, only the executable name of the other MyPass.
  import { t } from "../lib/i18n/index.svelte";
  import type { OpenWarning } from "../lib/bindings";
  import Button from "../lib/ui/Button.svelte";
  import Modal from "../lib/ui/Modal.svelte";

  let { warning, ondecide }: { warning: OpenWarning; ondecide: (proceed: boolean) => void } = $props();

  const copy = $derived({
    title: t("Another copy of MyPass is running"),
    body: t(
      "An older MyPass (%@) is open. Two copies writing the same vault can overwrite each other's changes. Quit the other copy first, or continue anyway.",
      [warning.executable],
    ),
  });
</script>

<Modal open={true} title={copy.title} onclose={() => ondecide(false)}>
  <p>{copy.body}</p>
  {#snippet footer()}
    <Button onclick={() => ondecide(false)}>{t("Cancel")}</Button>
    <Button variant="primary" onclick={() => ondecide(true)}>{t("Continue")}</Button>
  {/snippet}
</Modal>

<style>
  p {
    margin: 0;
    line-height: 1.5;
  }
</style>
