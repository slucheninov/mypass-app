import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createFakeBackend } from "../test/fakeBackend";
import { api, ApiFailure, call, errorDetail, errorMessage } from "./api";
import { commands, type ErrorCode } from "./bindings";
import { __resetCatalogsForTest, setLanguage, t } from "./i18n/index.svelte";
import en from "./i18n/locales/en.json";
import ru from "./i18n/locales/ru.json";
import uk from "./i18n/locales/uk.json";

beforeEach(() => {
  __resetCatalogsForTest();
  setLanguage("en");
});
afterEach(() => setLanguage("en"));

/** The `ErrorCode` union of bindings.ts, read from the generated source so a new code is noticed. */
function errorCodes(): ErrorCode[] {
  const text = readFileSync(resolve(process.cwd(), "src/lib/bindings.ts"), "utf8");
  const union = /export type ErrorCode = ([^;]+);/.exec(text)?.[1] ?? "";
  return [...union.matchAll(/"([A-Za-z]+)"/g)].map((m) => m[1] as ErrorCode);
}

describe("call", () => {
  it("call_unwraps_ok_and_throws_api_failure_on_error", async () => {
    const fake = createFakeBackend();
    fake.install();
    await expect(call(commands.vaultStatus())).resolves.toMatchObject({ state: "locked" });
    const failure = await call(commands.vaultUnlock("nope")).catch((e: unknown) => e);
    expect(failure).toBeInstanceOf(ApiFailure);
    expect((failure as ApiFailure).code).toBe("wrongPassword");
  });

  it("api_has_one_function_per_command", () => {
    expect(Object.keys(api).sort()).toEqual(Object.keys(commands).sort());
  });

  it("api_functions_forward_arguments_and_unwrap", async () => {
    const fake = createFakeBackend({ session: "unlocked" });
    fake.install();
    const rows = await api.entryList({ scope: { kind: "category", category: "all" }, search: "git", sort: "title" });
    expect(rows.map((r) => r.title)).toEqual(["GitHub", "GitLab"]);
    await expect(api.entryGet("missing")).rejects.toMatchObject({ code: "notFound" });
  });

  it("shortcut_conflicts_take_returns_the_bare_array", async () => {
    const fake = createFakeBackend();
    fake.handlers.shortcutConflictsTake = () => [{ kind: "lock", shortcut: "Ctrl+L" }];
    fake.install();
    await expect(api.shortcutConflictsTake()).resolves.toEqual([{ kind: "lock", shortcut: "Ctrl+L" }]);
  });
});

describe("errorMessage", () => {
  it("error_codes_map_to_localised_messages", () => {
    const codes = errorCodes();
    expect(codes.length).toBeGreaterThan(15);
    for (const code of codes) {
      const message = errorMessage(new ApiFailure(code, "RAW-BACKEND-TEXT"));
      expect(message, code).not.toBe("");
      expect(message, code).not.toBe(code);
      expect(message, code).not.toContain("RAW-BACKEND-TEXT");
    }
    expect(errorMessage(new ApiFailure("locked"))).toBe(t("Vault is locked"));
    expect(errorMessage(new ApiFailure("wrongPassword"))).toBe(t("Wrong master password"));
    expect(errorMessage(new ApiFailure("io", "x"))).toBe(t("Something went wrong"));
    expect(errorMessage(new Error("boom"))).toBe(t("Something went wrong"));
    expect(errorMessage("plain")).toBe(t("Something went wrong"));
  });

  it("detail_is_shown_only_for_io_and_internal", () => {
    expect(errorDetail(new ApiFailure("io", "disk full"))).toBe("disk full");
    expect(errorDetail(new ApiFailure("internal", "bug"))).toBe("bug");
    expect(errorDetail(new ApiFailure("io", ""))).toBeNull();
    expect(errorDetail(new ApiFailure("wrongPassword", "x"))).toBeNull();
    expect(errorDetail(new Error("x"))).toBeNull();
  });

  it("quitting_error_code_maps_to_localised_message", () => {
    expect(errorMessage(new ApiFailure("quitting"))).toBe(t("MyPass is quitting"));
    expect("MyPass is quitting" in uk).toBe(true);
    expect("MyPass is quitting" in ru).toBe(true);
    setLanguage("uk");
    expect(errorMessage(new ApiFailure("quitting"))).toBe(uk["MyPass is quitting" as keyof typeof uk]);
  });

  it("device_unlock_error_codes_map_to_exact_english_messages", () => {
    const expected: Record<string, string> = {
      cancelled: "Cancelled",
      lockout: "Too many attempts. Use your master password.",
      invalidated: "Device unlock was reset. Enter your master password.",
      notEnrolledOnDevice: "No fingerprint or device password is set up on this computer",
      deviceUnlockUnavailable: "Device unlock is not available on this computer",
    };
    for (const [code, text] of Object.entries(expected)) {
      expect(errorMessage(new ApiFailure(code as ErrorCode)), code).toBe(text);
    }
    expect("Cancelled" in uk && "Cancelled" in ru).toBe(true);
  });

  it("every_error_message_key_exists_in_all_catalogs", () => {
    for (const key of [
      "Vault is locked",
      "Wrong master password",
      "Vault not found",
      "Item not found",
      "Invalid input",
      "Limit reached",
      "The vault file looks damaged",
      "Not supported on this system",
      "The vault changed on disk",
      "Something went wrong",
      "Saving failed. MyPass will retry.",
      "Details",
      "Notifications",
      "A vault file already exists there",
      "Turn on at least one character type in the generator",
      "This site's password requirements could not be satisfied",
    ]) {
      expect(key in en && key in uk && key in ru, key).toBe(true);
    }
  });
});
