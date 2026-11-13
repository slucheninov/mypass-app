import type { VaultFileStatus } from "../lib/bindings";
import { t } from "../lib/i18n/index.svelte";

export type VaultStatusKind = "connected" | "missing" | "unreadable";

/** The dot of the vault status card; `null` when the session has no vault. */
export function statusKind(status: VaultFileStatus): VaultStatusKind | null {
  return status.kind === "noVault" ? null : status.kind;
}

export function statusLabel(kind: VaultStatusKind): string {
  switch (kind) {
    case "connected":
      return t("Connected");
    case "missing":
      return t("File not found");
    case "unreadable":
      return t("File is not readable");
  }
}

/** Size and modified time of a connected file; a missing time (0 or null) is left out by the caller. */
export function connectedInfo(status: VaultFileStatus): { size: number | null; modified: number | null } | null {
  return status.kind === "connected" ? { size: status.size, modified: status.modified } : null;
}

const SEPARATOR = /[\\/]/;

/** The last path component (`C:\\Vaults\\a.kdbx` and `/v/a.kdbx` both give `a.kdbx`). */
export function fileNameOf(path: string): string {
  return path.split(SEPARATOR).pop() || path;
}

/** The folder part of a path, without the trailing separator (root stays `/`). */
export function folderOf(path: string): string {
  const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  if (cut < 0) return "";
  return cut === 0 ? path.slice(0, 1) : path.slice(0, cut);
}
