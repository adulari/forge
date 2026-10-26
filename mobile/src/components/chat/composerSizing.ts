export const COMPOSER_LINE_HEIGHT = 22;
export const COMPOSER_MIN_HEIGHT = 44;
export const COMPOSER_MAX_LINES = 6;
export const COMPOSER_MAX_HEIGHT =
  COMPOSER_LINE_HEIGHT * COMPOSER_MAX_LINES + (COMPOSER_MIN_HEIGHT - COMPOSER_LINE_HEIGHT);

export function clampComposerHeight(contentHeight: number): number {
  return Math.min(COMPOSER_MAX_HEIGHT, Math.max(COMPOSER_MIN_HEIGHT, contentHeight));
}

export function composerUsesNativeMirror(platform: string): boolean {
  return platform !== "web";
}

export function nativeComposerMirrorText(text: string): string {
  return `${text}\u200b`;
}

export function composerScrollEnabled(platform: string, webHeight: number): boolean {
  return platform === "web" ? webHeight >= COMPOSER_MAX_HEIGHT : true;
}

export function composerInputVerticalPadding(platform: string): number {
  return platform === "web" ? 0 : (COMPOSER_MIN_HEIGHT - COMPOSER_LINE_HEIGHT) / 2;
}

interface OneLineProbe {
  contentWidth: number;
}

const oneLineProbes = new WeakMap<HTMLTextAreaElement, OneLineProbe>();
let measureContext: CanvasRenderingContext2D | null | undefined;

/**
 * True when `text` certainly fits on one line of `node` (a web textarea), decided WITHOUT touching
 * layout. The autosize effect used to collapse the textarea to 0px and read `scrollHeight` on every
 * keystroke, which forces a synchronous layout of the whole page (and a second one when the height
 * is restored). Single-line text is the overwhelmingly common case, and canvas text metrics need no
 * layout. The content width is cached from a ResizeObserver, so only the first call per node reads
 * layout. Anything uncertain (a newline, text within 4% of the line width, no canvas) answers false
 * and falls back to the exact DOM measure.
 */
export function textFitsOneLine(node: HTMLTextAreaElement, text: string): boolean {
  if (text.includes("\n")) return false;
  if (measureContext === undefined) {
    measureContext = typeof document === "undefined" ? null : document.createElement("canvas").getContext("2d");
  }
  const ctx = measureContext;
  if (!ctx) return false;
  const style = getComputedStyle(node);
  let probe = oneLineProbes.get(node);
  if (!probe) {
    const padding = parseFloat(style.paddingLeft) + parseFloat(style.paddingRight);
    const next: OneLineProbe = { contentWidth: node.clientWidth - (Number.isFinite(padding) ? padding : 0) };
    if (typeof ResizeObserver !== "undefined") {
      new ResizeObserver((entries) => {
        const width = entries[entries.length - 1]?.contentRect.width;
        if (width != null && width > 0) next.contentWidth = width;
      }).observe(node);
    }
    oneLineProbes.set(node, next);
    probe = next;
  }
  if (probe.contentWidth <= 0) return false;
  ctx.font = `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
  const spacing = parseFloat(style.letterSpacing);
  const width = ctx.measureText(text).width + (Number.isFinite(spacing) ? spacing * text.length : 0);
  return width < probe.contentWidth * 0.96;
}
