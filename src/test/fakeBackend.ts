// An in-memory stand-in for the Rust backend, installed with `mockIPC` (see render.ts). Every command
// of bindings.ts has a handler; a wire name it does not know (an unknown command or plugin call)
// throws `NotImplementedInFake(<name>)`, so a test never gets a silent default.
//
// The `handlers` object is typed against the generated `commands` of bindings.ts, so a command
// added on the Rust side and missing here fails `pnpm check`; the key sets are also compared at
// run time (fakeBackend.test.ts). There are no command counts anywhere.
import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  ApiError,
  AttachmentMeta,
  BackupFailure,
  BackupItem,
  CsvPreview,
  commands,
  DraftVersionDto,
  events,
  FieldRef,
  FoundVaultFileDto,
  GeneratorHistoryItem,
  GroupNode_Serialize,
  LoginItemStatus,
  OpenWarning,
  PickedFile,
  RecentVaultDto,
  RulesSummary_Serialize,
  SessionState,
  SessionStatus_Serialize,
  Settings_Serialize,
  ShortcutConflictEvent,
  TotpDto,
} from "../lib/bindings";
import {
  BIN_GROUP_ID,
  FIXTURE_MASTER_PASSWORD,
  PERSONAL_GROUP_ID,
  ROOT_GROUP_ID,
  defaultSettings,
  detailOf,
  changedKeys,
  makeEntry,
  fixtureEntries,
  fixtureGroups,
  fixtureSidebar,
  listEntries,
  sessionStatus,
  type FixtureEntry,
} from "./fixtures";

type Commands = typeof commands;
export type CommandName = keyof Commands;
export type EventName = keyof typeof events;

/** The success payload of a generated command (a `Result` is unwrapped; `shortcutConflictsTake` has none). */
export type OkData<K extends CommandName> =
  Awaited<ReturnType<Commands[K]>> extends infer R
    ? R extends { status: "ok"; data: infer D }
      ? D
      : R extends { status: "error" }
        ? never
        : R
    : never;

// Arguments arrive as the named object the generated wrapper sends. Handlers destructure what
// they need; the loose type keeps the table readable and is checked by the tests that use them.
export type Args = Record<string, any>;
export type Handler<K extends CommandName> = (args: Args) => OkData<K> | Promise<OkData<K>>;
export type Handlers = { [K in CommandName]: Handler<K> };

export class NotImplementedInFake extends Error {
  constructor(command: string) {
    super(`NotImplementedInFake(${command})`);
    this.name = "NotImplementedInFake";
  }
}

/** A handler that throws a backend failure: `throw apiError("locked")` inside a handler. */
export function apiError(code: ApiError["code"], message = ""): ApiError {
  return { code, message };
}

export interface FakeBackendOptions {
  os?: string;
  deviceUnlockAvailable?: boolean;
  /** The short layout code `platform_keyboard_layout` reports; `null` means unsupported. */
  keyboardLayout?: string | null;
  /** The session state the fake starts in (default `locked`). */
  session?: SessionState;
  /** Opt-in handlers for plugin calls by wire name, e.g. `"plugin:opener|open_url"`. */
  plugins?: Record<string, (payload: unknown) => unknown>;
}

export interface FakeCall {
  command: CommandName;
  args: Args;
}

