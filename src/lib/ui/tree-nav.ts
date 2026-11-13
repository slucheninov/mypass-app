/**
 * Keyboard model of a flat ARIA tree (`role="tree"` holding `role="treeitem"` rows in document
 * order): Up/Down/Home/End move focus, Enter and Space activate the row, Right and Left open and
 * close a row that carries `aria-expanded` (`toggle` receives its `data-id`); Right on an open row
 * goes to its first child, Left on a leaf or closed row goes to its parent (by `aria-level`).
 */
export function treeNav(node: HTMLElement, options: { toggle?: (id: string, open: boolean) => void } = {}) {
  let toggle = options.toggle;

  function items(): HTMLElement[] {
    return Array.from(node.querySelectorAll<HTMLElement>('[role="treeitem"]:not([aria-disabled="true"])'));
  }

  function onkeydown(event: KeyboardEvent) {
    const current = event.target instanceof HTMLElement ? event.target.closest<HTMLElement>('[role="treeitem"]') : null;
    if (!current || event.altKey || event.ctrlKey || event.metaKey) return;
    const all = items();
    const at = all.indexOf(current);
    const expanded = current.getAttribute("aria-expanded");
    const id = current.dataset.id ?? "";
    if (event.key === "ArrowDown") all[Math.min(at + 1, all.length - 1)]?.focus();
    else if (event.key === "ArrowUp") all[Math.max(at - 1, 0)]?.focus();
    else if (event.key === "Home") all[0]?.focus();
    else if (event.key === "End") all[all.length - 1]?.focus();
    else if (event.key === "Enter" || event.key === " ") current.click();
    else if (event.key === "ArrowRight" && expanded === "false") toggle?.(id, true);
    else if (event.key === "ArrowRight" && expanded === "true") all[at + 1]?.focus();
    else if (event.key === "ArrowLeft" && expanded === "true") toggle?.(id, false);
    else if (event.key === "ArrowLeft") {
      // A leaf or collapsed row: up to its parent, the nearest earlier row one level shallower.
      const level = Number(current.getAttribute("aria-level") ?? "1");
      const parent = all.slice(0, at).reverse().find((el) => Number(el.getAttribute("aria-level") ?? "1") < level);
      if (!parent) return;
      parent.focus();
    } else return;
    event.preventDefault();
  }

  node.addEventListener("keydown", onkeydown);
  return {
    update(next: { toggle?: (id: string, open: boolean) => void }) {
      toggle = next.toggle;
    },
    destroy() {
      node.removeEventListener("keydown", onkeydown);
    },
  };
}
