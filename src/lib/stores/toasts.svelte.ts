import { ApiFailure, errorDetail, errorMessage } from "../api";

export type ToastKind = "info" | "error" | "success";

export interface Toast {
  id: number;
  kind: ToastKind;
  /** Already translated. */
  text: string;
  /** English diagnostics shown in a collapsible; only ever set from `errorDetail`. */
  detail?: string;
  sticky: boolean;
}

/** How long a non-sticky toast stays. */
export const TOAST_LIFETIME_MS = 5000;

class ToastStore {
  items = $state<Toast[]>([]);
  #nextId = 1;
  #timers = new Map<number, ReturnType<typeof setTimeout>>();

  /** Adds a toast and returns its id. A non-sticky toast disappears after `TOAST_LIFETIME_MS`. */
  push(kind: ToastKind, text: string, opts: { sticky?: boolean; detail?: string } = {}): number {
    const id = this.#nextId++;
    const sticky = opts.sticky ?? false;
    const toast: Toast = { id, kind, text, sticky };
    if (opts.detail) toast.detail = opts.detail;
    this.items.push(toast);
    if (!sticky) this.#timers.set(id, setTimeout(() => this.dismiss(id), TOAST_LIFETIME_MS));
    return id;
  }

  dismiss(id: number): void {
    const timer = this.#timers.get(id);
    if (timer !== undefined) clearTimeout(timer);
    this.#timers.delete(id);
    this.items = this.items.filter((t) => t.id !== id);
  }

  clear(): void {
    for (const id of [...this.#timers.keys()]) this.dismiss(id);
    this.items = [];
  }

  /**
   * Shows a backend failure by its code. `cancelled` (a dismissed system prompt) shows nothing;
   * `quitting` is one short, non-sticky message with no detail (the app is going away, there is
   * nothing to retry); `io` and `internal` stay until closed and carry the English diagnostics.
   */
  reportError(e: unknown): void {
    const code = e instanceof ApiFailure ? e.code : null;
    if (code === "cancelled") return;
    const text = errorMessage(e);
    if (code === "quitting") {
      if (!this.items.some((t) => t.kind === "error" && t.text === text)) this.push("error", text);
      return;
    }
    const detail = errorDetail(e);
    this.push("error", text, detail ? { sticky: true, detail } : {});
  }
}

export const toasts = new ToastStore();
