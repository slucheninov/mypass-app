<script lang="ts">
  // A save proposal from the browser extension (main window only). The event carries no secret;
  // the model, which does, comes from `browser_proposal_take` at the moment it is shown and goes
  // straight into the editor store (cleared on close, save and lock like every editor model).
  //
  // Rules (R16, C2, L2, I1, I2):
  // - A proposal never replaces an editor that holds unsaved changes, nor an editor that shows an
  //   earlier proposal. It shows one "Saving from your browser is waiting" toast instead, and is
  //   shown only after the user closed the editor.
  // - If the editor turns dirty while the model is being taken, the taken model is HELD here (a
  //   plain variable of this component, in no shared store) and treated like any waiting proposal.
  // - The backend keeps a proposal for 120 s from when it asked for it. A waiting proposal (and a
  //   held model) is dropped at the same age: the toast goes, the held password is overwritten and
  //   no take is attempted. A take that still answers not-found says so in one specific message.
  // - Proposals are not throttled by the backend, so only the LATEST one is kept: a burst leaves
  //   one waiting proposal, one toast and at most one editor. A replaced proposal is never taken
  //   (taking it only to drop it would pull its password into the webview); the backend expires it
  //   after 120 s, keeps at most 8 and wipes what it drops.
  import { onDestroy, onMount } from "svelte";
  import { api, ApiFailure } from "../lib/api";
  import { events, type EntryEditModel_Serialize, type ProposalEvent_Deserialize } from "../lib/bindings";
  import { t } from "../lib/i18n/index.svelte";
  import { lockGuard } from "../lib/lock-guard";
  import { listenTo } from "../lib/stores/browser.svelte";
  import { navigation } from "../lib/stores/navigation.svelte";
  import { toasts } from "../lib/stores/toasts.svelte";

  /** How long the backend keeps a proposal (`ProposalStore`). */
  const PROPOSAL_TTL_MS = 120_000;

  // Ids and URLs only, no secret.
  let pending = $state<ProposalEvent_Deserialize | null>(null);
  // True once the proposal had to wait behind an editor; it then opens when the editor is gone.
  let deferred = $state(false);
  // The taken model of `pending`, when the editor turned dirty during the take. A secret.
  let held: EntryEditModel_Serialize | null = null;
  // State, so the effect below runs again when a take finishes behind a closed editor.
  let busy = $state(false);
  let toastId: number | null = null;
  let expiry: ReturnType<typeof setTimeout> | null = null;

  const blocked = () =>
    navigation.presentation !== "browse" &&
    navigation.editModel !== null &&
    (navigation.editorSource === "browser" || navigation.editorDirty?.() === true);

  /** Overwrites what a model holds secret before it is let go. */
  function scrub(model: EntryEditModel_Serialize | null) {
    if (!model) return;
    model.password = "";
    model.totp = "";
    for (const field of model.customFields) field.value = "";
    if (model.card) {
      model.card.number = "";
      model.card.cvv = "";
    }
  }

  function dismissToast() {
    if (toastId !== null) toasts.dismiss(toastId);
    toastId = null;
  }

  function showWaiting() {
    // The user may have closed the sticky toast: show it again rather than trust a stale id.
    if (toastId !== null && toasts.items.some((x) => x.id === toastId)) return;
    toastId = toasts.push("info", t("Saving from your browser is waiting"), { sticky: true });
  }

  function forget() {
    if (expiry !== null) clearTimeout(expiry);
    expiry = null;
    scrub(held);
    held = null;
    pending = null;
    deferred = false;
    dismissToast();
  }

  function incoming(event: ProposalEvent_Deserialize) {
    // Latest only: what waited before is replaced (its held model, if any, is overwritten).
    scrub(held);
    held = null;
    pending = event;
    if (expiry !== null) clearTimeout(expiry);
    expiry = setTimeout(forget, PROPOSAL_TTL_MS);
    arrive();
  }

  /** Shows `pending` now, or parks it behind the editor. */
  function arrive() {
    if (busy || !pending) return;
    if (blocked()) {
      deferred = true;
      showWaiting();
    } else {
      void open();
    }
  }

  async function open() {
    const event = pending;
    if (!event || busy) return;
    busy = true;
    try {
      await show(event);
    } finally {
      busy = false;
    }
    // A newer proposal arrived while this one was taken.
    if (pending && pending !== event) arrive();
  }

  async function show(event: ProposalEvent_Deserialize) {
    const unchanged = lockGuard();
    let model = held;
    held = null;
    if (!model) {
      try {
        model = await api.browserProposalTake(event.id);
      } catch (e) {
        if (unchanged()) {
          if (e instanceof ApiFailure && e.code === "notFound") toasts.push("error", t("This browser request has expired"));
          else toasts.reportError(e);
        }
        if (pending === event) forget();
        return;
      }
    }
    if (!unchanged()) {
      scrub(model);
      return;
    }
    if (pending !== event) {
      // A newer proposal replaced this one while it was on its way: the newest wins.
      scrub(model);
      return;
    }
    if (blocked()) {
      // The user started typing meanwhile: nothing is replaced and nothing is asked.
      held = model;
      deferred = true;
      showWaiting();
      return;
    }
    if (expiry !== null) clearTimeout(expiry);
    expiry = null;
    pending = null;
    deferred = false;
    dismissToast();
    navigation.openEditor(model, "browser");
  }

  $effect(() => {
    // Every dependency is read first: a short-circuit would drop `presentation` from the set.
    const idle = navigation.presentation === "browse" && !busy;
    if (idle && pending && deferred) void open();
  });

  onMount(() => listenTo(events.proposalEvent, incoming));
  // Also covers a lock: the Shell, and with it this component, goes away.
  onDestroy(forget);
</script>
