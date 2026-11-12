// Drag and drop between the lists and the sidebar. Only ids travel in the drag data, under custom
// MIME types: never a title, a URL or any secret, so a drop onto another application leaks nothing.

export const ENTRY_MIME = "application/x-mypass-entry";
export const GROUP_MIME = "application/x-mypass-group";

export type DragKind = "entry" | "group";
export type DragPayload = { kind: DragKind; id: string };

const MIME: Record<DragKind, string> = { entry: ENTRY_MIME, group: GROUP_MIME };

/** Starts dragging the entry or group `id` (call from `ondragstart`). */
export function startDrag(event: DragEvent, kind: DragKind, id: string): void {
  const data = event.dataTransfer;
  if (!data) return;
  data.setData(MIME[kind], id);
  data.effectAllowed = "move";
}

/** What kind of item is being dragged. Works during `dragover`, when browsers hide the data. */
export function dragKind(event: DragEvent): DragKind | null {
  const types = Array.from(event.dataTransfer?.types ?? []);
  if (types.includes(ENTRY_MIME)) return "entry";
  if (types.includes(GROUP_MIME)) return "group";
  return null;
}

/** The dragged item, available in `drop` only. */
export function readDrag(event: DragEvent): DragPayload | null {
  const kind = dragKind(event);
  if (!kind) return null;
  const id = event.dataTransfer?.getData(MIME[kind]) ?? "";
  return id === "" ? null : { kind, id };
}
