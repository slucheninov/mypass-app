import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { invoke } from "@tauri-apps/api/core";
import { describe, expect, it } from "vitest";
import { createFakeBackend } from "./fakeBackend";
import { BIN_GROUP_ID, WORK_GROUP_ID } from "./fixtures";
import { renderWithBackend } from "./render";
import MainApp from "../windows/MainApp.svelte";
import { commands } from "../lib/bindings";

describe("fake backend", () => {
  it("fake_backend_covers_every_binding_command", () => {
    // Parse the generated file instead of importing it, so a command the fake forgot cannot hide
    // behind the same object the fake was built from.
    const text = readFileSync(resolve(process.cwd(), "src/lib/bindings.ts"), "utf8");
    const block = /export const commands = \{([\s\S]*?)\n\};/.exec(text)?.[1] ?? "";
    const names = [...block.matchAll(/^\t([a-zA-Z0-9]+): /gm)].map((m) => m[1]).sort();
    expect(names.length).toBeGreaterThan(0);
    expect(Object.keys(createFakeBackend().handlers).sort()).toEqual(names);
    expect(Object.keys(commands).sort()).toEqual(names);
    // The fake derives the wire name with `snake()`; it must equal the string the binding invokes.
    const wire = new Map(
      block
        .split(/^\t(?=[a-zA-Z0-9]+: )/m)
        .map((chunk) => /^([a-zA-Z0-9]+): [\s\S]*?__TAURI_INVOKE(?:<.*?>)?\("([a-z0-9_]+)"/.exec(chunk))
        .filter((m) => m !== null)
        .map((m) => [m[1], m[2]] as const),
    );
    expect(wire.size).toBe(names.length);
    const snake = (n: string) => n.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`);
    for (const [name, wireName] of wire) expect(snake(name ?? ""), name).toBe(wireName);
  });

  it("fake_app_info_reports_device_unlock_flag_and_os", async () => {
    const fake = createFakeBackend({ os: "windows", deviceUnlockAvailable: true });
    renderWithBackend(MainApp, {}, fake);
    expect(await commands.appInfo()).toEqual({
      status: "ok",
      data: { version: expect.any(String), os: "windows", deviceUnlockAvailable: true },
    });
    // The main window's event bridge takes the startup hotkey conflicts when it mounts.
    expect(fake.calls.map((c) => c.command)).toEqual(["appInfo", "shortcutConflictsTake"]);
  });

  it("platform_keyboard_layout_is_null_unless_configured", async () => {
    renderWithBackend(MainApp, {}, createFakeBackend());
    expect(await commands.platformKeyboardLayout()).toEqual({ status: "ok", data: null });
    renderWithBackend(MainApp, {}, createFakeBackend({ keyboardLayout: "UK" }));
    expect(await commands.platformKeyboardLayout()).toEqual({ status: "ok", data: "UK" });
  });

  it("an unimplemented command fails loudly", async () => {
    renderWithBackend(MainApp, {}, createFakeBackend());
    await expect(invoke("plugin:never|called")).rejects.toThrow("NotImplementedInFake(plugin:never|called)");
  });

  it("the settings window commands: backups, login item status and clamped settings", async () => {
    const fake = createFakeBackend({ session: "unlocked" });
    renderWithBackend(MainApp, {}, fake);
    fake.state.backups = [{ id: "backup-1.kdbx", created: 1000, size: 10 }];
    expect(await commands.backupList()).toEqual({
      status: "ok",
      data: { items: fake.state.backups, lastFailure: null, count: 10, interval: "everySave" },
    });
    expect(await commands.settingsLoginItemStatus()).toEqual({ status: "ok", data: "disabled" });
    await commands.settingsUpdate({ launchAtLogin: true });
    expect(await commands.settingsLoginItemStatus()).toEqual({ status: "ok", data: "enabled" });
    const updated = await commands.settingsUpdate({ backupCount: 99, clipboardClearSeconds: 33 });
    expect(updated).toMatchObject({ status: "ok", data: { backupCount: 50, clipboardClearSeconds: 35 } });
    expect(await commands.appRevealBackups()).toEqual({ status: "ok", data: null });
  });

  it("an unknown command or an unhandled plugin call throws and names itself", async () => {
    renderWithBackend(MainApp, {}, createFakeBackend());
    await expect(invoke("no_such_command")).rejects.toThrow("NotImplementedInFake(unknown command no_such_command)");
    await expect(invoke("plugin:opener|open_url")).rejects.toThrow(
      "NotImplementedInFake(plugin:opener|open_url)",
    );
  });

  it("a plugin call works when the test opts in", async () => {
    const seen: unknown[] = [];
    const fake = createFakeBackend({
      plugins: { "plugin:opener|open_url": (p) => (seen.push(p), null) },
    });
    renderWithBackend(MainApp, {}, fake);
    await invoke("plugin:opener|open_url", { url: "u" });
    expect(seen).toEqual([{ url: "u" }]);
  });

  it("emit reaches listeners registered through the generated events", async () => {
    const fake = createFakeBackend();
    renderWithBackend(MainApp, {}, fake);
    const { events } = await import("../lib/bindings");
    const seen: unknown[] = [];
    const un = await events.vaultChanged.listen((e) => seen.push(e.payload));
    await fake.emit("vaultChanged", null);
    un();
    expect(seen).toEqual([null]);
  });

  it("fake_entry_delete_and_group_delete_match_the_backend", async () => {
    const fake = createFakeBackend({ session: "unlocked" });
    renderWithBackend(MainApp, {}, fake);
    const inBin = (id: string) => fake.state.entries.find((e) => e.summary.id === id)?.summary.inRecycleBin;
    // A live entry goes to the bin, an entry already there is purged.
    expect(await commands.entryDelete("e-github")).toEqual({ status: "ok", data: null });
    expect(inBin("e-github")).toBe(true);
    await commands.entryDelete("e-github");
    expect(inBin("e-github")).toBeUndefined();
    // With the bin disabled a delete purges at once.
    fake.state.recycleBinEnabled = false;
    await commands.entryDelete("e-gitlab");
    expect(inBin("e-gitlab")).toBeUndefined();
    expect(fake.state.entries.some((e) => e.summary.id === "e-gitlab")).toBe(false);
    // The recycle bin is not a group that can be deleted or moved: invalid input, like Rust.
    const bin = fake.state.groups.find((g) => g.isRecycleBin)?.id ?? "";
    expect(await commands.groupDelete(bin)).toMatchObject({ status: "error", error: { code: "invalidInput" } });
    expect(await commands.groupMove(bin, "")).toMatchObject({ status: "error", error: { code: "invalidInput" } });
  });

  it("fake_detail_commands_reveal_copy_totp_export_and_save_like_the_backend", async () => {
    const fake = createFakeBackend({ session: "unlocked" });
    renderWithBackend(MainApp, {}, fake);
    const ok = (r: { status: string }) => r.status === "ok";
    // Reveal and copy resolve every field kind; an unknown custom key is notFound.
    expect(await commands.entryReveal("e-visa", { kind: "cardCvv" })).toEqual({ status: "ok", data: "737" });
    expect(await commands.entryReveal("e-visa", { kind: "cardNumber" })).toEqual({ status: "ok", data: "4111111111111111" });
    expect(await commands.entryReveal("e-github", { kind: "totp" })).toEqual({ status: "ok", data: "123456" });
    expect(await commands.entryReveal("e-gitlab", { kind: "totp" })).toMatchObject({ status: "error", error: { code: "invalidInput" } });
    expect(await commands.entryCopy("e-github", { kind: "custom", key: "nope" })).toMatchObject({ status: "error", error: { code: "notFound" } });
    expect(ok(await commands.entryTotp("e-github"))).toBe(true);
    // The detail never carries a protected value, a card number or a CVV.
    const detail = await commands.entryGet("e-visa");
    expect(JSON.stringify(detail)).not.toContain("4111111111111111");
    expect(JSON.stringify(detail)).not.toContain("737");
    // Save replaces the editable content; restore takes an entry out of the bin.
    const model = fake.state.entries.find((e) => e.summary.id === "e-github")!.model;
    expect(await commands.entrySave({ ...structuredClone(model), urls: ["https://b.test"] })).toEqual({ status: "ok", data: "e-github" });
    expect(fake.state.entries.find((e) => e.summary.id === "e-github")!.model.urls).toEqual(["https://b.test"]);
    // Like `save_entry`: saving an entry with another group moves it, out of the bin too.
    const old = fake.state.entries.find((e) => e.summary.id === "e-old")!;
    expect(old.summary.inRecycleBin).toBe(true);
    expect(await commands.entrySave({ ...structuredClone(old.model), groupId: WORK_GROUP_ID })).toEqual({ status: "ok", data: "e-old" });
    expect(old.summary.inRecycleBin).toBe(false);
    old.summary.inRecycleBin = true;
    old.summary.groupId = BIN_GROUP_ID;
    old.model.groupId = BIN_GROUP_ID;
    expect(ok(await commands.entryRestore("e-old", null))).toBe(true);
    expect(fake.state.entries.find((e) => e.summary.id === "e-old")!.summary.inRecycleBin).toBe(false);
    // Export answers false when the (Rust-side) save dialog is cancelled.
    fake.state.entries[0]!.model.attachments = [{ name: "a.txt", size: 1 }];
    expect(await commands.attachmentExport("e-github", "a.txt")).toEqual({ status: "ok", data: true });
    fake.state.exportCancelled = true;
    expect(await commands.attachmentExport("e-github", "a.txt")).toEqual({ status: "ok", data: false });
    expect(await commands.attachmentExport("e-github", "b.txt")).toMatchObject({ status: "error", error: { code: "notFound" } });
  });
});
