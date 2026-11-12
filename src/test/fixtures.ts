// In-memory vault content for the fake backend and for store tests. The fixture login carries a
// password (`FIXTURE_LOGIN_PASSWORD`) so tests can prove it never reaches a store.
import type {
  Category,
  CategoryCount_Serialize,
  EntryDetailDto_Serialize,
  EntryEditModel_Serialize,
  EntryQuery,
  EntrySummary,
  GroupNode_Serialize,
  Scope,
  SecurityFlag,
  SessionStatus_Serialize,
  Settings_Serialize,
  Sidebar_Serialize,
} from "../lib/bindings";

export const FIXTURE_MASTER_PASSWORD = "correct horse battery";
export const FIXTURE_LOGIN_PASSWORD = "hunter2";
export const FIXTURE_VAULT_PATH = "/vaults/test.kdbx";

export const ROOT_GROUP_ID = "group-root";
export const PERSONAL_GROUP_ID = "group-personal";
export const WORK_GROUP_ID = "group-work";
export const BIN_GROUP_ID = "group-bin";

export interface FixtureEntry {
  summary: EntrySummary;
  model: EntryEditModel_Serialize;
  modified: number;
}

interface EntrySpec {
  id: string;
  title: string;
  subtitle?: string;
  entryType?: EntrySummary["entryType"];
  groupId?: string;
  favorite?: boolean;
  hasTotp?: boolean;
  inRecycleBin?: boolean;
  flags?: SecurityFlag[];
  password?: string;
  urls?: string[];
  modified: number;
}

function entry(spec: EntrySpec): FixtureEntry {
  const entryType = spec.entryType ?? "login";
  const groupId = spec.groupId ?? PERSONAL_GROUP_ID;
  const favorite = spec.favorite ?? false;
  return {
    modified: spec.modified,
    summary: {
      id: spec.id,
      title: spec.title,
      subtitle: spec.subtitle ?? "",
      entryType,
      favorite,
      hasTotp: spec.hasTotp ?? false,
      hasPasskey: false,
      attachmentCount: 0,
      modified: spec.modified,
      groupId,
      inRecycleBin: spec.inRecycleBin ?? false,
      flags: spec.flags ?? [],
    },
    model: {
      id: spec.id,
      entryType,
      groupId,
      title: spec.title,
      favorite,
      username: spec.subtitle ?? "",
      password: spec.password ?? "",
      urls: spec.urls ?? [],
      uriMatchMode: "domain",
      totp: "",
      notes: "",
      address: null,
      card: null,
      customFields: [],
      attachments: [],
      attachmentOps: [],
    },
  };
}

export function fixtureEntries(): FixtureEntry[] {
  return [
    entry({
      id: "e-github",
      title: "GitHub",
      subtitle: "octo",
      favorite: true,
      hasTotp: true,
      password: FIXTURE_LOGIN_PASSWORD,
      urls: ["https://github.com"],
      modified: 1_700_000_300_000,
    }),
    entry({
      id: "e-gitlab",
      title: "GitLab",
      subtitle: "octo-lab",
      groupId: WORK_GROUP_ID,
      password: "gitlab-secret",
      urls: ["https://gitlab.com"],
      flags: ["weak"],
      modified: 1_700_000_200_000,
    }),
    entry({ id: "e-visa", title: "Visa", entryType: "card", modified: 1_700_000_100_000 }),
    entry({ id: "e-notes", title: "Server notes", entryType: "notes", modified: 1_700_000_000_000 }),
    entry({
      id: "e-old",
      title: "Old forum",
      subtitle: "legacy",
      groupId: BIN_GROUP_ID,
      inRecycleBin: true,
      modified: 1_600_000_000_000,
    }),
  ];
}

const TYPE_OF: Partial<Record<Category, EntrySummary["entryType"]>> = {
  addresses: "address",
  cards: "card",
  notes: "notes",
  files: "files",
};

function inScope(e: EntrySummary, scope: Scope): boolean {
  if (scope.kind === "group") return e.groupId === scope.groupId;
  switch (scope.category) {
    case "deleted":
      return e.inRecycleBin;
    case "favorites":
      return !e.inRecycleBin && e.favorite;
    case "codes":
      return !e.inRecycleBin && e.hasTotp;
    case "passkeys":
      return !e.inRecycleBin && e.hasPasskey;
    case "security":
      return !e.inRecycleBin && e.flags.length > 0;
    case "all":
      return !e.inRecycleBin;
    default:
      return !e.inRecycleBin && e.entryType === TYPE_OF[scope.category];
  }
}

