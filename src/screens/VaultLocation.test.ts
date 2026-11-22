import { screen, within } from "@testing-library/svelte";
import { describe, expect, it } from "vitest";
import { renderMainApp } from "../test/mainApp";
import { renderTrayPanel } from "../test/trayApp";

const ICLOUD = "/Users/someone/Library/Mobile Documents/com~apple~CloudDocs/Family/Shared/Family.kdbx";
const DROPBOX = "/Users/someone/Dropbox/Vaults/Work.kdbx";

function location(root: ParentNode): HTMLElement {
  const node = root.querySelector<HTMLElement>(".location");
  if (!node) throw new Error("no vault location");
  return node;
}

describe("vault location", () => {
  it("the_unlock_card_shows_the_short_location_with_the_full_path_as_tooltip_and_screen_reader_text", async () => {
    await renderMainApp({ session: "locked" }, (f) => (f.state.session.vaultPath = ICLOUD));
    const card = (await screen.findByRole("heading", { name: "Family.kdbx" })).closest("section") as HTMLElement;
    const node = location(card);
    expect(node.getAttribute("title")).toBe(ICLOUD);
    const visible = node.querySelector('[aria-hidden="true"]');
    expect(visible?.textContent).toBe("iCloud Drive/…/Shared");
    expect(within(node).getByText(ICLOUD).className).toContain("sr-only");
  });

  it("the_tray_unlock_card_shows_the_short_location", async () => {
    await renderTrayPanel({ session: "locked" }, (f) => (f.state.session.vaultPath = DROPBOX));
    const node = location(document.body);
    expect(node.getAttribute("title")).toBe(DROPBOX);
    expect(node.querySelector('[aria-hidden="true"]')?.textContent).toBe("Dropbox/Vaults");
  });

  it("recent_vaults_show_the_short_location_and_keep_the_full_path_in_the_tooltip", async () => {
    await renderMainApp({ session: "noVault" }, (f) => {
      f.state.recents = [
        { path: ICLOUD, fileName: "Family.kdbx", exists: true },
        { path: DROPBOX, fileName: "Work.kdbx", exists: true },
      ];
    });
    const featured = await screen.findByRole("button", { name: /Family\.kdbx/ });
    expect(location(featured).getAttribute("title")).toBe(ICLOUD);
    const recents = screen.getByRole("list", { name: "Recent vaults" });
    const work = within(recents).getByRole("button", { name: /Work\.kdbx/ });
    expect(location(work).getAttribute("title")).toBe(DROPBOX);
    expect(location(work).querySelector('[aria-hidden="true"]')?.textContent).toBe("Dropbox/Vaults");
    // The accessible name carries the full path, not the shortened one.
    expect(work.textContent).toContain(DROPBOX);
  });
});
