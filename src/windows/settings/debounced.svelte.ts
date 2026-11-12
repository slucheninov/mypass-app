// A numeric setting edited with a stepper: the control shows the draft at once, the write is sent
// 300 ms after the last change (trailing), and the stored value takes over again when the write has
// finished, so a rejected or clamped write is reflected in the control. `flush` sends a pending
// change immediately (the pane closes before the timer fires).
export const DEBOUNCE_MS = 300;

export class DebouncedNumber {
  #draft = $state<number | null>(null);
  #timer: ReturnType<typeof setTimeout> | null = null;
  #read: () => number;
  #send: (value: number) => Promise<void>;

  constructor(read: () => number, send: (value: number) => Promise<void>) {
    this.#read = read;
    this.#send = send;
  }

  /** What the control shows: the draft while a change is pending or in flight, else the stored value. */
  get value(): number {
    return this.#draft ?? this.#read();
  }

  set(next: number): void {
    this.#draft = next;
    if (this.#timer !== null) clearTimeout(this.#timer);
    this.#timer = setTimeout(() => void this.#fire(), DEBOUNCE_MS);
  }

  /** Sends a pending change now; does nothing when none is pending. */
  flush(): void {
    if (this.#timer === null) return;
    clearTimeout(this.#timer);
    void this.#fire();
  }

  async #fire(): Promise<void> {
    this.#timer = null;
    const sent = this.#draft;
    if (sent === null) return;
    try {
      await this.#send(sent);
    } finally {
      // A newer edit made meanwhile keeps its draft; otherwise the stored value shows again.
      if (this.#draft === sent && this.#timer === null) this.#draft = null;
    }
  }
}
