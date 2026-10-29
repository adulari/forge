// Web-only selection and clipboard helpers for the terminal. On native the OS owns selection and
// copy, so every function degrades to "nothing selected".
/** True where a DOM exists to select in and listen on: the browser and the desktop webview. */
export const hasDom = typeof document !== "undefined" && typeof window !== "undefined" && "getSelection" in window;

function selection(): Selection | null {
  return hasDom ? window.getSelection() : null;
}

/** The text selected inside `root`, or "" when the selection is elsewhere or empty. */
export function selectedTextWithin(root: Node | null | undefined): string {
  const current = selection();
  if (!current || current.isCollapsed || !root) return "";
  const anchor = current.anchorNode;
  if (!anchor || !root.contains(anchor)) return "";
  return current.toString();
}

export function hasAnySelection(): boolean {
  const current = selection();
  return current != null && !current.isCollapsed && current.toString() !== "";
}

export function clearSelection(): void {
  selection()?.removeAllRanges();
}

export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    try {
      const area = document.createElement("textarea");
      area.value = text;
      area.style.position = "fixed";
      area.style.opacity = "0";
      document.body.appendChild(area);
      area.select();
      const ok = document.execCommand("copy");
      document.body.removeChild(area);
      return ok;
    } catch {
      return false;
    }
  }
}
