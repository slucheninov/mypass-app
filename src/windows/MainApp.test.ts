import { screen, within } from "@testing-library/svelte";
import { it, expect } from "vitest";
import { renderMainApp } from "../test/mainApp";

it.each(["locked", "noVault"] as const)("shows the pairing code sheet over the %s screen", async (session) => {
  const { fake } = await renderMainApp({ session });
  await screen.findByTestId("unlock-screen");
  await fake.emit("pairingChallengeEvent", {
    code: "123456",
    browser: "Chrome",
    expiresAt: Date.now() + 60_000,
  });
  const dialog = await screen.findByRole("dialog", { name: "Pairing code" });
  expect(within(dialog).getByText("123 456")).toBeTruthy();
  expect(screen.getByTestId("unlock-screen")).toBeTruthy();
});
