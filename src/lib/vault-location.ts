// A short, readable location for a vault file: the cloud provider and folder, `~` for the home
// folder, or the last two folders. The screens show it under the vault name and keep the full
// path in the tooltip and for screen readers. Path components are compared whole and without
// case, never as a substring of the path, so a folder named "My Dropbox notes" is not Dropbox.

/** Folders of the vault file's directory, below the provider root. */
type Cloud = { provider: string; below: string[] };

const same = (a: string | undefined, b: string): boolean => a !== undefined && a.toLowerCase() === b.toLowerCase();

/** Names of the File Provider folders in `~/Library/CloudStorage`, by their prefix before `-`. */
function cloudStorageProvider(folder: string): string {
  const name = folder.split("-")[0] || folder;
  switch (name.toLowerCase()) {
    case "googledrive":
      return "Google Drive";
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
  if (same(top, "Google Drive")) return { provider: "Google Drive", below: rel.slice(1) };
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

/**
 * The short location of a vault file at `path` (macOS or Windows, `/` or `\`): `iCloud Drive`,
 * `Google Drive/Vaults`, `OneDrive/…/Vaults`, `~/Documents`, `D:\Vaults` or `…/Backups/Vaults`.
 * An empty string for a bare file name.
 */
export function shortVaultLocation(path: string): string {
  const unc = path.startsWith("\\\\");
  const drive = /^[A-Za-z]:([\\/]|$)/.test(path);
  const windows = unc || drive || (path.includes("\\") && !path.includes("/"));
  const sep = windows ? "\\" : "/";
  const parts = path.split(/[\\/]+/).filter(part => part !== "");
  const folders = parts.slice(0, -1);
  if (folders.length === 0) {
    if (drive) return `${path.slice(0, 2)}\\`;
    return path.startsWith("/") ? "/" : "";
  }

  // `G:\My Drive\…` (Google Drive for desktop on Windows) and `/Volumes/GoogleDrive/My Drive/…`.
  const myDrive = drive ? 1 : same(folders[0], "Volumes") && /^googledrive/i.test(folders[1] ?? "") ? 2 : -1;
  if (myDrive > 0 && (same(folders[myDrive], "My Drive") || same(folders[myDrive], "Shared drives"))) {
    return cloudLabel({ provider: "Google Drive", below: folders.slice(myDrive + 1) });
  }

  // The home folder: `/Users/<name>` on macOS, `<drive>:\Users\<name>` on Windows.
  const users = drive ? 1 : 0;
  const home = same(folders[users], "Users") && folders.length > users + 1 ? users + 2 : -1;
  if (home > 0) {
    const rel = folders.slice(home);
    const cloud = cloudInHome(rel);
    if (cloud) return cloudLabel(cloud);
    return rel.length <= 2 ? ["~", ...rel].join(sep) : tail(rel, sep);
  }

  if (folders.length > 2) return tail(folders, sep);
  if (unc) return `\\\\${folders.join("\\")}`;
  if (drive) return folders.length === 1 ? `${folders[0]}\\` : folders.join("\\");
  return `/${folders.join("/")}`;
}
