// Mobile Safari (and any browser with an on-screen keyboard) shrinks the VISUAL viewport when the
// keyboard opens but leaves the layout viewport, and so a bottom-anchored sheet, where it was:
// the keyboard then sits on top of the field being typed into. The inset is how much of the
// layout viewport's bottom the visual viewport no longer covers.

export interface ViewportSnapshot {
  innerHeight: number;
  height: number;
  offsetTop: number;
  scale: number;
}

// Address-bar collapse and rubber-banding move the visual viewport by a few dozen pixels; the
// smallest on-screen keyboard is taller than this.
const MIN_KEYBOARD_PX = 120;
// A pinch-zoomed page also has a smaller visual viewport, and that is not a keyboard.
const MAX_ZOOM_SCALE = 1.05;

export function keyboardInset(viewport: ViewportSnapshot): number {
  if (viewport.scale > MAX_ZOOM_SCALE) return 0;
  const covered = viewport.innerHeight - viewport.height - viewport.offsetTop;
  return covered >= MIN_KEYBOARD_PX ? Math.round(covered) : 0;
}

/** Calls `onChange` with the inset now and on every change; returns the unsubscribe. A browser
 * without `visualViewport` reports 0 once and never again. */
export function watchKeyboardInset(onChange: (inset: number) => void): () => void {
  const vv = typeof window !== "undefined" ? window.visualViewport : null;
  if (!vv) {
    onChange(0);
    return () => undefined;
  }
  const report = () =>
    onChange(keyboardInset({ innerHeight: window.innerHeight, height: vv.height, offsetTop: vv.offsetTop, scale: vv.scale }));
  report();
  vv.addEventListener("resize", report);
  vv.addEventListener("scroll", report);
  return () => {
    vv.removeEventListener("resize", report);
    vv.removeEventListener("scroll", report);
  };
}
