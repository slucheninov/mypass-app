import type { IconName } from "./icons";

export type MenuItem = { id: string; label: string; icon?: IconName; disabled?: boolean; danger?: boolean };