/** The list the backend would return for a query: scope filter, text search, sort. */
export function listEntries(entries: FixtureEntry[], query: EntryQuery): EntrySummary[] {
  const needle = query.search.trim().toLowerCase();
  const rows = entries
    .filter((e) => inScope(e.summary, query.scope))
    .filter(
      (e) =>
        needle === "" ||
        [e.summary.title, e.summary.subtitle, ...e.model.urls].some((s) => s.toLowerCase().includes(needle)),
    );
  rows.sort((a, b) =>
    query.sort === "title" ? a.summary.title.localeCompare(b.summary.title) : b.modified - a.modified,
  );
  return rows.map((e) => structuredClone(e.summary));
}

const CATEGORIES: Category[] = [
  "all",
  "favorites",
  "codes",
  "passkeys",
  "addresses",
  "cards",
  "notes",
  "files",
  "security",
  "deleted",
];

export function fixtureSidebar(entries: FixtureEntry[]): Sidebar_Serialize {
  const summaries = entries.map((e) => e.summary);
  const categories: CategoryCount_Serialize[] = CATEGORIES.map((category) => ({
    category,
    count: category === "security" ? null : summaries.filter((e) => inScope(e, { kind: "category", category })).length,
  }));
  const group = (id: string, name: string, isRecycleBin = false): GroupNode_Serialize => ({
    id,
    name,
    emoji: null,
    count: summaries.filter((e) => e.groupId === id).length,
    isRecycleBin,
    children: [],
  });
  return {
    categories,
    rootGroupId: ROOT_GROUP_ID,
    groups: [group(PERSONAL_GROUP_ID, "Personal"), group(WORK_GROUP_ID, "Work"), group(BIN_GROUP_ID, "Bin", true)],
  };
}

export function detailOf(e: FixtureEntry): EntryDetailDto_Serialize {
  const { summary, model } = e;
  return {
    id: summary.id,
    title: summary.title,
    entryType: summary.entryType,
    groupId: summary.groupId,
    groupName: summary.groupId === WORK_GROUP_ID ? "Work" : "Personal",
    favorite: summary.favorite,
    created: e.modified,
    modified: e.modified,
    username: model.username,
    url: model.urls[0] ?? null,
    urls: [...model.urls],
    uriMatchMode: model.uriMatchMode,
    hasPassword: model.password !== "",
    totpPeriod: summary.hasTotp ? 30 : null,
    totpDigits: summary.hasTotp ? 6 : null,
    passkey: null,
    address: null,
    card: null,
    hasNotes: model.notes !== "",
    customFields: [],
    attachments: [],
    historyCount: 0,
    inRecycleBin: summary.inRecycleBin,
  };
}

export function defaultSettings(): Settings_Serialize {
  return {
    language: "system",
    theme: "system",
    autoLockIdleTimeout: "thirtyMinutes",
    lockOnDeviceSleep: true,
    lockOnScreenLock: true,
    clipboardClearSeconds: 30,
    backupCount: 10,
    backupInterval: "everySave",
    launchAtLogin: false,
    showMenuBarExtra: true,
    shortcutLock: "CommandOrControl+Shift+L",
    shortcutShow: "",
    generator: { length: 20, lowercase: true, uppercase: true, digits: true, symbols: true },
    passkeyProxyEnabled: true,
    unlockTouchIdEnabled: false,
    unlockMacPasswordEnabled: false,
    unlockWithDeviceWindow: "eightHours",
    requireAccountPasswordWindow: "thirtyDays",
    secondaryFolder: null,
    updateCheckEnabled: true,
    vaultPath: null,
    recentVaults: [],
  };
}

export function sessionStatus(state: SessionStatus_Serialize["state"]): SessionStatus_Serialize {
  const hasVault = state !== "noVault";
  return {
    state,
    vaultPath: hasVault ? FIXTURE_VAULT_PATH : null,
    vaultName: hasVault ? "Test vault" : null,
    lockReason: null,
    fileStatus: hasVault ? { kind: "connected", size: 4096, modified: 1_700_000_000_000 } : { kind: "noVault" },
    save: { state: "idle", message: null },
  };
}
