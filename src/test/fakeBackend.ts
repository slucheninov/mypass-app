// An in-memory stand-in for the Rust backend, installed with `mockIPC` (see render.ts). It is grown
// task by task: a command no task has implemented yet throws `NotImplementedInFake(<name>)`, so a
// test that needs one fails loudly instead of getting a silent default.
//
// The `handlers` object is typed against the generated `commands` of bindings.ts, so a command
// added on the Rust side and missing here fails `pnpm check`; the key sets are also compared at
// run time (fakeBackend.test.ts). There are no command counts anywhere.
import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  ApiError,
  commands,
  events,
  FoundVaultFileDto,
  GroupNode_Serialize,
  OpenWarning,
  RecentVaultDto,
  SessionState,
  SessionStatus_Serialize,
  Settings_Serialize,
} from "../lib/bindings";
import {
  BIN_GROUP_ID,
  FIXTURE_MASTER_PASSWORD,
  PERSONAL_GROUP_ID,
  ROOT_GROUP_ID,
  defaultSettings,
  detailOf,
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

function notImplemented<K extends CommandName>(command: K): Handler<K> {
  return () => {
    throw new NotImplementedInFake(command);
  };
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
  /** What `vault_open_warnings` answers for every path (default none). */
  openWarnings: OpenWarning[];
  /** The recent-vaults list `vault_recent` returns; `vault_forget` and a successful open/create edit it. */
  recents: RecentVaultDto[];
  /** What `vault_find_files` returns (default none). */
  foundFiles: FoundVaultFileDto[];
  /** Paths `vault_create` refuses with `alreadyExists`. */
  existingFiles: string[];
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
    openWarnings: [],
    recents: [],
    foundFiles: [],
    existingFiles: [],
  };
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
  const changed = () => publish("vaultChanged", null);
  let groupSeq = 0;

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
    appOpenEntry: notImplemented("appOpenEntry"),
    appOpenSettings: () => null,
    appRecordActivity: () => null,
    appRevealBackups: notImplemented("appRevealBackups"),
    attachmentExport: notImplemented("attachmentExport"),
    attachmentPick: notImplemented("attachmentPick"),
    backupList: notImplemented("backupList"),
    backupRestore: notImplemented("backupRestore"),
    draftClear: notImplemented("draftClear"),
    draftSave: notImplemented("draftSave"),
    draftVersions: notImplemented("draftVersions"),
    entryCopy: notImplemented("entryCopy"),
    entryDelete: ({ id }) => {
      requireUnlocked();
      sendToBin(findEntry(id));
      changed();
      return null;
    },
    entryEditModel: ({ id }) => {
      requireUnlocked();
      return structuredClone(findEntry(id).model);
    },
    entryEmptyBin: notImplemented("entryEmptyBin"),
    entryGet: ({ id }) => {
      requireUnlocked();
      return detailOf(findEntry(id));
    },
    entryHistory: notImplemented("entryHistory"),
    entryHistoryRestore: notImplemented("entryHistoryRestore"),
    entryList: ({ query }) => {
      requireUnlocked();
      return listEntries(state.entries, query);
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
      return {
        id: null,
        entryType,
        groupId: groupId ?? PERSONAL_GROUP_ID,
        title: "",
        favorite: false,
        username: "",
        password: "",
        urls: [],
        uriMatchMode: "domain",
        totp: "",
        notes: "",
        address: null,
        card: null,
        customFields: [],
        attachments: [],
        attachmentOps: [],
      };
    },
    entryPurge: notImplemented("entryPurge"),
    entryRecent: notImplemented("entryRecent"),
    entryRestore: notImplemented("entryRestore"),
    entryReveal: ({ id, field }) => {
      requireUnlocked();
      const { model } = findEntry(id);
      if (field.kind === "password") return model.password;
      if (field.kind === "username") return model.username;
      throw new NotImplementedInFake(`entryReveal(${field.kind})`);
    },
    entrySave: notImplemented("entrySave"),
    entrySetFavorite: notImplemented("entrySetFavorite"),
    entryTotp: notImplemented("entryTotp"),
    entryTouch: notImplemented("entryTouch"),
    generatorCopy: notImplemented("generatorCopy"),
    generatorGenerate: notImplemented("generatorGenerate"),
    generatorHistoryCopy: notImplemented("generatorHistoryCopy"),
    generatorHistoryDelete: notImplemented("generatorHistoryDelete"),
    generatorHistoryList: notImplemented("generatorHistoryList"),
    generatorHistoryReveal: notImplemented("generatorHistoryReveal"),
    generatorRulesSummary: notImplemented("generatorRulesSummary"),
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
      if (!found || found.node.isRecycleBin) throw apiError("notFound");
      // Like the backend: the group's items are moved to the recycle bin, then the group goes.
      const gone = subtreeIds(found.node);
      for (const e of state.entries) if (gone.includes(e.summary.groupId)) sendToBin(e);
      found.siblings.splice(found.siblings.indexOf(found.node), 1);
      changed();
      return null;
    },
    groupMove: ({ id, parentId }) => {
      requireUnlocked();
      const found = locate(id);
      if (!found || found.node.isRecycleBin) throw apiError("notFound");
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
    importCsvCommit: notImplemented("importCsvCommit"),
    importCsvPreview: notImplemented("importCsvPreview"),
    platformKeyboardLayout: () => keyboardLayout,
    settingsGet: () => structuredClone(state.settings),
    settingsLoginItemStatus: notImplemented("settingsLoginItemStatus"),
    settingsUpdate: ({ patch }) => {
      const defined = Object.entries(patch as Record<string, unknown>).filter(([, v]) => v !== null && v !== undefined);
      const next = { ...state.settings, ...Object.fromEntries(defined) } as Settings_Serialize;
      // Nested objects merge field by field, like the backend's patch.
      if (patch.generator) next.generator = { ...state.settings.generator, ...patch.generator };
      state.settings = next;
      publish("settingsChanged", structuredClone(state.settings));
      return structuredClone(state.settings);
    },
    shortcutConflictsTake: notImplemented("shortcutConflictsTake"),
    sidebarGet: () => {
      requireUnlocked();
      return fixtureSidebar(state.entries, state.groups);
    },
    vaultCreate: ({ path, password, name }) => {
      if (state.existingFiles.includes(path)) throw apiError("alreadyExists");
      state.masterPassword = password;
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
    vaultOpen: ({ path, password }) => {
      if (password !== state.masterPassword) throw apiError("wrongPassword");
      return openVault(path, fileNameOf(path).replace(/\.kdbx$/i, ""));
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
  const pluginHandlers: Record<string, (payload: unknown) => unknown> = { ...opts.plugins };

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
