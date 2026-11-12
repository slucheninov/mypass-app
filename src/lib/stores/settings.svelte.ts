/** Applies the theme setting: `system` leaves the attribute off so `prefers-color-scheme` decides. */
export function applyTheme(theme: "system" | "light" | "dark"): void {
  const root = document.documentElement;
  if (theme === "system") delete root.dataset.theme;
  else root.dataset.theme = theme;
}
