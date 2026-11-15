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
  downloading: () =>
    t("The vault at that location is still downloading from iCloud. Wait until it finishes, then try again."),
  quitting: () => t("MyPass is quitting"),
  generatorNoClass: () => t("Turn on at least one character type in the generator"),
  generatorUnsatisfiable: () => t("This site's password requirements could not be satisfied"),
  cancelled: () => t("Cancelled"),
  lockout: () => t("Too many attempts. Use your master password."),
  invalidated: () => t("Device unlock was reset. Enter your master password."),
  notEnrolledOnDevice: () => t("No fingerprint or device password is set up on this computer"),
  deviceUnlockUnavailable: () => t("Device unlock is not available on this computer"),
  migrationLocked: () => t("The vault changed or was locked. Unlock it and choose the folder again."),
  migrationTargetUnreadable: () => t("Cannot read the destination. Check access and availability, then choose the folder again."),
  migrationTargetNotAVault: () => t("The destination file is not a valid vault. Choose another folder or restore that file from a backup."),
  migrationTargetPasswordMismatch: () => t("The destination uses a different master password. Choose a vault with the same password."),
  migrationTargetWriteFailed: () => t("Cannot write the destination, or another app changed it. Check access and choose the folder again."),
  migrationSourceInvalid: () => t("Cannot validate the source vault or its backup. Keep the original file and restore a known good backup before retrying."),
  migrationSameFile: () => t("This is the current vault. Choose a different folder."),
  migrationValidationFailed: () => t("Destination verification failed. The source and verified backup are retained; the destination may already have changed. Keep all copies and inspect them before retrying."),
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
  appQuit: wrap(commands.appQuit),
  appRecordActivity: wrap(commands.appRecordActivity),
  appRevealBackups: wrap(commands.appRevealBackups),
  appSetEditorDirty: wrap(commands.appSetEditorDirty),
  appShowMain: wrap(commands.appShowMain),
  attachmentExport: wrap(commands.attachmentExport),
  attachmentPick: wrap(commands.attachmentPick),
  backupList: wrap(commands.backupList),
  backupRestore: wrap(commands.backupRestore),
  browserPairingCancel: wrap(commands.browserPairingCancel),
  browserProposalTake: wrap(commands.browserProposalTake),
  browserReinstallHost: wrap(commands.browserReinstallHost),
  browserResetPairings: wrap(commands.browserResetPairings),
  browserRevoke: wrap(commands.browserRevoke),
  browserShowExtensionFolder: wrap(commands.browserShowExtensionFolder),
  browserStatus: wrap(commands.browserStatus),
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
  entryPasskeyDetails: wrap(commands.entryPasskeyDetails),
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
  syncDisconnectSecondary: wrap(commands.syncDisconnectSecondary),
  syncConnectSecondary: wrap(commands.syncConnectSecondary),
  syncNow: wrap(commands.syncNow),
  syncStatus: wrap(commands.syncStatus),
  vaultCreate: wrap(commands.vaultCreate),
  vaultFindFiles: wrap(commands.vaultFindFiles),
  vaultForget: wrap(commands.vaultForget),
  vaultLock: wrap(commands.vaultLock),
  vaultOpen: wrap(commands.vaultOpen),
  vaultOpenWarnings: wrap(commands.vaultOpenWarnings),
  vaultRecent: wrap(commands.vaultRecent),
  vaultStatus: wrap(commands.vaultStatus),
  vaultStorageStatus: wrap(commands.vaultStorageStatus),
  vaultLocationPick: wrap(commands.vaultLocationPick),
  vaultLocationProbe: wrap(commands.vaultLocationProbe),
  vaultLocationChange: wrap(commands.vaultLocationChange),
  deviceUnlockStatus: wrap(commands.deviceUnlockStatus),
  deviceUnlockConfigure: wrap(commands.deviceUnlockConfigure),
  deviceUnlockConsumeExtensionRequest: commands.deviceUnlockConsumeExtensionRequest,
  vaultUnlockDevice: wrap(commands.vaultUnlockDevice),
  updateStatus: commands.updateStatus,
  updateCheckNow: wrap(commands.updateCheckNow),
  updateDismiss: wrap(commands.updateDismiss),
  updateOpenRelease: wrap(commands.updateOpenRelease),
  vaultUnlock: wrap(commands.vaultUnlock),
  shortcutConflictsTake,
} satisfies Record<keyof typeof commands, unknown>;
