// The sidebar categories in the spec's order, with their icons and labels. Shared by the sidebar
// and the empty detail pane, so a category looks the same in both places.
import type { Category } from "./bindings";
import { t } from "./i18n/index.svelte";
import type { IconName } from "./ui/icons";

export const CATEGORIES: readonly { category: Category; icon: IconName }[] = [
  { category: "all", icon: "all" },
  { category: "favorites", icon: "star" },
  { category: "codes", icon: "shield-key" },
  { category: "passkeys", icon: "passkey" },
  { category: "addresses", icon: "address" },
  { category: "people", icon: "person" },
  { category: "cards", icon: "card" },
  { category: "notes", icon: "note" },
  { category: "files", icon: "paperclip" },
  { category: "security", icon: "security" },
  { category: "deleted", icon: "trash" },
];

/** The icon of `category`. */
export function categoryIcon(category: Category): IconName {
  return CATEGORIES.find((c) => c.category === category)?.icon ?? "all";
}

/** The translated label of `category`. */
export function categoryLabel(category: Category): string {
  switch (category) {
    case "all":
      return t("All");
    case "favorites":
      return t("Favorites");
    case "codes":
      return t("Codes");
    case "passkeys":
      return t("Passkeys");
    case "addresses":
      return t("Addresses");
    case "people":
      return t("People");
    case "cards":
      return t("Cards");
    case "notes":
      return t("Notes");
    case "files":
      return t("Files");
    case "security":
      return t("Security");
    case "deleted":
      return t("Deleted");
  }
}
