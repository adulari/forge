// Terminal dock scrollback model — a bounded, incremental ANSI parser.
//
// Why hand-rolled rather than xterm.js: the dock renders react-native primitives on every
// surface the shell runs on (RN-web inside Tauri, RN-web in the browser), and xterm.js is a
// DOM-only canvas renderer — pulling it in would add a web-only dependency and a second
// rendering model for one 190px strip. What the dock actually needs is narrow: readable
// output with colour, a usable prompt line, and no raw escape bytes leaking into the view.
//
// So this is deliberately a *log* model, not a screen model: there is no cursor grid, no
// alternate screen buffer, no scroll regions. Full-screen TUIs (vim, htop) will not render
// correctly here — they are not the target; shell commands and their output are. Every escape
// sequence this file does not implement is CONSUMED and dropped, never printed, which is the
// one hard guarantee: unsupported input degrades to plain text, not to garbage.
//
// Pure + theme-free on purpose (no token imports) so it stays unit-testable — the renderer
// resolves `AnsiColor` to Machined tokens, see TerminalDock.tsx's `ansiPalette`.

/** A colour as the stream expressed it. Resolution to a real token happens at render time. */
export type AnsiColor =
  | { kind: "basic"; index: number } // 0-7 normal, 8-15 bright
  | { kind: "rgb"; r: number; g: number; b: number };

export interface AnsiStyle {
  fg: AnsiColor | null;
  bg: AnsiColor | null;
  bold: boolean;
  dim: boolean;
  italic: boolean;
  underline: boolean;
  inverse: boolean;
}

export interface AnsiSpan {
  text: string;
  style: AnsiStyle;
}

export interface AnsiLine {
  /** Monotonic, stable for the lifetime of a line — the render list keys off this so a
   * memoised row survives the re-render that every output chunk triggers. */
  key: number;
  spans: AnsiSpan[];
}

export const DEFAULT_ANSI_STYLE: AnsiStyle = {
  fg: null,
  bg: null,
  bold: false,
  dim: false,
  italic: false,
  underline: false,
  inverse: false,
};

const TAB_WIDTH = 8;
const ESC = "\x1b";

/** CSI parameter bytes 0x30-0x3F, intermediates 0x20-0x2F, final 0x40-0x7E. */
function isParamByte(ch: string): boolean {
  return ch >= "\x30" && ch <= "\x3f";
}
function isIntermediateByte(ch: string): boolean {
  return ch >= "\x20" && ch <= "\x2f";
}
function isFinalByte(ch: string): boolean {
  return ch >= "\x40" && ch <= "\x7e";
}

function parseParams(raw: string): number[] {
  if (raw === "") return [];
  // `38:2:…` (colon-separated sub-parameters) is as common as `38;2;…` in modern shells;
  // both flatten to the same list here.
  return raw.split(/[;:]/).map((part) => (part === "" ? 0 : parseInt(part, 10) || 0));
}

export class AnsiScrollback {
  private readonly maxLines: number;
  private done: AnsiLine[] = [];
  private current: AnsiSpan[] = [];
  /** Write column on the current line — `\r`, `\b` and erase sequences move it without
   * emitting text, which is what makes progress bars and prompt redraws readable. */
  private col = 0;
  private style: AnsiStyle = DEFAULT_ANSI_STYLE;
  /** Trailing bytes of an escape sequence split across two socket frames. */
  private partial = "";
  private nextKey = 0;

  constructor(maxLines = 2000) {
    this.maxLines = maxLines;
  }

  clear(): void {
    this.done = [];
    this.current = [];
    this.col = 0;
    this.partial = "";
  }

  /** Completed lines plus the in-progress one (always present, so the prompt row renders). */
  lines(): AnsiLine[] {
    return [...this.done, { key: this.nextKey, spans: this.current }];
  }