/** The mutable in-memory vault behind the implemented handlers. */
export interface FakeState {
  session: SessionStatus_Serialize;
  settings: Settings_Serialize;
  entries: FixtureEntry[];
  /** The group tree below the root (the recycle bin last); `group_*` commands edit it. */
  groups: GroupNode_Serialize[];
  masterPassword: string;
  /** Vaults a test registers by path: `vault_open` of one loads its own content and password. */
  vaults: Record<string, { entries: FixtureEntry[]; groups: GroupNode_Serialize[]; masterPassword?: string }>;
  /** What `vault_open_warnings` answers for every path (default none). */
  openWarnings: OpenWarning[];
  /** The recent-vaults list `vault_recent` returns; `vault_forget` and a successful open/create edit it. */
  recents: RecentVaultDto[];
  /** What `vault_find_files` returns (default none). */
  foundFiles: FoundVaultFileDto[];
  /** Paths `vault_create` refuses with `alreadyExists`. */
  existingFiles: string[];
  /** The vault's `RecycleBinEnabled` flag, reported by `sidebar_get`; when false a delete purges. */
  recycleBinEnabled: boolean;
  /** What `entry_totp` answers (and `entry_reveal`/`entry_copy` resolve a `totp` field to). */
  totp: TotpDto;
  /** Makes `attachment_export` answer `false`, like a cancelled save dialog. */
  exportCancelled: boolean;
  /** What the window plugin's `is_minimized` answers (only reachable after `mockWindows`). */
  minimized: boolean;
  /** What the next `attachment_pick` answers, like the files chosen in the native dialog (empty = cancelled). */
  pickResult: PickedFile[];
  /** A failure the next `entry_save` throws (then it is cleared), e.g. `invalidInput`. */
  saveFailure: ApiError["code"] | null;
  /** Saved note versions per entry id, newest last. */
  drafts: Record<string, DraftVersionDto[]>;
  /** Like a draft key that cannot be created: `draft_save` answers `false` and stores nothing. */
  draftsAvailable: boolean;
  /** Site rules by canonical host (lower case, no scheme, path or `www.`); `generator_rules_summary` and `generator_generate` read them. */
  generatorRules: Record<string, RulesSummary_Serialize>;
  /** The generator history, oldest first; a row keeps its password here like the vault's hidden group does. */
  generatorHistory: (GeneratorHistoryItem & { password: string })[];
  /** The backup files `backup_list` shows, newest first. */
  backups: BackupItem[];
  /** The last snapshot failure `backup_list` reports. */
  backupFailure: BackupFailure | null;
  /** A failure the next `backup_restore` throws (then it is cleared), e.g. `invalidInput` for a password mismatch. */
  restoreFailure: ApiError["code"] | null;
  /** What `settings_login_item_status` answers; `settings_update` of `launchAtLogin` flips enabled/disabled. */
  loginItem: LoginItemStatus;
  /** A system message the next `launchAtLogin` update fails with (code `internal`), like a refused registration. */
  loginItemError: string | null;
  /** A failure the next `settings_update` throws (then it is cleared). */
  settingsFailure: ApiError["code"] | null;
  /** What the next `import_csv_preview` shows, like the file chosen in the native dialog (`null` = cancelled). */
  csvPreview: CsvPreview | null;
  /** A failure the next `import_csv_preview` throws (then it is cleared), with the backend's English message. */
  csvFailure: ApiError | null;
  /** What the next `shortcut_conflicts_take` returns (and empties). */
  shortcutConflicts: ShortcutConflictEvent[];
  /** Ids `entry_touch` was called with, in order. */
  touched: string[];
}

export interface FakeBackend {
  handlers: Handlers;
  state: FakeState;
  /** Every command the webview invoked, in order. */
  calls: FakeCall[];
  /** Installs this backend as the IPC of the current test (called by `renderWithBackend`). */
  install(): void;
  /** Delivers a backend event to the listeners registered through the generated `events`. */
  emit(event: EventName, payload?: unknown): Promise<void>;
}

const snake = (name: string) => name.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`);
const kebab = (name: string) => name.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`);

