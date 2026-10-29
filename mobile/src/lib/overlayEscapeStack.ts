// One Escape listener for every overlay on web/desktop. Each open overlay registers its close
// callback; a capture-phase window listener closes only the most recently opened one and swallows
// the key, so a palette over a sheet takes one Esc to dismiss the palette and a second for the
// sheet. Keystrokes that belong to something else are left alone: an IME composition (Esc cancels
// the candidate window), and any element marked `data-escape-skip` (the terminal, where Esc is a
// shell key).
interface Entry {
  id: number;
  close: () => void;
}

const stack: Entry[] = [];
let nextId = 1;
let listening = false;

export const ESCAPE_SKIP_ATTRIBUTE = "data-escape-skip";

export function isEscapeSkipped(target: EventTarget | null): boolean {
  const element = target as { closest?: (selector: string) => unknown } | null;
  return typeof element?.closest === "function" && element.closest(`[${ESCAPE_SKIP_ATTRIBUTE}]`) != null;
}

function onKeyDown(event: KeyboardEvent): void {
  if (event.key !== "Escape" || event.defaultPrevented) return;
  if (event.isComposing || event.keyCode === 229) return;
  if (isEscapeSkipped(event.target)) return;
  const top = stack[stack.length - 1];
  if (!top) return;
  event.preventDefault();
  event.stopPropagation();
  top.close();
}

function ensureListener(): void {
  if (listening || typeof window === "undefined") return;
  window.addEventListener("keydown", onKeyDown, true);
  listening = true;
}

function releaseListener(): void {
  if (!listening || stack.length > 0 || typeof window === "undefined") return;
  window.removeEventListener("keydown", onKeyDown, true);
  listening = false;
}

/** Push an overlay on top of the stack; the returned function removes it. */
export function registerOverlay(close: () => void): () => void {
  const entry: Entry = { id: nextId++, close };
  stack.push(entry);
  ensureListener();
  return () => {
    const index = stack.indexOf(entry);
    if (index >= 0) stack.splice(index, 1);
    releaseListener();
  };
}

export function openOverlayCount(): number {
  return stack.length;
}
