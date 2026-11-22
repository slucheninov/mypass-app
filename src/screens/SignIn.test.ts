// The social sign-in method in the app: the detail row with its last use, the editor select, the
// list badge of a sign-in-only login and the "Remember social sign-in" setting.
import { screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";
import { formatDate } from "../lib/i18n/format";
import { setLanguage, t } from "../lib/i18n/index.svelte";
import { navigation } from "../lib/stores/navigation.svelte";
import { backendWith, openEdit, user } from "../test/editorHelpers";
import type { EntrySpec } from "../test/fixtures";
import { calls, renderMainApp } from "../test/mainApp";
import { renderSettingsApp } from "../test/settingsApp";

afterEach(() => setLanguage("en"));

const LAST_USED = Date.UTC(2026, 10, 20, 9, 30);

const SIGN_IN_ONLY: EntrySpec = {
  id: "e-shop",
  title: "shop.example",
  urls: ["https://shop.example"],
  subtitle: "",
  username: "",
  modified: 1_700_000_400_000,
  signIn: { provider: "google", lastUsed: LAST_USED },
};

async function openDetail(spec: EntrySpec) {
  const fake = backendWith([spec]);
  await renderMainApp(fake);
  await screen.findByRole("listbox");
  navigation.select(spec.id);
  await screen.findByRole("heading", { name: spec.title });
  return fake;
}

const pane = () => screen.getByRole("region", { name: t("Details") });

describe("sign-in method", () => {
  it("detail_shows_the_provider_and_the_last_use", async () => {
    await openDetail(SIGN_IN_ONLY);
    const row = within(pane()).getByRole("group", { name: "Sign-in method" });
    expect(row.textContent).toContain("Google");
    expect(row.textContent).toContain(`Last used ${formatDate(LAST_USED)}`);
    // A sign-in-only login has no password row.
    expect(within(pane()).queryByRole("group", { name: t("Password") })).toBeNull();
  });

  it("detail_without_a_recorded_use_shows_only_the_provider", async () => {
    await openDetail({ ...SIGN_IN_ONLY, signIn: { provider: "github" } });
    const row = within(pane()).getByRole("group", { name: "Sign-in method" });
    expect(row.textContent).toContain("GitHub");
    expect(row.textContent).not.toContain("Last used");
  });

  it("detail_of_a_login_without_a_method_has_no_sign_in_row", async () => {
    await openDetail({ id: "e-plain", title: "Plain", username: "me", password: "pw", modified: 1 });
    expect(within(pane()).queryByRole("group", { name: "Sign-in method" })).toBeNull();
  });

  it("detail_is_localized", async () => {
    await openDetail(SIGN_IN_ONLY);
    setLanguage("de");
    const row = await within(pane()).findByRole("group", { name: "Anmeldemethode" });
    expect(row.textContent).toContain("Google");
    expect(row.textContent).toContain("Zuletzt verwendet");
  });

  it("sign_in_only_login_shows_a_provider_badge_in_the_list", async () => {
    const fake = backendWith([SIGN_IN_ONLY]);
    await renderMainApp(fake);
    const list = await screen.findByRole("listbox");
    const badge = await within(list).findByTestId("sign-in-badge");
    expect(badge.textContent).toContain("Sign in with Google");
  });

  it("login_with_a_user_name_keeps_the_subtitle_and_adds_an_icon_badge", async () => {
    const fake = backendWith([{ ...SIGN_IN_ONLY, subtitle: "me@shop.example", username: "me@shop.example" }]);
    await renderMainApp(fake);
    const list = await screen.findByRole("listbox");
    await within(list).findByText("me@shop.example");
    expect(within(list).queryByTestId("sign-in-badge")).toBeNull();
    expect(within(list).getByRole("img", { name: "Sign in with Google" })).toBeTruthy();
  });

  it("editor_select_shows_the_stored_method_and_saves_a_change", async () => {
    const fake = backendWith([SIGN_IN_ONLY]);
    await openEdit(fake, SIGN_IN_ONLY.id);
    const select = screen.getByRole("combobox", { name: "Sign-in method" }) as HTMLSelectElement;
    expect(select.value).toBe("google");
    expect(Array.from(select.options).map((o) => o.textContent)).toEqual([
      "None",
      "Google",
      "Facebook",
      "Apple",
      "Microsoft",
      "GitHub",
    ]);
    await user().selectOptions(select, "apple");
    await user().click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "entrySave")).toHaveLength(1));
    expect(calls(fake, "entrySave")[0]!.args.model.signInEdit).toEqual({ op: "set", provider: "apple" });
  });

  it("editor_none_clears_the_method", async () => {
    const fake = backendWith([SIGN_IN_ONLY]);
    await openEdit(fake, SIGN_IN_ONLY.id);
    await user().selectOptions(screen.getByRole("combobox", { name: "Sign-in method" }), "");
    await user().click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "entrySave")).toHaveLength(1));
    expect(calls(fake, "entrySave")[0]!.args.model.signInEdit).toEqual({ op: "clear" });
  });

  it("editor_save_without_a_pick_leaves_the_method_unchanged", async () => {
    const fake = backendWith([SIGN_IN_ONLY]);
    await openEdit(fake, SIGN_IN_ONLY.id);
    // The browser records another method while the editor is open.
    fake.state.entries.find((e) => e.summary.id === SIGN_IN_ONLY.id)!.summary.signInProvider = "github";
    await user().click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(calls(fake, "entrySave")).toHaveLength(1));
    expect(calls(fake, "entrySave")[0]!.args.model.signInEdit).toEqual({ op: "unchanged" });
    expect(fake.state.entries.find((e) => e.summary.id === SIGN_IN_ONLY.id)!.summary.signInProvider).toBe("github");
  });
});

describe("Remember social sign-in setting", () => {
  const toggle = () => screen.getByRole("switch", { name: "Remember social sign-in" });

  it("browser_pane_toggle_sends_the_patch_and_follows_the_setting", async () => {
    const { fake } = await renderSettingsApp({}, { pane: "Browser" });
    expect(toggle().getAttribute("aria-checked")).toBe("true");
    expect(screen.getByText(/the browser extension remembers which one and highlights it next time/)).toBeTruthy();
    await user().click(toggle());
    expect(calls(fake, "settingsUpdate").map((c) => c.args.patch)).toEqual([{ rememberSocialSignIn: false }]);
    await waitFor(() => expect(toggle().getAttribute("aria-checked")).toBe("false"));
    await fake.emit("settingsChanged", { ...fake.state.settings, rememberSocialSignIn: true });
    await waitFor(() => expect(toggle().getAttribute("aria-checked")).toBe("true"));
  });
});
