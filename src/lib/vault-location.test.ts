import { describe, expect, it } from "vitest";
import { shortVaultLocation } from "./vault-location";

const MAC = "/Users/someone";
const WIN = "C:\\Users\\someone";

describe("shortVaultLocation", () => {
  it.each([
    // iCloud Drive
    [`${MAC}/Library/Mobile Documents/com~apple~CloudDocs/a.kdbx`, "iCloud Drive"],
    [`${MAC}/Library/Mobile Documents/com~apple~CloudDocs/Vaults/a.kdbx`, "iCloud Drive/Vaults"],
    [`${MAC}/Library/Mobile Documents/com~apple~CloudDocs/Work/Team/Vaults/a.kdbx`, "iCloud Drive/…/Vaults"],
    [`${MAC}/Library/Mobile Documents/iCloud~com~example~app/Documents/a.kdbx`, "iCloud Drive"],
    [`${MAC}/Library/Mobile Documents/iCloud~com~example~app/Documents/Shared/a.kdbx`, "iCloud Drive/Shared"],
    [`${MAC}/library/mobile documents/COM~APPLE~CLOUDDOCS/Vaults/a.kdbx`, "iCloud Drive/Vaults"],
    [`${WIN}\\iCloudDrive\\a.kdbx`, "iCloud Drive"],
    [`${WIN}\\iCloudDrive\\Vaults\\a.kdbx`, "iCloud Drive/Vaults"],
    // Google Drive
    [`${MAC}/Library/CloudStorage/GoogleDrive-me@example.com/My Drive/a.kdbx`, "Google Drive"],
    [`${MAC}/Library/CloudStorage/GoogleDrive-me@example.com/My Drive/Vaults/a.kdbx`, "Google Drive/Vaults"],
    [`${MAC}/Library/CloudStorage/GoogleDrive-me@example.com/My Drive/A/B/Vaults/a.kdbx`, "Google Drive/…/Vaults"],
    [`${MAC}/Library/CloudStorage/GoogleDrive-me@example.com/Shared drives/Team/a.kdbx`, "Google Drive/Team"],
    // OneDrive
    [`${MAC}/Library/CloudStorage/OneDrive-Personal/a.kdbx`, "OneDrive"],
    [`${MAC}/Library/CloudStorage/OneDrive-Contoso/Vaults/a.kdbx`, "OneDrive/Vaults"],
    [`${WIN}\\OneDrive\\Vaults\\a.kdbx`, "OneDrive/Vaults"],
    [`${WIN}\\OneDrive - Contoso\\Docs\\Team\\Vaults\\a.kdbx`, "OneDrive/…/Vaults"],
    // Dropbox
    [`${MAC}/Library/CloudStorage/Dropbox/a.kdbx`, "Dropbox"],
    [`${MAC}/Library/CloudStorage/Dropbox-Personal/Vaults/a.kdbx`, "Dropbox/Vaults"],
    [`${MAC}/Dropbox/Vaults/a.kdbx`, "Dropbox/Vaults"],
    [`${WIN}\\Dropbox\\a.kdbx`, "Dropbox"],
    // Box
    [`${MAC}/Library/CloudStorage/Box-Box/Vaults/a.kdbx`, "Box/Vaults"],
    [`${MAC}/Box/a.kdbx`, "Box"],
    // Any other File Provider folder is named after the part before the account.
    [`${MAC}/Library/CloudStorage/pCloud-me@example.com/Vaults/a.kdbx`, "pCloud/Vaults"],
    [`${MAC}/Library/CloudStorage/SomeDrive/a.kdbx`, "SomeDrive"],
  ])("cloud folder %s shows %s", (path, expected) => {
    expect(shortVaultLocation(path, path.startsWith(MAC) ? MAC : WIN)).toBe(expected);
  });

  it.each([
    // Under the home folder: `~` while short, then the last two folders.
    [`${MAC}/a.kdbx`, "~"],
    [`${MAC}/Documents/a.kdbx`, "~/Documents"],
    [`${MAC}/Documents/Vaults/a.kdbx`, "~/Documents/Vaults"],
    [`${MAC}/Documents/Private/Vaults/a.kdbx`, "…/Private/Vaults"],
    [`${WIN}\\Documents\\a.kdbx`, "~\\Documents"],
    [`${WIN}\\Documents\\Private\\Vaults\\a.kdbx`, "…\\Private\\Vaults"],
    // Elsewhere: the full folder while it has at most two components.
    ["/a.kdbx", "/"],
    ["/Volumes/USB/a.kdbx", "/Volumes/USB"],
    ["/Volumes/USB/Backups/Vaults/a.kdbx", "…/Backups/Vaults"],
    ["D:\\a.kdbx", "D:\\"],
    ["D:\\Vaults\\a.kdbx", "D:\\Vaults"],
    ["D:\\Backups\\Old\\Vaults\\a.kdbx", "…\\Old\\Vaults"],
    ["d:/Vaults/a.kdbx", "d:\\Vaults"],
    ["\\\\server\\share\\a.kdbx", "\\\\server\\share"],
    ["\\\\server\\share\\Team\\Vaults\\a.kdbx", "\\\\server\\share\\Team\\Vaults"],
    ["a.kdbx", ""],
  ])("local folder %s shows %s", (path, expected) => {
    expect(shortVaultLocation(path, path.startsWith(MAC) ? MAC : WIN)).toBe(expected);
  });

  it.each([
    [String.raw`\\server\share\Team\Backups\Vaults\a.kdbx`, String.raw`\\server\share\…\Backups\Vaults`],
    ["//server/share/Team/Vaults/a.kdbx", "//server/share/Team/Vaults"],
    ["/Users/other/Documents/a.kdbx", "…/other/Documents"],
    ["/Users/someone-else/a.kdbx", "/Users/someone-else"],
    ["/Volumes/GoogleDriveBackup/My Drive/Vaults/a.kdbx", "…/My Drive/Vaults"],
    ["/Volumes/GoogleDrive/My Drive/Vaults/a.kdbx", "Google Drive/Vaults"],
  ])("formats %s against the actual home directory", (path, expected) => {
    expect(shortVaultLocation(path, MAC)).toBe(expected);
  });

  it("handles Windows canonical paths and network homes", () => {
    expect(shortVaultLocation(String.raw`\\?\C:\Users\someone\Vaults\a.kdbx`, WIN))
      .toBe(String.raw`~\Vaults`);
    expect(shortVaultLocation(String.raw`\\?\UNC\server\share\Team\Vaults\a.kdbx`))
      .toBe(String.raw`\\server\share\Team\Vaults`);
    expect(shortVaultLocation("//server/share/Team/Vaults/a.kdbx", String.raw`\\server\share\Team`))
      .toBe("//server/share/Team/Vaults");
  });

  it("matches whole components only", () => {
    expect(shortVaultLocation(`${MAC}/Documents/My Dropbox notes/Vaults/a.kdbx`, MAC)).toBe("…/My Dropbox notes/Vaults");
    expect(shortVaultLocation(`${MAC}/Projects/Library/CloudStorage/Dropbox/a.kdbx`, MAC)).toBe("…/CloudStorage/Dropbox");
    expect(shortVaultLocation("/Volumes/Data/Dropbox/a.kdbx")).toBe("…/Data/Dropbox");
    expect(shortVaultLocation(`${WIN}\\Documents\\OneDrive\\a.kdbx`, WIN)).toBe("~\\Documents\\OneDrive");
  });
});
