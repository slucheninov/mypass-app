// A short, readable location for a vault file: the cloud provider and folder, `~` for the home
// folder, or the last two folders. The screens show it under the vault name and keep the full
// path in the tooltip and for screen readers. Path components are compared whole and without
// case, never as a substring of the path, so a folder named "My Dropbox notes" is not Dropbox.

/** Folders of the vault file's directory, below the provider root. */
type Cloud = { provider: string; below: string[] };

const same = (a: string | undefined, b: string): boolean => a !== undefined && a.toLowerCase() === b.toLowerCase();

/** Names of the File Provider folders in `~/Library/CloudStorage`, by their prefix before `-`. */
function cloudStorageProvider(folder: string): string {
  if (/^GoogleDrive-.+$/i.test(folder)) return "Google Drive";
  const name = folder.split("-")[0] || folder;
  switch (name.toLowerCase()) {
    case "onedrive":
      return "OneDrive";
    case "dropbox":
      return "Dropbox";
    case "box":
      return "Box";
    default:
      return name;
  }
}

/** The cloud provider a home-relative folder list (`rel`) is in, if any. */
function cloudInHome(rel: string[]): Cloud | null {
  if (same(rel[0], "Library") && same(rel[1], "Mobile Documents") && rel[2] !== undefined) {
    // iCloud Drive itself, or an app's container (shown inside iCloud Drive); a container keeps
    // its files in `Documents`.
    const skip = !same(rel[2], "com~apple~CloudDocs") && same(rel[3], "Documents") ? 4 : 3;
    return { provider: "iCloud Drive", below: rel.slice(skip) };
  }
  if (same(rel[0], "Library") && same(rel[1], "CloudStorage") && rel[2] !== undefined) {
    const provider = cloudStorageProvider(rel[2]);
    const drive = provider === "Google Drive" && (same(rel[3], "My Drive") || same(rel[3], "Shared drives"));
    return { provider, below: rel.slice(drive ? 4 : 3) };
  }
  const top = rel[0];
  if (top === undefined) return null;
  if (same(top, "iCloudDrive")) return { provider: "iCloud Drive", below: rel.slice(1) };
  if (/^onedrive( - .+)?$/i.test(top)) return { provider: "OneDrive", below: rel.slice(1) };
  if (/^dropbox( \(.+\))?$/i.test(top)) return { provider: "Dropbox", below: rel.slice(1) };
  if (same(top, "Box")) return { provider: "Box", below: rel.slice(1) };
  return null;
}

function cloudLabel({ provider, below }: Cloud): string {
  if (below.length === 0) return provider;
  if (below.length === 1) return `${provider}/${below[0]}`;
  return `${provider}/…/${below[below.length - 1]}`;
}

/** `…` and the last two folders. */
function tail(folders: string[], sep: string): string {
  return `…${sep}${folders.slice(-2).join(sep)}`;
}

/** Recognize the root before counting folder components. Windows canonical paths can be verbatim. */
function parsePath(input: string) {
  const path = input.replace(/^\\\\\?\\UNC\\/i, "\\\\").replace(/^\\\\\?\\(?=[A-Za-z]:)/, "");
  const network = path.match(/^(\\\\|\/\/)([^\\/]+)[\\/]([^\\/]+)(?:[\\/]|$)/);
  const drive = path.match(/^([A-Za-z]:)(?:[\\/]|$)/);
  const windows = !!(network || drive || path.startsWith("\\"));
  const sep = network?.[1] === "//" ? "/" : windows ? "\\" : "/";
  const root = network
    ? `${network[1]}${network[2]}${sep}${network[3]}`
    : drive ? `${drive[1]}\\` : path.startsWith("/") ? "/" : path.startsWith("\\") ? "\\" : "";
  const rest = path.slice(network?.[0].length ?? drive?.[0].length ?? root.length);
  const parts = rest.split(/[\\/]+/).filter(Boolean);
  return { root, parts, sep, windows, network: !!network };
}

/** Short location of a vault file, relative to the current user's home when applicable. */
export function shortVaultLocation(path: string, homeDir: string | null = null): string {
  const { root, parts, sep, windows, network } = parsePath(path);
  const folders = parts.slice(0, -1);

  // The legacy Google Drive mount has a fixed root and directory layout.
  if (root === "/" && same(folders[0], "Volumes") && same(folders[1], "GoogleDrive")
    && (same(folders[2], "My Drive") || same(folders[2], "Shared drives"))) {
    return cloudLabel({ provider: "Google Drive", below: folders.slice(3) });
  }

  if (network) {
    if (folders.length === 0) return root;
    return `${root}${sep}${folders.length > 2 ? tail(folders, sep) : folders.join(sep)}`;
  }

  if (homeDir && !folders.includes("..")) {
    const home = parsePath(homeDir);
    const equal = (a: string | undefined, b: string) => windows ? same(a, b) : a === b;
    if (root && equal(root.replaceAll("\\", "/"), home.root.replaceAll("\\", "/"))
      && home.parts.every((part, i) => equal(folders[i], part))) {
      const rel = folders.slice(home.parts.length);
      const cloud = cloudInHome(rel);
      if (cloud) return cloudLabel(cloud);
      return rel.length <= 2 ? ["~", ...rel].join(sep) : tail(rel, sep);
    }
  }

  if (folders.length > 2) return tail(folders, sep);
  return `${root}${folders.join(sep)}`;
}
