// An in-memory stand-in for the Rust backend, installed with `mockIPC` (see render.ts). It is grown
// task by task: a command no task has implemented yet throws `NotImplementedInFake(<name>)`, so a
// test that needs one fails loudly instead of getting a silent default.
//
// The `handlers` object is typed against the generated `commands` of bindings.ts, so a command
// added on the Rust side and missing here fails `pnpm check`; the key sets are also compared at
// run time (fakeBackend.test.ts). There are no command counts anywhere.
import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { ApiError, commands, events } from "../lib/bindings";

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
  /** Opt-in handlers for plugin calls by wire name, e.g. `"plugin:opener|open_url"`. */
  plugins?: Record<string, (payload: unknown) => unknown>;
}

export interface FakeCall {
  command: CommandName;
  args: Args;
}

export interface FakeBackend {
  handlers: Handlers;
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

  const handlers = {
    appInfo: () => ({
      version: "0.0.0-test",
      os,
      deviceUnlockAvailable,
    }),
    appOpenEntry: notImplemented("appOpenEntry"),
    appOpenSettings: notImplemented("appOpenSettings"),
    appRecordActivity: notImplemented("appRecordActivity"),
    appRevealBackups: notImplemented("appRevealBackups"),
    attachmentExport: notImplemented("attachmentExport"),
    attachmentPick: notImplemented("attachmentPick"),
    backupList: notImplemented("backupList"),
    backupRestore: notImplemented("backupRestore"),
    draftClear: notImplemented("draftClear"),
    draftSave: notImplemented("draftSave"),
    draftVersions: notImplemented("draftVersions"),
    entryCopy: notImplemented("entryCopy"),
    entryDelete: notImplemented("entryDelete"),
    entryEditModel: notImplemented("entryEditModel"),
    entryEmptyBin: notImplemented("entryEmptyBin"),
    entryGet: notImplemented("entryGet"),
    entryHistory: notImplemented("entryHistory"),
    entryHistoryRestore: notImplemented("entryHistoryRestore"),
    entryList: notImplemented("entryList"),
    entryMove: notImplemented("entryMove"),
    entryNewModel: notImplemented("entryNewModel"),
    entryPurge: notImplemented("entryPurge"),
    entryRecent: notImplemented("entryRecent"),
    entryRestore: notImplemented("entryRestore"),
    entryReveal: notImplemented("entryReveal"),
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
    groupCreate: notImplemented("groupCreate"),
    groupDelete: notImplemented("groupDelete"),
    groupMove: notImplemented("groupMove"),
    groupUpdate: notImplemented("groupUpdate"),
    importCsvCommit: notImplemented("importCsvCommit"),
    importCsvPreview: notImplemented("importCsvPreview"),
    platformKeyboardLayout: () => keyboardLayout,
    settingsGet: notImplemented("settingsGet"),
    settingsLoginItemStatus: notImplemented("settingsLoginItemStatus"),
    settingsUpdate: notImplemented("settingsUpdate"),
    shortcutConflictsTake: notImplemented("shortcutConflictsTake"),
    sidebarGet: notImplemented("sidebarGet"),
    vaultCreate: notImplemented("vaultCreate"),
    vaultFindFiles: notImplemented("vaultFindFiles"),
    vaultForget: notImplemented("vaultForget"),
    vaultLock: notImplemented("vaultLock"),
    vaultOpen: notImplemented("vaultOpen"),
    vaultOpenWarnings: notImplemented("vaultOpenWarnings"),
    vaultRecent: notImplemented("vaultRecent"),
    vaultStatus: notImplemented("vaultStatus"),
    vaultUnlock: notImplemented("vaultUnlock"),
  } satisfies Handlers;

  const byWireName = new Map<string, CommandName>(
    (Object.keys(handlers) as CommandName[]).map((name) => [snake(name), name]),
  );
  const calls: FakeCall[] = [];
  const pluginHandlers: Record<string, (payload: unknown) => unknown> = { ...opts.plugins };

  return {
    handlers,
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