  write(chunk: string): void {
    const data = this.partial + chunk;
    this.partial = "";
    let text = "";
    let i = 0;

    const flush = () => {
      if (text) {
        this.writeText(text);
        text = "";
      }
    };

    while (i < data.length) {
      const ch = data[i];

      if (ch === ESC) {
        flush();
        const consumed = this.parseEscape(data, i);
        if (consumed < 0) {
          // Incomplete sequence at the end of the frame — carry it into the next one rather
          // than printing its bytes.
          this.partial = data.slice(i);
          return;
        }
        i += consumed;
        continue;
      }

      if (ch === "\n") {
        flush();
        this.newline();
        i += 1;
        continue;
      }
      if (ch === "\r") {
        flush();
        this.col = 0;
        i += 1;
        continue;
      }
      if (ch === "\b") {
        flush();
        this.col = Math.max(0, this.col - 1);
        i += 1;
        continue;
      }
      if (ch === "\t") {
        flush();
        this.writeText(" ".repeat(TAB_WIDTH - (this.col % TAB_WIDTH)));
        i += 1;
        continue;
      }
      // Remaining C0 controls and DEL carry no meaning in a log view — swallow them.
      if (ch < " " || ch === "\x7f") {
        i += 1;
        continue;
      }

      text += ch;
      i += 1;
    }

    flush();
  }

  // -------------------------------------------------------------------------
  // Escape sequences
  // -------------------------------------------------------------------------

  /** Returns how many characters the sequence at `start` consumed, or -1 if incomplete. */
  private parseEscape(data: string, start: number): number {
    const next = data[start + 1];
    if (next === undefined) return -1;

    if (next === "[") {
      let i = start + 2;
      while (i < data.length && isParamByte(data[i])) i += 1;
      const paramEnd = i;
      while (i < data.length && isIntermediateByte(data[i])) i += 1;
      if (i >= data.length) return -1;
      const final = data[i];
      if (!isFinalByte(final)) return i - start + 1; // malformed — drop it
      this.applyCsi(final, data.slice(start + 2, paramEnd));
      return i - start + 1;
    }

    // OSC (window title, hyperlinks) and the DCS/PM/APC family: string sequences ending at
    // BEL or ST. Nothing here is renderable, so they are consumed whole.
    if (next === "]" || next === "P" || next === "^" || next === "_") {
      for (let i = start + 2; i < data.length; i += 1) {
        if (data[i] === "\x07") return i - start + 1;
        if (data[i] === ESC && data[i + 1] === "\\") return i - start + 2;
        if (data[i] === ESC && data[i + 1] === undefined) return -1;
      }
      return -1;
    }

    // Charset designators (`ESC ( B`) — three bytes.
    if (next === "(" || next === ")" || next === "*" || next === "+" || next === "%" || next === "#") {
      if (data[start + 2] === undefined) return -1;
      return 3;
    }

    // Everything else (`ESC =`, `ESC >`, `ESC 7`, `ESC M`, a lone ST) is two bytes.
    return 2;
  }

  private applyCsi(final: string, params: string): void {
    switch (final) {
      case "m":
        this.applySgr(parseParams(params));
        return;
      case "K": {
        const mode = parseParams(params)[0] ?? 0;
        if (mode === 0) this.truncate(this.col);
        else if (mode === 1) this.eraseToCursor();
        else if (mode === 2) {
          this.current = [];
          this.col = 0;
        }
        return;
      }
      case "J": {
        const mode = parseParams(params)[0] ?? 0;
        // 2/3 = erase the whole screen (and scrollback for 3). A log view honours both by
        // starting fresh; 0/1 only ever touch the line we are on.
        if (mode >= 2) this.clear();
        else this.truncate(this.col);
        return;
      }
      case "G": {
        this.col = Math.max(0, (parseParams(params)[0] ?? 1) - 1);
        return;
      }
      case "C": {
        this.col += Math.max(1, parseParams(params)[0] ?? 1);
        return;
      }
      case "D": {
        this.col = Math.max(0, this.col - Math.max(1, parseParams(params)[0] ?? 1));
        return;
      }
      default:
        // Cursor positioning, scroll regions, mode switches: consumed, not rendered.
        return;
    }
  }

