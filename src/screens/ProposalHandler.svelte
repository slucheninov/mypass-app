<script lang="ts">
  // A save proposal from the browser extension (main window only). The event carries no secret;
  // the model, which does, comes from `browser_proposal_take` at the moment it is shown and goes
  // straight into the editor store (cleared on close, save and lock like every editor model).
  //
  // Rules (R16, C2, L2):
  // - A proposal never replaces an editor that holds unsaved changes, nor an editor that shows an
  //   earlier proposal. It shows one "Saving from your browser is waiting" toast instead, and is
  //   taken only after the user closed the editor.
  // - Proposals are not throttled by the backend, so only the LATEST one is kept: a burst leaves
  //   one waiting proposal, one toast and at most one editor. An older, replaced proposal is never
  //   taken (the backend drops it on lock).
  import { onDestroy, onMount } from "svelte";
  import { api } from "../lib/api";
  import { events, type ProposalEvent_Deserialize } from "../lib/bindings";
  import { t } from "../lib/i18n/index.svelte";
  import { lockGuard } from "../lib/lock-guard";
  import { listenTo } from "../lib/stores/browser.svelte";
  import { navigation } from "../lib/stores/navigation.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";

  // Ids and URLs only, no secret.
  let pending = $state<ProposalEvent_Deserialize | null>(null);
  // True once the proposal had to wait behind an editor; it then opens when the editor is gone.
  let deferred = $state(false);
  let busy = false;
  let toastId: number | null = null;

  const blocked = () =>
    navigation.presentation !== "browse" &&
    navigation.editModel !== null &&
    (navigation.editorSource === "browser" || navigation.editorDirty?.() === true);

  function dismissToast() {
    if (toastId !== null) toasts.dismiss(toastId);
    toastId = null;
  }

  function incoming(event: ProposalEvent_Deserialize) {
    pending = event;
    if (busy) return;
    if (blocked()) {
      deferred = true;
      // One toast however many proposals arrive.
      toastId ??= toasts.push("info", t("Saving from your browser is waiting"), { sticky: true });
    } else {
      void open();
    }
  }

  async function open() {
    const event = pending;
    if (!event || busy) return;
    pending = null;
    deferred = false;
    dismissToast();
    busy = true;
    const unchanged = lockGuard();
    try {
      const model = await api.browserProposalTake(event.id);
      if (!unchanged()) return;
      // The user may have started typing while the model was on its way: then the usual
      // "Discard changes?" question decides, nothing is replaced silently.
      if (blocked()) navigation.requestOpenEditor(model);
      else navigation.openEditor(model, "browser");
    } catch (e) {
      if (unchanged()) toasts.reportError(e);
    } finally {
      busy = false;
    }
    // A newer proposal arrived while this one was taken.
    if (pending) incoming(pending);
  }

  $effect(() => {
    if (pending && deferred && !busy && navigation.presentation === "browse") void open();
  });

  onMount(() => listenTo(events.proposalEvent, incoming));
  onDestroy(() => {
    pending = null;
    dismissToast();
  });
</script>
