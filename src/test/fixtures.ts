// In-memory vault content for the fake backend and for store tests. The fixture login carries a
// password (`FIXTURE_LOGIN_PASSWORD`) so tests can prove it never reaches a store.
import type {
  AddressDto,
  AttachmentMeta,
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
export const BANKING_GROUP_ID = "group-banking";

export interface FixtureEntry {
  summary: EntrySummary;
  model: EntryEditModel_Serialize;
  modified: number;
}

export interface EntrySpec {
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
  username?: string;
  notes?: string;
  address?: AddressDto;
  attachments?: AttachmentMeta[];
  hasPasskey?: boolean;
  modified: number;
}

/** One entry of the fake vault; the summary and the edit model are kept consistent. */
export function makeEntry(spec: EntrySpec): FixtureEntry {
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
      hasPasskey: spec.hasPasskey ?? false,
      attachmentCount: spec.attachments?.length ?? 0,
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
      username: spec.username ?? spec.subtitle ?? "",
      password: spec.password ?? "",
      urls: spec.urls ?? [],
      uriMatchMode: "domain",
      totp: "",
      notes: spec.notes ?? "",
      address: spec.address ?? null,
      card: null,
      customFields: [],
      attachments: spec.attachments ?? [],
      attachmentOps: [],
    },
  };
}

export function fixtureEntries(): FixtureEntry[] {
  return [
    makeEntry({
      id: "e-github",
      title: "GitHub",
      subtitle: "octo",
      favorite: true,
      hasTotp: true,
      password: FIXTURE_LOGIN_PASSWORD,
      urls: ["https://github.com"],
      modified: 1_700_000_300_000,
    }),
    makeEntry({
      id: "e-gitlab",
      title: "GitLab",
      subtitle: "octo-lab",
      groupId: WORK_GROUP_ID,
      password: "gitlab-secret",
      urls: ["https://gitlab.com"],
      flags: ["weak"],
      modified: 1_700_000_200_000,
    }),
    makeEntry({ id: "e-visa", title: "Visa", entryType: "card", modified: 1_700_000_100_000 }),
    makeEntry({ id: "e-notes", title: "Server notes", entryType: "notes", modified: 1_700_000_000_000 }),
    makeEntry({
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

function groupById(nodes: GroupNode_Serialize[], id: string): GroupNode_Serialize | null {
  for (const node of nodes) {
    if (node.id === id) return node;
    const inside = groupById(node.children, id);
    if (inside) return inside;
  }
  return null;
}

function subtreeOf(node: GroupNode_Serialize): string[] {
  return [node.id, ...node.children.flatMap(subtreeOf)];
}

/**
 * The one search predicate of the backend (`queries::entry_matches`, in-app flavour): a
 * case-insensitive substring over title, username, every URL, notes (the note body is the same
 * field here), the group name, the 14 address fields of ADDRESS entries only, and attachment
 * names. Never passwords, TOTP, card data or custom values.
 */
export function entryMatches(e: FixtureEntry, groupName: string, search: string): boolean {
  const needle = search.toLowerCase();
  if (needle === "") return true;
  const hit = (text: string) => text.toLowerCase().includes(needle);
  const { model } = e;
  if (hit(model.title) || hit(model.username) || hit(model.notes) || hit(groupName)) return true;
  if (model.urls.some(hit)) return true;
  if (e.summary.entryType === "address" && model.address && Object.values(model.address).some((v) => hit(v))) {
    return true;
  }
  return model.attachments.some((a) => hit(a.name));
}

/**
 * The list the backend would return for a query (`queries::list_entries`): a category scope
 * searches inside the category; a group scope covers the group's subtree, but with a search it
 * covers the whole vault (never the bin unless the group is in the bin); sorted by lower-cased
 * title or by newest modification.
 */
export function listEntries(
  entries: FixtureEntry[],
  query: EntryQuery,
  groups: GroupNode_Serialize[] = fixtureGroups(),
): EntrySummary[] {
  const searching = query.search !== "";
  const nameOf = (id: string) => groupById(groups, id)?.name ?? "";
  let scoped: FixtureEntry[];
  if (query.scope.kind === "group") {
    const target = groupById(groups, query.scope.groupId);
    if (!target) return [];
    const bin = groups.find((g) => g.isRecycleBin);
    const inBin = bin ? subtreeOf(bin).includes(target.id) : false;
    const subtree = subtreeOf(target);
    scoped = entries.filter(
      (e) => (inBin || !e.summary.inRecycleBin) && (searching || subtree.includes(e.summary.groupId)),
    );
  } else {
    scoped = entries.filter((e) => inScope(e.summary, query.scope));
  }
  const rows = scoped.filter((e) => entryMatches(e, nameOf(e.summary.groupId), query.search));
  const key = (e: FixtureEntry) => e.summary.title.toLowerCase();
  rows.sort((a, b) => (query.sort === "title" ? (key(a) < key(b) ? -1 : key(a) > key(b) ? 1 : 0) : b.modified - a.modified));
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

/** The group tree below the root: Personal (with a nested Banking), Work, and the recycle bin last. Counts are filled by `fixtureSidebar`. */
export function fixtureGroups(): GroupNode_Serialize[] {
  const group = (
    id: string,
    name: string,
    emoji: string | null,
    children: GroupNode_Serialize[] = [],
    isRecycleBin = false,
  ): GroupNode_Serialize => ({ id, name, emoji, count: 0, isRecycleBin, children });
  return [
    group(PERSONAL_GROUP_ID, "Personal", null, [group(BANKING_GROUP_ID, "Banking", "\u{1F3E6}")]),
    group(WORK_GROUP_ID, "Work", "\u{1F4BC}"),
    group(BIN_GROUP_ID, "Bin", null, [], true),
  ];
}

/** A copy of the tree with every `count` set to the entries in that group's whole subtree. */
function counted(nodes: GroupNode_Serialize[], summaries: EntrySummary[]): { nodes: GroupNode_Serialize[]; total: number } {
  let total = 0;
  const out = nodes.map((node) => {
    const inner = counted(node.children, summaries);
    const own = summaries.filter((e) => e.groupId === node.id).length;
    total += own + inner.total;
    return { ...node, count: own + inner.total, children: inner.nodes };
  });
  return { nodes: out, total };
}

export function fixtureSidebar(
  entries: FixtureEntry[],
  groups: GroupNode_Serialize[] = fixtureGroups(),
  recycleBinEnabled = true,
): Sidebar_Serialize {
  const summaries = entries.map((e) => e.summary);
  const categories: CategoryCount_Serialize[] = CATEGORIES.map((category) => ({
    category,
    count: category === "security" ? null : summaries.filter((e) => inScope(e, { kind: "category", category })).length,
  }));
  return { categories, rootGroupId: ROOT_GROUP_ID, groups: counted(groups, summaries).nodes, recycleBinEnabled };
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
