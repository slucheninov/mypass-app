// The one wrapper over the generated `commands`: every call returns the unwrapped value and throws
// `ApiFailure` on a backend error, so a screen never handles a `{status}` result. IPC goes only
// through bindings.ts (guarded by scaffold.test.ts); this file and the fake backend are the only
// importers of `commands`.
import { commands, type ApiError, type ErrorCode } from "./bindings";
import { t } from "./i18n/index.svelte";

/** A backend failure: `code` drives the localised message, `detail` is English diagnostics. */
export class ApiFailure extends Error {
  readonly code: ErrorCode;
  constructor(code: ErrorCode, message = "") {
    super(code);
    this.name = "ApiFailure";
    this.code = code;
    this.detail = message;
  }
  /** `ApiError.message`: English, secret-free, shown only as collapsible details for `io` and `internal`. */
  readonly detail: string;
}

type Result<T> = { status: "ok"; data: T } | { status: "error"; error: ApiError };

/** Unwraps a generated command result; a backend error becomes an `ApiFailure`. */
export async function call<T>(p: Promise<Result<T>>): Promise<T> {
  const result = await p;
  if (result.status === "ok") return result.data;
  throw new ApiFailure(result.error.code, result.error.message);
}

function isApiError(e: unknown): e is ApiError {
  return typeof e === "object" && e !== null && typeof (e as ApiError).code === "string";
}

const MESSAGES: Record<ErrorCode, () => string> = {
  locked: () => t("Vault is locked"),
  noVault: () => t("No vault is open"),
  wrongPassword: () => t("Wrong master password"),
  alreadyExists: () => t("A vault file already exists there"),
  notFound: () => t("Item not found"),
  invalidInput: () => t("Invalid input"),
  limitExceeded: () => t("Limit reached"),
  corrupt: () => t("The vault file looks damaged"),
  unsupported: () => t("Not supported on this system"),
  changedOnDisk: () => t("The vault changed on disk"),
  io: () => t("Something went wrong"),
  quitting: () => t("MyPass is quitting"),
  generatorNoClass: () => t("Turn on at least one character type in the generator"),
  generatorUnsatisfiable: () => t("This site's password requirements could not be satisfied"),
  cancelled: () => t("Cancelled"),
  lockout: () => t("Too many attempts. Use your master password."),
  invalidated: () => t("Device unlock was reset. Enter your master password."),
  notEnrolledOnDevice: () => t("No fingerprint or device password is set up on this computer"),
  deviceUnlockUnavailable: () => t("Device unlock is not available on this computer"),
  internal: () => t("Something went wrong"),
};

/** The localised text for a failure. `ApiError.message` is never used here. */
export function errorMessage(e: unknown): string {
  if (e instanceof ApiFailure) return (MESSAGES[e.code] ?? MESSAGES.internal)();
  return t("Something went wrong");
}

/** English diagnostics for the collapsible "Details", only for `io` and `internal` (the backend
 *  keeps these messages free of secrets); every other code is fully explained by `errorMessage`. */
export function errorDetail(e: unknown): string | null {
  if (e instanceof ApiFailure && (e.code === "io" || e.code === "internal") && e.detail !== "") return e.detail;
  return null;
}

function wrap<A extends unknown[], T>(fn: (...args: A) => Promise<Result<T>>) {
  return (...args: A): Promise<T> => call(fn(...args));
}

/** `shortcut_conflicts_take` returns a bare array (no `Result`), so it cannot go through `call`. */
async function shortcutConflictsTake() {
  try {
    return await commands.shortcutConflictsTake();
  } catch (e) {
    if (isApiError(e)) throw new ApiFailure(e.code, e.message);
    throw e;
  }
}

/** One typed, already unwrapped async function per command. */
export const api = {
  appInfo: wrap(commands.appInfo),
  appOpenEntry: wrap(commands.appOpenEntry),
  appOpenSettings: wrap(commands.appOpenSettings),
  appRecordActivity: wrap(commands.appRecordActivity),
  appRevealBackups: wrap(commands.appRevealBackups),
  attachmentExport: wrap(commands.attachmentExport),
  attachmentPick: wrap(commands.attachmentPick),
  backupList: wrap(commands.backupList),
  backupRestore: wrap(commands.backupRestore),
  draftClear: wrap(commands.draftClear),
  draftSave: wrap(commands.draftSave),
  draftVersions: wrap(commands.draftVersions),
  entryCopy: wrap(commands.entryCopy),
  entryDelete: wrap(commands.entryDelete),
  entryEditModel: wrap(commands.entryEditModel),
  entryEmptyBin: wrap(commands.entryEmptyBin),
  entryGet: wrap(commands.entryGet),
  entryHistory: wrap(commands.entryHistory),
  entryHistoryRestore: wrap(commands.entryHistoryRestore),
  entryList: wrap(commands.entryList),
  entryMove: wrap(commands.entryMove),
  entryNewModel: wrap(commands.entryNewModel),
  entryPurge: wrap(commands.entryPurge),
  entryRecent: wrap(commands.entryRecent),
  entryRestore: wrap(commands.entryRestore),
  entryReveal: wrap(commands.entryReveal),
  entrySave: wrap(commands.entrySave),
  entrySetFavorite: wrap(commands.entrySetFavorite),
  entryTotp: wrap(commands.entryTotp),
  entryTouch: wrap(commands.entryTouch),
  generatorCopy: wrap(commands.generatorCopy),
  generatorGenerate: wrap(commands.generatorGenerate),
  generatorHistoryCopy: wrap(commands.generatorHistoryCopy),
  generatorHistoryDelete: wrap(commands.generatorHistoryDelete),
  generatorHistoryList: wrap(commands.generatorHistoryList),
  generatorHistoryReveal: wrap(commands.generatorHistoryReveal),
  generatorRulesSummary: wrap(commands.generatorRulesSummary),
  groupCreate: wrap(commands.groupCreate),
  groupDelete: wrap(commands.groupDelete),
  groupMove: wrap(commands.groupMove),
  groupUpdate: wrap(commands.groupUpdate),
  importCsvCommit: wrap(commands.importCsvCommit),
  importCsvPreview: wrap(commands.importCsvPreview),
  platformKeyboardLayout: wrap(commands.platformKeyboardLayout),
  settingsGet: wrap(commands.settingsGet),
  settingsLoginItemStatus: wrap(commands.settingsLoginItemStatus),
  settingsUpdate: wrap(commands.settingsUpdate),
  sidebarGet: wrap(commands.sidebarGet),
  vaultCreate: wrap(commands.vaultCreate),
  vaultFindFiles: wrap(commands.vaultFindFiles),
  vaultForget: wrap(commands.vaultForget),
  vaultLock: wrap(commands.vaultLock),
  vaultOpen: wrap(commands.vaultOpen),
  vaultOpenWarnings: wrap(commands.vaultOpenWarnings),
  vaultRecent: wrap(commands.vaultRecent),
  vaultStatus: wrap(commands.vaultStatus),
  vaultUnlock: wrap(commands.vaultUnlock),
  shortcutConflictsTake,
} satisfies Record<keyof typeof commands, unknown>;
