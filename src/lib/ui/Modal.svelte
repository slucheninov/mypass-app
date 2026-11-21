<script lang="ts">
  import { tick, type Snippet } from "svelte";

  let {
    open,
    title,
    onclose,
    children,
    footer,
    icon,
    subtitle,
    backdropCloses = true,
    focusDialog = false,
  }: {
    open: boolean;
    title: string;
    onclose: () => void;
    children: Snippet;
    footer?: Snippet;
    /** A tile left of the title (a sheet with a subject, like the vault being opened); the dialog gets roomier padding. */
    icon?: Snippet;
    /** One muted line under the title. */
    subtitle?: string;
    /** A click on the backdrop closes the dialog (default). */
    backdropCloses?: boolean;
    /** Focus the dialog itself, not its first control: for a dialog nobody asked for, where a stray key must not press a button. */
    focusDialog?: boolean;
  } = $props();

  const uid = $props.id();
  const titleId = `modal-title-${uid}`;
  let dialog: HTMLElement | undefined = $state();
  // A drag that starts inside the dialog and ends on the backdrop must not close it.
  let pressedOnBackdrop = false;

  const FOCUSABLE =
    'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

  function focusables(): HTMLElement[] {
    return dialog ? Array.from(dialog.querySelectorAll<HTMLElement>(FOCUSABLE)) : [];
  }

  // Capture the opener when the dialog opens, move focus inside, restore it on close or destroy.
  $effect(() => {
    if (!open) return;
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    void tick().then(() => {
      const first = focusDialog ? undefined : focusables()[0];
      (first ?? dialog)?.focus();
    });
    return () => opener?.focus();
  });

  // A manual trap (no `inert`): Tab wraps between the first and last focusable element.
  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      onclose();
      return;
    }
    if (event.key !== "Tab") return;
    const items = focusables();
    const first = items[0];
    const last = items[items.length - 1];
    if (!first || !last) {
      event.preventDefault();
      dialog?.focus();
      return;
    }
    const active = document.activeElement;
    if (event.shiftKey && (active === first || active === dialog)) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus();
    } else if (!dialog?.contains(active)) {
      event.preventDefault();
      first.focus();
    }
  }
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div
    class="backdrop"
    onpointerdown={(e) => (pressedOnBackdrop = e.target === e.currentTarget)}
    onclick={(e) => {
      if (backdropCloses && pressedOnBackdrop && e.target === e.currentTarget) onclose();
      pressedOnBackdrop = false;
    }}
  >
    <div
      class="dialog"
      class:roomy={icon !== undefined}
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      tabindex="-1"
      bind:this={dialog}
      {onkeydown}
    >
      <div class="header">
        {#if icon}<div class="header-icon">{@render icon()}</div>{/if}
        <div class="header-text">
          <h2 id={titleId}>{title}</h2>
          {#if subtitle}<p class="subtitle">{subtitle}</p>{/if}
        </div>
      </div>
      <div class="body">{@render children()}</div>
      {#if footer}<div class="footer">{@render footer()}</div>{/if}
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--overlay);
  }
  .dialog {
    min-width: 320px;
    max-width: min(560px, calc(100vw - 32px));
    max-height: calc(100vh - 48px);
    display: flex;
    flex-direction: column;
    background: var(--bg);
    border-radius: 10px;
    box-shadow: var(--shadow);
  }
  .header {
    display: flex;
    align-items: center;
    gap: var(--space-6);
    padding: 16px 20px 8px;
  }
  .header-icon {
    display: flex;
    flex: none;
  }
  .header-text {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }
  h2 {
    margin: 0;
    font-size: var(--font-size-lg);
    font-weight: 600;
  }
  .subtitle {
    margin: 0;
    overflow: hidden;
    color: var(--text-2);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .body {
    padding: 4px 20px 16px;
    overflow: auto;
  }
  .footer {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-4);
    padding: 12px 20px;
    border-top: var(--sep);
  }
  .roomy .header {
    padding: var(--space-8) var(--space-8) var(--space-6);
  }
  .roomy .body {
    padding: var(--space-2) var(--space-8) var(--space-7);
  }
  .roomy .footer {
    padding: var(--space-6) var(--space-8);
  }
</style>
