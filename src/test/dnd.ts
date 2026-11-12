// Drag and drop helpers: jsdom has no DataTransfer, so a minimal one carries the ids.
import { fireEvent } from "@testing-library/svelte";

export class FakeDataTransfer {
  #data = new Map<string, string>();
  effectAllowed = "uninitialized";
  dropEffect = "none";
  get types(): string[] {
    return [...this.#data.keys()];
  }
  setData(type: string, value: string): void {
    this.#data.set(type, value);
  }
  getData(type: string): string {
    return this.#data.get(type) ?? "";
  }
}

/** Drags with a data transfer that already holds `mime` = `id` (what a list row would set), then drops on `target`. */
export async function dropPayload(target: Element, mime: string, id: string): Promise<FakeDataTransfer> {
  const dataTransfer = new FakeDataTransfer();
  dataTransfer.setData(mime, id);
  await fireEvent.dragOver(target, { dataTransfer });
  await fireEvent.drop(target, { dataTransfer });
  return dataTransfer;
}

/** A full drag from a draggable `source` (its own `dragstart` fills the data) onto `target`. */
export async function dragAndDrop(source: Element, target: Element): Promise<FakeDataTransfer> {
  const dataTransfer = new FakeDataTransfer();
  await fireEvent.dragStart(source, { dataTransfer });
  await fireEvent.dragOver(target, { dataTransfer });
  await fireEvent.drop(target, { dataTransfer });
  return dataTransfer;
}