export function createFakeBackend(opts: FakeBackendOptions = {}): FakeBackend {
  const { os = "macos", deviceUnlockAvailable = false, keyboardLayout = null } = opts;
  const state: FakeState = {
    session: sessionStatus(opts.session ?? "locked"),
    settings: defaultSettings(),
    entries: fixtureEntries(),
    groups: fixtureGroups(),
    masterPassword: FIXTURE_MASTER_PASSWORD,
    vaults: {},
    openWarnings: [],
    recents: [],
    foundFiles: [],
    existingFiles: [],
    recycleBinEnabled: true,
    totp: { code: "123456", secondsRemaining: 12, period: 30 },
    exportCancelled: false,
    minimized: false,
    pickResult: [],
    saveFailure: null,
    drafts: {},
    draftsAvailable: true,
    generatorRules: {},
    generatorHistory: [],
    backups: [],
    backupFailure: null,
    restoreFailure: null,
    loginItem: "disabled",
    loginItemError: null,
    settingsFailure: null,
    csvPreview: null,
    csvFailure: null,
    touched: [],
    shortcutConflicts: [],
  };
  /** Like the Rust grant: set by a successful preview, spent by the next commit. */
  let csvRemembered: CsvPreview | null = null;
  // The real backend emits its events as a side effect of a command; so does the fake.
  const publish = (event: EventName, payload: unknown) => void emit(kebab(event), payload ?? null);
  const requireUnlocked = () => {
    if (state.session.state !== "unlocked") throw apiError("locked");
  };
  const findEntry = (id: string) => {
    const found = state.entries.find((e) => e.summary.id === id);
    if (!found) throw apiError("notFound");
    return found;
  };
  // Group tree helpers (independent of the app's own, so a client bug cannot hide behind them).
  const locate = (id: string, nodes: GroupNode_Serialize[] = state.groups): { node: GroupNode_Serialize; siblings: GroupNode_Serialize[] } | null => {
    for (const node of nodes) {
      if (node.id === id) return { node, siblings: nodes };
      const inside = locate(id, node.children);
      if (inside) return inside;
    }
    return null;
  };
  const subtreeIds = (node: GroupNode_Serialize): string[] => [node.id, ...node.children.flatMap(subtreeIds)];
  /** The list a new child of `parentId` goes into; the top level keeps the recycle bin last. */
  const childrenOf = (parentId: string | null): GroupNode_Serialize[] => {
    if (parentId === null || parentId === ROOT_GROUP_ID) return state.groups;
    const parent = locate(parentId);
    if (!parent) throw apiError("notFound");
    return parent.node.children;
  };
  const place = (list: GroupNode_Serialize[], node: GroupNode_Serialize) => {
    const bin = list.findIndex((g) => g.isRecycleBin);
    list.splice(bin === -1 ? list.length : bin, 0, node);
  };
  const sendToBin = (entry: FixtureEntry) => {
    entry.summary.groupId = BIN_GROUP_ID;
    entry.model.groupId = BIN_GROUP_ID;
    entry.summary.inRecycleBin = true;
  };
  /** Like `Database::delete_entry`: purges when the bin is disabled or the entry is already in it. */
  const deleteLikeBackend = (entry: FixtureEntry) => {
    if (!state.recycleBinEnabled || entry.summary.inRecycleBin) {
      state.entries.splice(state.entries.indexOf(entry), 1);
    } else {
      sendToBin(entry);
    }
  };
  const changed = () => publish("vaultChanged", null);
  /** The value behind a field reference, as the backend would copy or reveal it. */
  const secretOf = (entry: FixtureEntry, field: FieldRef): string => {
    const { model } = entry;
    switch (field.kind) {
      case "password":
        return model.password;
      case "username":
        return model.username;
      case "url":
        return model.urls[0] ?? "";
      case "notesBody":
        return model.notes;
      case "cardNumber":
        return model.card?.number ?? "";
      case "cardCvv":
        return model.card?.cvv ?? "";
      case "totp":
        if (!entry.summary.hasTotp) throw apiError("invalidInput");
        return state.totp.code;
      case "custom": {
        const found = model.customFields.find((f) => f.key === field.key);
        if (!found) throw apiError("notFound");
        return found.value;
      }
    }
  };
  let groupSeq = 0;
  let generatedSeq = 0;
  let historySeq = 0;
  /** The host a rules lookup keys on: what the user typed or pasted, reduced to its lower-case host. */
  const canonicalHost = (raw: string) =>
    raw.trim().toLowerCase().replace(/^[a-z][a-z0-9+.-]*:\/\//, "").replace(/[/?#].*$/, "").replace(/^www\./, "");
  const rulesOf = (host: string | null | undefined): RulesSummary_Serialize | null =>
    host ? (state.generatorRules[canonicalHost(host)] ?? null) : null;
  const recordHistory = (value: string, url: string) => {
    const title = canonicalHost(url) || url;
    state.generatorHistory.push({ id: `gen-${++historySeq}`, url, title, created: 1_700_000_000_000 + historySeq * 1000, password: value });
    if (state.generatorHistory.length > 30) state.generatorHistory.shift();
    changed();
  };
  let entrySeq = 0;
  /** Tokens `attachment_pick` handed out; `entry_save` swaps each for its file and spends it. */
  const pickedFiles = new Map<string, PickedFile>();

  const setSession = (next: SessionStatus_Serialize) => {
    state.session = next;
    publish("sessionChanged", structuredClone(next));
    return structuredClone(next);
  };

  const fileNameOf = (path: string) => path.split(/[\\/]/).pop() ?? path;
  const rememberRecent = (path: string) => {
    state.recents = [
      { path, fileName: fileNameOf(path), exists: true },
      ...state.recents.filter((r) => r.path !== path),
    ];
  };
  // Opening or creating a vault unlocks it straight away, like the real backend.
  const openVault = (path: string, name: string) => {
    rememberRecent(path);
    return setSession({
      state: "unlocked",
      vaultPath: path,
      vaultName: name,
      lockReason: null,
      fileStatus: { kind: "connected", size: 4096, modified: 1_700_000_000_000 },
      save: { state: "idle", message: null },
    });
  };

  const handlers = {
    appInfo: () => ({
      version: "0.0.0-test",
      os,
      deviceUnlockAvailable,
    }),
    // Like Rust: a locked session or a missing entry does nothing; otherwise the main window shows
    // and `OpenEntryEvent` goes out (the window calls are not modelled).
    appOpenEntry: ({ id, edit }) => {
      if (state.session.state !== "unlocked") return null;
      if (!state.entries.some((e) => e.summary.id === id)) return null;
      publish("openEntryEvent", { id, edit });
      return null;
    },
    appOpenSettings: () => null,
    // Only the call log matters: the window and the quit flow live in Rust.
    appQuit: () => null,
    // Only the call log matters: the flag is read by the Rust quit flow.
    appSetEditorDirty: () => null,
    appShowMain: () => null,
    appRecordActivity: () => null,
    // Only the call log matters: the file manager opens on the Rust side.
    appRevealBackups: () => null,
    // Like Rust: the save dialog opens there; `false` means the user cancelled it.
    attachmentExport: ({ entryId, name }) => {
      requireUnlocked();
      if (!findEntry(entryId).model.attachments.some((a) => a.name === name)) throw apiError("notFound");
      return !state.exportCancelled;
    },
    // Like Rust: the native dialog opens there; the webview only gets single-use tokens back.
    attachmentPick: () => {
      requireUnlocked();
      const picked = structuredClone(state.pickResult);
      state.pickResult = [];
      for (const file of picked) pickedFiles.set(file.token, file);
      return picked;
    },
    backupList: () => ({
      items: structuredClone(state.backups),
      lastFailure: structuredClone(state.backupFailure),
      count: state.settings.backupCount,
      interval: state.settings.backupInterval,
    }),
    // Like Rust: the vault is replaced and locked; a wrong-password backup is `invalidInput`.
    backupRestore: ({ id }) => {
      if (state.restoreFailure) {
        const code = state.restoreFailure;
        state.restoreFailure = null;
        throw apiError(code);
      }
      if (!state.backups.some((b) => b.id === id)) throw apiError("notFound");
      setSession({ ...sessionStatus("locked"), lockReason: "manual" });
      return null;
    },
    draftClear: ({ entryId }) => {
      requireUnlocked();
      delete state.drafts[entryId];
      return null;
    },
    // `false` is "no draft key": nothing is stored and the editor works without drafts.
    draftSave: ({ entryId, notes }) => {
      requireUnlocked();
      if (!state.draftsAvailable) return false;
      const list = (state.drafts[entryId] ??= []);
      list.push({ id: `draft-${list.length + 1}-${entryId}`, savedAt: Date.now(), notes });
      if (list.length > 10) list.shift();
      return true;
    },
    // Newest first, like the backend.
    draftVersions: ({ entryId }) => {
      requireUnlocked();
      return structuredClone(state.drafts[entryId] ?? []).reverse();
    },
    // The value goes to the (fake) clipboard inside Rust and never reaches the webview.
    entryCopy: ({ id, field }) => {
      requireUnlocked();
      secretOf(findEntry(id), field);
      return null;
    },
    entryDelete: ({ id }) => {
      requireUnlocked();
      deleteLikeBackend(findEntry(id));
      changed();
      return null;
    },
    entryEditModel: ({ id }) => {
      requireUnlocked();
      return structuredClone(findEntry(id).model);
    },
    entryEmptyBin: () => {
      requireUnlocked();
      const before = state.entries.length;
      state.entries = state.entries.filter((e) => !e.summary.inRecycleBin);
      const count = before - state.entries.length;
      if (count > 0) changed();
      return count;
    },
    entryGet: ({ id }) => {
      requireUnlocked();
      return detailOf(findEntry(id));
    },
    // Newest first; `changedFields` names what the next newer version (or the entry) changed.
    entryHistory: ({ id }) => {
      requireUnlocked();
      const found = findEntry(id);
      return [...found.history].reverse().map((v, index) => {
        const newer = found.history[found.history.length - index] ?? { model: found.model };
        return {
          index,
          modified: v.modified,
          title: v.model.title,
          changedFields: v.changedFields ?? changedKeys(v.model, newer.model),
        };
      });
    },
    // Like `editing::restore_history`: the current state is snapshotted first (the restore is
    // undoable), then the version's content replaces it; the group and the favourite stay.
    entryHistoryRestore: ({ id, index }) => {
      requireUnlocked();
      const found = findEntry(id);
      const old = found.history[found.history.length - 1 - index];
      if (!Number.isInteger(index) || index < 0 || !old) throw apiError("notFound");
      found.history.push({ modified: found.modified, model: structuredClone(found.model) });
      found.model = {
        ...structuredClone(old.model),
        id: found.model.id,
        groupId: found.model.groupId,
        favorite: found.model.favorite,
      };
      found.summary.title = found.model.title;
      found.summary.subtitle = found.model.username;
      found.summary.hasTotp = found.model.totp !== "";
      found.summary.attachmentCount = found.model.attachments.length;
      found.modified += 1;
      found.summary.modified = found.modified;
      changed();
      return null;
    },
    entryList: ({ query }) => {
      requireUnlocked();
      return listEntries(state.entries, query, state.groups);
    },
    entryMove: ({ id, groupId }) => {
      requireUnlocked();
      const found = findEntry(id);
      if (groupId !== ROOT_GROUP_ID && !locate(groupId)) throw apiError("notFound");
      found.summary.groupId = groupId;
      found.model.groupId = groupId;
      found.summary.inRecycleBin = locate(groupId)?.node.isRecycleBin ?? false;
      changed();
      return null;
    },
    entryNewModel: ({ entryType, groupId }) => {
      requireUnlocked();
      const blankAddress = {
        label: "Home",
        fullName: "",
        givenName: "",
        familyName: "",
        organization: "",
        addressLine1: "",
        addressLine2: "",
        city: "",
        region: "",
        postalCode: "",
        countryCode: "",
        countryName: "",
        phone: "",
        email: "",
      };
      return {
        id: null,
        entryType,
        groupId: groupId ?? PERSONAL_GROUP_ID,
        title: "",
        favorite: false,
        username: "",
        password: "",
        urls: [],
        uriMatchMode: "registrable-domain",
        totp: "",
        notes: "",
        address: entryType === "address" ? blankAddress : null,
        card: entryType === "card" ? { holder: "", number: "", expiry: "", cvv: "" } : null,
        customFields: [],
        attachments: [],
        attachmentOps: [],
      };
    },
    entryPurge: ({ id }) => {
      requireUnlocked();
      state.entries.splice(state.entries.indexOf(findEntry(id)), 1);
      changed();
      return null;
    },
    // The fixture has no access times, so "recently used" is "recently modified".
    entryRecent: ({ limit }) => {
      requireUnlocked();
      return state.entries
        .filter((e) => !e.summary.inRecycleBin)
        .sort((a, b) => b.modified - a.modified)
        .slice(0, limit)
        .map((e) => structuredClone(e.summary));
    },
    // Like `editing::restore_entry`: only a bin entry, into `groupId` (the root when null; the real
    // backend does not remember the group an entry came from either).
    entryRestore: ({ id, groupId }) => {
      requireUnlocked();
      const found = findEntry(id);
      if (!found.summary.inRecycleBin) throw apiError("invalidInput");
      const target = groupId ?? ROOT_GROUP_ID;
      if (target !== ROOT_GROUP_ID && (!locate(target) || locate(target)?.node.isRecycleBin)) {
        throw apiError(locate(target) ? "invalidInput" : "notFound");
      }
      found.summary.groupId = target;
      found.model.groupId = target;
      found.summary.inRecycleBin = false;
      changed();
      return null;
    },
    entryReveal: ({ id, field }) => {
      requireUnlocked();
      return secretOf(findEntry(id), field);
    },
    // Creates (no id) or replaces the entry like `Database::save_entry`: the title is trimmed (an
    // address with none takes its label), attachment ops are applied in order with the real limits
    // (8 files, 5 MB each), and a token is spent when it is swapped for its file.
    entrySave: ({ model }) => {
      requireUnlocked();
      if (state.saveFailure) {
        const code = state.saveFailure;
        state.saveFailure = null;
        throw apiError(code);
      }
      let attachments: AttachmentMeta[] = structuredClone(model.attachments);
      for (const op of model.attachmentOps) {
        if (op.kind === "remove") {
          attachments = attachments.filter((a) => a.name !== op.name);
          continue;
        }
        const file = pickedFiles.get(op.path);
        if (!file) throw apiError("invalidInput");
        if ((file.size ?? 0) > 5 * 1024 * 1024) throw apiError("limitExceeded");
        attachments.push({ name: file.name, size: file.size });
      }
      if (attachments.length > 8) throw apiError("limitExceeded");
      for (const op of model.attachmentOps) if (op.kind === "add") pickedFiles.delete(op.path);
      const title =
        model.entryType === "address" && model.title.trim() === "" ? (model.address?.label ?? "") : model.title.trim();
      const saved = { ...structuredClone(model), title, attachments, attachmentOps: [] };
      const found = model.id ? findEntry(model.id) : null;
      if (found) {
        // Every save of an existing entry snapshots the previous state into its history.
        found.history.push({ modified: found.modified, model: structuredClone(found.model) });
        found.model = saved;
        found.summary.title = title;
        found.summary.favorite = model.favorite;
        found.summary.subtitle = model.username;
        found.summary.groupId = model.groupId;
        // Like `save_entry`: a save into another group moves the entry, out of the bin too.
        found.summary.inRecycleBin = locate(model.groupId)?.node.isRecycleBin ?? false;
        found.summary.attachmentCount = attachments.length;
        found.summary.hasTotp = model.totp !== "";
        found.modified += 1;
        found.summary.modified = found.modified;
        changed();
        return found.summary.id;
      }
      const created = makeEntry({
        id: `e-new-${++entrySeq}`,
        title,
        entryType: model.entryType,
        groupId: model.groupId,
        favorite: model.favorite,
        hasTotp: model.totp !== "",
        modified: Date.now(),
      });
      created.model = { ...saved, id: created.summary.id };
      created.summary.subtitle = model.username;
      created.summary.attachmentCount = attachments.length;
      state.entries.push(created);
      changed();
      return created.summary.id;
    },
    entrySetFavorite: ({ id, favorite }) => {
      requireUnlocked();
      const found = findEntry(id);
      found.summary.favorite = favorite;
      found.model.favorite = favorite;
      changed();
      return null;
    },
    entryTotp: ({ id }) => {
      requireUnlocked();
      if (!findEntry(id).summary.hasTotp) throw apiError("invalidInput");
      return { ...state.totp };
    },
    entryTouch: ({ id }) => {
      requireUnlocked();
      findEntry(id);
      state.touched.push(id);
      return null;
    },
    // The value goes to the (fake) clipboard inside Rust; with a record URL the history is written first.
    generatorCopy: ({ value, recordUrl }) => {
      if (recordUrl !== null && recordUrl !== undefined) {
        requireUnlocked();
        recordHistory(value, recordUrl);
      }
      return null;
    },
    // Mirrors `resolve_options`: the length is clamped to the site bounds, forced classes switch on,
    // excluded ones off; no class is `generatorNoClass`, every class excluded `generatorUnsatisfiable`.
    // The value is deterministic (no RNG): it cycles the enabled classes so each one appears.
    generatorGenerate: ({ req }) => {
      const { options, host, recordUrl } = req as {
        options: { length?: number; lowercase?: boolean; uppercase?: boolean; digits?: boolean; symbols?: boolean };
        host?: string | null;
        recordUrl?: string | null;
      };
      const rules = rulesOf(host);
      const length = options.length ?? 20;
      if (length < 8 || length > 64) throw apiError("invalidInput");
      const pick = (name: "lowercase" | "uppercase" | "digits" | "symbols") => {
        const own = options[name] ?? true;
        return rules ? rules.forcedOn.includes(name) || (own && !rules.excluded.includes(name)) : own;
      };
      const asked = [options.lowercase ?? true, options.uppercase ?? true, options.digits ?? true, options.symbols ?? true];
      if (!asked.some(Boolean) && !(rules && rules.forcedOn.length > 0)) throw apiError("generatorNoClass");
      let effectiveLength = length;
      if (rules?.minLength != null) effectiveLength = Math.max(effectiveLength, rules.minLength);
      if (rules?.maxLength != null) effectiveLength = Math.min(effectiveLength, rules.maxLength);
      const effective = {
        length: effectiveLength,
        lowercase: pick("lowercase"),
        uppercase: pick("uppercase"),
        digits: pick("digits"),
        symbols: pick("symbols"),
      };
      const pools = [
        effective.lowercase ? "abcdefghijkmnopqrstuvwxyz" : "",
        effective.uppercase ? "ABCDEFGHJKLMNPQRSTUVWXYZ" : "",
        effective.digits ? "23456789" : "",
        effective.symbols ? "!#$%&*+=?@" : "",
      ].filter((p) => p !== "");
      if (pools.length === 0) throw apiError("generatorUnsatisfiable");
      if (recordUrl !== null && recordUrl !== undefined) requireUnlocked();
      const seq = ++generatedSeq;
      let value = "";
      for (let i = 0; i < effectiveLength; i++) {
        const pool = pools[i % pools.length]!;
        value += pool[(i * 7 + seq) % pool.length];
      }
      let historyId: string | null = null;
      if (recordUrl !== null && recordUrl !== undefined) {
        recordHistory(value, recordUrl);
        historyId = state.generatorHistory[state.generatorHistory.length - 1]!.id;
      }
      return { value, effective, rules: structuredClone(rules), historyId };
    },
    generatorHistoryCopy: ({ id }) => {
      requireUnlocked();
      if (!state.generatorHistory.some((h) => h.id === id)) throw apiError("notFound");
      return null;
    },
    generatorHistoryDelete: ({ id }) => {
      requireUnlocked();
      const at = state.generatorHistory.findIndex((h) => h.id === id);
      if (at === -1) throw apiError("notFound");
      state.generatorHistory.splice(at, 1);
      changed();
      return null;
    },
    // Newest first, without the password.
    generatorHistoryList: () => {
      requireUnlocked();
      return state.generatorHistory.map(({ password: _password, ...item }) => item).reverse();
    },
    generatorHistoryReveal: ({ id }) => {
      requireUnlocked();
      const found = state.generatorHistory.find((h) => h.id === id);
      if (!found) throw apiError("notFound");
      return found.password;
    },
    generatorRulesSummary: ({ host }) => structuredClone(rulesOf(host)),
    groupCreate: ({ parentId, name, emoji }) => {
      requireUnlocked();
      const id = `group-new-${++groupSeq}`;
      place(childrenOf(parentId), { id, name, emoji, count: 0, isRecycleBin: false, children: [] });
      changed();
      return id;
    },
    groupDelete: ({ id }) => {
      requireUnlocked();
      const found = locate(id);
      if (!found) throw apiError("notFound");
      // Like `delete_group_removed`: the bin itself is not a group that can be deleted.
      if (found.node.isRecycleBin) throw apiError("invalidInput");
      // Like the backend: the group's items are moved to the recycle bin, then the group goes.
      const gone = subtreeIds(found.node);
      for (const e of [...state.entries]) if (gone.includes(e.summary.groupId)) deleteLikeBackend(e);
      found.siblings.splice(found.siblings.indexOf(found.node), 1);
      changed();
      return null;
    },
    groupMove: ({ id, parentId }) => {
      requireUnlocked();
      const found = locate(id);
      if (!found) throw apiError("notFound");
      if (found.node.isRecycleBin) throw apiError("invalidInput");
      // A group cannot go into itself or into its own subtree.
      if (subtreeIds(found.node).includes(parentId)) throw apiError("invalidInput");
      const target = childrenOf(parentId);
      found.siblings.splice(found.siblings.indexOf(found.node), 1);
      place(target, found.node);
      changed();
      return null;
    },
    groupUpdate: ({ id, name, emoji }) => {
      requireUnlocked();
      const found = locate(id);
      if (!found) throw apiError("notFound");
      found.node.name = name;
      found.node.emoji = emoji;
      changed();
      return null;
    },
    // Like Rust: the file is chosen in a native dialog there; a failed preview forgets the earlier file.
    importCsvPreview: () => {
      requireUnlocked();
      csvRemembered = null;
      if (state.csvFailure) {
        const failure = state.csvFailure;
        state.csvFailure = null;
        throw failure;
      }
      if (state.csvPreview === null) return null;
      csvRemembered = structuredClone(state.csvPreview);
      return structuredClone(state.csvPreview);
    },
    // The remembered file is spent by the commit; the fake writes one login per sample title (so a
    // total beyond the five titles imports as many as the titles it knows).
    importCsvCommit: ({ groupId }) => {
      requireUnlocked();
      const preview = csvRemembered;
      csvRemembered = null;
      if (!preview) throw apiError("invalidInput", "no CSV file was previewed");
      if (groupId !== ROOT_GROUP_ID && !locate(groupId)) throw apiError("notFound");
      for (const [index, title] of preview.sampleTitles.entries()) {
        state.entries.push(
          makeEntry({ id: `csv-${++entrySeq}-${index}`, title, groupId, modified: 1_700_000_000_000 + entrySeq }),
        );
      }
      changed();
      return { imported: preview.total, skipped: preview.skipped.length };
    },
    platformKeyboardLayout: () => keyboardLayout,
    settingsGet: () => structuredClone(state.settings),
    settingsLoginItemStatus: () => state.loginItem,
    settingsUpdate: ({ patch }) => {
      if (state.settingsFailure) {
        const code = state.settingsFailure;
        state.settingsFailure = null;
        throw apiError(code);
      }
      if (typeof patch.launchAtLogin === "boolean") {
        if (state.loginItemError !== null) {
          const message = state.loginItemError;
          state.loginItemError = null;
          throw apiError("internal", message);
        }
        if (state.loginItem === "enabled" || state.loginItem === "disabled") {
          state.loginItem = patch.launchAtLogin ? "enabled" : "disabled";
        }
      }
      const defined = Object.entries(patch as Record<string, unknown>).filter(([, v]) => v !== null && v !== undefined);
      const next = { ...state.settings, ...Object.fromEntries(defined) } as Settings_Serialize;
      // Nested objects merge field by field, like the backend's patch.
      if (patch.generator) next.generator = { ...state.settings.generator, ...patch.generator };
      // Like `Settings::normalize`: out-of-range numbers are clamped (the clipboard delay to a multiple of 5).
      next.backupCount = Math.min(50, Math.max(1, next.backupCount));
      next.clipboardClearSeconds = Math.floor((Math.min(120, Math.max(10, next.clipboardClearSeconds)) + 2) / 5) * 5;
      state.settings = next;
      publish("settingsChanged", structuredClone(state.settings));
      return structuredClone(state.settings);
    },
    // Take-once like the Rust list: what the hotkey layer could not register at startup.
    shortcutConflictsTake: () => state.shortcutConflicts.splice(0),
    sidebarGet: () => {
      requireUnlocked();
      return fixtureSidebar(state.entries, state.groups, state.recycleBinEnabled);
    },
    vaultCreate: ({ path, password, name }) => {
      if (state.existingFiles.includes(path)) throw apiError("alreadyExists");
      state.masterPassword = password;
      // A new vault is empty: the fixture content belongs to the vault that was open before.
      state.entries = [];
      state.groups = fixtureGroups().filter((g) => g.isRecycleBin);
      return openVault(path, name);
    },
    vaultFindFiles: () => structuredClone(state.foundFiles),
    vaultForget: ({ path }) => {
      state.recents = state.recents.filter((r) => r.path !== path);
      return null;
    },
    vaultLock: () => {
      if (state.session.state === "noVault") return null;
      setSession({ ...state.session, state: "locked", lockReason: "manual" });
      return null;
    },
    // Like Rust, opening loads THAT file: the path already open keeps its state (its own password
    // applies), a registered path brings its own content and password, and any other path is the
    // default fixture vault with the fixture password. The password is always checked against the
    // vault being opened, never the previous one; a refused open changes nothing. Leaving a vault
    // writes its current state back, so reopening it later finds its edits.
    vaultOpen: ({ path, password }) => {
      const name = fileNameOf(path).replace(/\.kdbx$/i, "");
      if (path === state.session.vaultPath) {
        if (password !== state.masterPassword) throw apiError("wrongPassword");
        return openVault(path, name);
      }
      const own = state.vaults[path];
      const expected = own?.masterPassword ?? FIXTURE_MASTER_PASSWORD;
      if (password !== expected) throw apiError("wrongPassword");
      if (state.session.vaultPath) {
        state.vaults[state.session.vaultPath] = {
          entries: structuredClone(state.entries),
          groups: structuredClone(state.groups),
          masterPassword: state.masterPassword,
        };
      }
      state.masterPassword = expected;
      state.entries = own ? structuredClone(own.entries) : fixtureEntries();
      state.groups = own ? structuredClone(own.groups) : fixtureGroups();
      return openVault(path, name);
    },
    vaultOpenWarnings: () => structuredClone(state.openWarnings),
    vaultRecent: () => structuredClone(state.recents),
    vaultStatus: () => structuredClone(state.session),
    vaultUnlock: ({ password }) => {
      if (state.session.state === "noVault") throw apiError("noVault");
      if (password !== state.masterPassword) throw apiError("wrongPassword");
      return setSession({ ...state.session, state: "unlocked", lockReason: null });
    },
  } satisfies Handlers;

  const byWireName = new Map<string, CommandName>(
    (Object.keys(handlers) as CommandName[]).map((name) => [snake(name), name]),
  );
  const calls: FakeCall[] = [];
  const pluginHandlers: Record<string, (payload: unknown) => unknown> = {
    "plugin:window|is_minimized": () => state.minimized,
    ...opts.plugins,
  };

  return {
    handlers,
    state,
    calls,
    install() {
      mockIPC(
        async (cmd, payload) => {
          const name = byWireName.get(cmd);
          if (!name) {
            // Calls of the runtime itself (event plumbing is mocked by `shouldMockEvents`) are the
            // only ones that may pass; everything else names itself so a test never gets a silent
            // `undefined` for a command or plugin the fake does not know.
            if (cmd.startsWith("plugin:")) {
              const plugin = pluginHandlers[cmd];
              if (plugin) return plugin(payload);
              throw new NotImplementedInFake(cmd);
            }
            throw new NotImplementedInFake(`unknown command ${cmd}`);
          }
          const args = (payload ?? {}) as Args;
          calls.push({ command: name, args });
          return (handlers[name] as Handler<CommandName>)(args);
        },
        { shouldMockEvents: true },
      );
    },
    emit(event, payload) {
      return emit(kebab(event), payload ?? null);
    },
  };
}
