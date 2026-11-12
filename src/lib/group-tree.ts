import type { GroupNode_Serialize } from "./bindings";

/** One visible row of the group tree. */
export interface GroupRow {
  node: GroupNode_Serialize;
  /** 1 for the top level (ARIA `aria-level`). */
  level: number;
  /** Has children that are listed (the recycle bin is never listed). */
  expandable: boolean;
  expanded: boolean;
}

/** The children that are shown: the recycle bin is its own category, never a group. */
export function listedChildren(node: { children: GroupNode_Serialize[] }): GroupNode_Serialize[] {
  return node.children.filter((c) => !c.isRecycleBin);
}

/** The rows to draw, depth first. Groups are expanded unless `collapsed[id]` is set. */
export function visibleGroups(
  nodes: GroupNode_Serialize[],
  collapsed: Record<string, boolean>,
  level = 1,
): GroupRow[] {
  const rows: GroupRow[] = [];
  for (const node of nodes) {
    if (node.isRecycleBin) continue;
    const children = listedChildren(node);
    const expanded = children.length > 0 && !collapsed[node.id];
    rows.push({ node, level, expandable: children.length > 0, expanded });
    if (expanded) rows.push(...visibleGroups(children, collapsed, level + 1));
  }
  return rows;
}

export function findGroup(nodes: GroupNode_Serialize[], id: string): GroupNode_Serialize | null {
  for (const node of nodes) {
    if (node.id === id) return node;
    const inside = findGroup(node.children, id);
    if (inside) return inside;
  }
  return null;
}

/** True when `id` is `ancestorId` itself or lies inside its subtree (a group cannot move there). */
export function isSelfOrDescendant(nodes: GroupNode_Serialize[], ancestorId: string, id: string): boolean {
  const ancestor = findGroup(nodes, ancestorId);
  return ancestor !== null && (ancestor.id === id || findGroup(ancestor.children, id) !== null);
}
