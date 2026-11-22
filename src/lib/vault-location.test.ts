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
    [`${MAC}/Google Drive/Vaults/a.kdbx`, "Google Drive/Vaults"],
    ["G:\\My Drive\\Vaults\\a.kdbx", "Google Drive/Vaults"],
    ["G:\\My Drive\\a.kdbx", "Google Drive"],
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
    expect(shortVaultLocation(path)).toBe(expected);
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
    ["\\\\server\\share\\Team\\Vaults\\a.kdbx", "…\\Team\\Vaults"],
    ["a.kdbx", ""],
  ])("local folder %s shows %s", (path, expected) => {
    expect(shortVaultLocation(path)).toBe(expected);
  });

  it("matches whole components only, never a part of the path", () => {
    expect(shortVaultLocation(`${MAC}/Documents/My Dropbox notes/Vaults/a.kdbx`)).toBe("…/My Dropbox notes/Vaults");
    expect(shortVaultLocation(`${MAC}/Projects/Library/CloudStorage/Dropbox/a.kdbx`)).toBe("…/CloudStorage/Dropbox");
    expect(shortVaultLocation("/Volumes/Data/Dropbox/a.kdbx")).toBe("…/Data/Dropbox");
    expect(shortVaultLocation(`${WIN}\\Documents\\OneDrive\\a.kdbx`)).toBe("~\\Documents\\OneDrive");
  });
});