  private applySgr(params: number[]): void {
    if (params.length === 0) {
      this.style = DEFAULT_ANSI_STYLE;
      return;
    }
    let style: AnsiStyle = { ...this.style };
    for (let i = 0; i < params.length; i += 1) {
      const code = params[i];
      if (code === 0) style = { ...DEFAULT_ANSI_STYLE };
      else if (code === 1) style.bold = true;
      else if (code === 2) style.dim = true;
      else if (code === 3) style.italic = true;
      else if (code === 4) style.underline = true;
      else if (code === 7) style.inverse = true;
      else if (code === 21 || code === 22) {
        style.bold = false;
        style.dim = false;
      } else if (code === 23) style.italic = false;
      else if (code === 24) style.underline = false;
      else if (code === 27) style.inverse = false;
      else if (code >= 30 && code <= 37) style.fg = { kind: "basic", index: code - 30 };
      else if (code === 39) style.fg = null;
      else if (code >= 40 && code <= 47) style.bg = { kind: "basic", index: code - 40 };
      else if (code === 49) style.bg = null;
      else if (code >= 90 && code <= 97) style.fg = { kind: "basic", index: code - 90 + 8 };
      else if (code >= 100 && code <= 107) style.bg = { kind: "basic", index: code - 100 + 8 };
      else if (code === 38 || code === 48) {
        const mode = params[i + 1];
        let color: AnsiColor | null = null;
        if (mode === 5) {
          color = indexedColor(params[i + 2] ?? 0);
          i += 2;
        } else if (mode === 2) {
          color = { kind: "rgb", r: params[i + 2] ?? 0, g: params[i + 3] ?? 0, b: params[i + 4] ?? 0 };
          i += 4;
        } else {
          i += 1;
        }
        if (code === 38) style.fg = color;
        else style.bg = color;
      }
      // Unknown attributes (blink, framed, overline, …) fall through untouched.
    }
    this.style = style;
  }

  // -------------------------------------------------------------------------
  // Line buffer
  // -------------------------------------------------------------------------

  private lineLength(): number {
    let total = 0;
    for (const span of this.current) total += span.text.length;
    return total;
  }

  private truncate(n: number): void {
    const spans: AnsiSpan[] = [];
    let remaining = n;
    for (const span of this.current) {
      if (remaining <= 0) break;
      if (span.text.length <= remaining) {
        spans.push(span);
        remaining -= span.text.length;
      } else {
        spans.push({ text: span.text.slice(0, remaining), style: span.style });
        remaining = 0;
      }
    }
    this.current = spans;
  }

  private eraseToCursor(): void {
    const tail: AnsiSpan[] = [];
    let skipped = 0;
    for (const span of this.current) {
      if (skipped >= this.col) {
        tail.push(span);
        continue;
      }
      const take = Math.min(span.text.length, this.col - skipped);
      skipped += take;
      if (take < span.text.length) tail.push({ text: span.text.slice(take), style: span.style });
    }
    this.current = this.col > 0 ? [{ text: " ".repeat(this.col), style: DEFAULT_ANSI_STYLE }, ...tail] : tail;
  }

  private appendSpan(text: string, style: AnsiStyle): void {
    const last = this.current[this.current.length - 1];
    // `style` is replaced wholesale on every SGR, so reference equality is an exact test for
    // "same run" and keeps consecutive chunks from fragmenting into one span per frame.
    if (last && last.style === style) last.text += text;
    else this.current.push({ text, style });
  }

  private writeText(text: string): void {
    if (!text) return;
    const len = this.lineLength();
    if (this.col < len) this.truncate(this.col);
    else if (this.col > len) this.appendSpan(" ".repeat(this.col - len), DEFAULT_ANSI_STYLE);
    this.appendSpan(text, this.style);
    this.col += text.length;
  }

  private newline(): void {
    this.done.push({ key: this.nextKey, spans: this.current });
    this.nextKey += 1;
    this.current = [];
    this.col = 0;
    if (this.done.length > this.maxLines) this.done.splice(0, this.done.length - this.maxLines);
  }
}

/** xterm's 256-colour cube/greyscale ramp → an rgb triple the renderer can match on. */
function indexedColor(index: number): AnsiColor {
  if (index < 16) return { kind: "basic", index };
  if (index < 232) {
    const n = index - 16;
    const level = (v: number) => (v === 0 ? 0 : 55 + v * 40);
    return { kind: "rgb", r: level(Math.floor(n / 36) % 6), g: level(Math.floor(n / 6) % 6), b: level(n % 6) };
  }
  const grey = 8 + (index - 232) * 10;
  return { kind: "rgb", r: grey, g: grey, b: grey };
}

/** Plain text of a line — used by tests and by the copy affordance. */
export function lineText(line: AnsiLine): string {
  return line.spans.map((span) => span.text).join("");
}
