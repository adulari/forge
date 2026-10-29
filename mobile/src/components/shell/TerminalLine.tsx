// One rendered row of the terminal scrollback, plus the palette that maps ANSI colours onto the
// Machined tokens. Split out of TerminalDock so the dock holds only socket and layout logic.
import React from "react";
import { Platform, StyleSheet, Text, type TextStyle } from "react-native";

import { type ColorTokens, hexToRgba } from "../../theme/tokens";
import { monoFamily } from "../../theme/typography";
import { type AnsiColor, type AnsiLine, type AnsiSpan } from "./ansi";
import { type LineHighlight, splitByHighlights } from "./terminalFind";
import { detectTerminalLinks, type TerminalLinkTarget } from "./terminalLinks";

export interface AnsiPalette {
  /** The 16 ANSI slots, in order. */
  basic: string[];
  /** Parsed rgb of each slot, for nearest-match on 256-colour/truecolour output. */
  basicRgb: [number, number, number][];
  fg: string;
  bg: string;
  link: string;
  /** Find matches: a wash for every hit, the solid accent for the one the bar is on. */
  match: string;
  matchActive: string;
  matchActiveInk: string;
}

function parseHex(hex: string): [number, number, number] {
  const h = hex.replace("#", "");
  return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)];
}

/**
 * ANSI's 8+8 palette mapped onto Machined's semantic tokens. Machined has no magenta or cyan
 * ramp — rather than inventing hex outside tokens.ts, magenta reuses the ember ramp and cyan
 * reuses `info`; every other slot has an exact semantic counterpart. Bright variants are the
 * same hue at full ink, normal variants sit one step quieter.
 */
export function ansiPalette(tokens: ColorTokens): AnsiPalette {
  const basic = [
    tokens.ink4, // black
    tokens.danger, // red
    tokens.success, // green
    tokens.warn, // yellow
    tokens.info, // blue
    tokens.ember.ember600, // magenta
    tokens.info, // cyan
    tokens.ink2, // white
    tokens.ink3, // bright black
    tokens.danger, // bright red
    tokens.success, // bright green
    tokens.warn, // bright yellow
    tokens.info, // bright blue
    tokens.ember.ember400, // bright magenta
    tokens.info, // bright cyan
    tokens.ink, // bright white
  ];
  return { basic, basicRgb: basic.map(parseHex), fg: tokens.ink, bg: tokens.bg0, link: tokens.info,
    match: hexToRgba(tokens.accent, 0.3),
    matchActive: tokens.accent,
    matchActiveInk: tokens.bg0,
  };
}

function resolveColor(color: AnsiColor | null, palette: AnsiPalette): string | null {
  if (color == null) return null;
  if (color.kind === "basic") return palette.basic[color.index] ?? palette.fg;
  // Truecolour and the 256-colour cube collapse to the nearest palette entry, so a build
  // script that emits #00d7ff still lands on a token instead of an off-system hex.
  let best = 0;
  let bestDistance = Infinity;
  for (let i = 0; i < palette.basicRgb.length; i += 1) {
    const [r, g, b] = palette.basicRgb[i];
    const distance = (r - color.r) ** 2 + (g - color.g) ** 2 + (b - color.b) ** 2;
    if (distance < bestDistance) {
      bestDistance = distance;
      best = i;
    }
  }
  return palette.basic[best];
}

function spanStyle(span: AnsiSpan, palette: AnsiPalette): TextStyle {
  const { style } = span;
  const fg = resolveColor(style.fg, palette) ?? palette.fg;
  const bg = resolveColor(style.bg, palette);
  const out: TextStyle = { color: style.inverse ? palette.bg : fg };
  if (style.inverse) out.backgroundColor = fg;
  else if (bg) out.backgroundColor = bg;
  if (style.bold) out.fontFamily = monoFamily.medium;
  if (style.dim) out.opacity = 0.6;
  if (style.italic) out.fontStyle = "italic";
  if (style.underline) out.textDecorationLine = "underline";
  return out;
}

/** What an OSC 8 target opens: web pages in a browser, `file://` URIs in the Files dock. */
export function osc8Target(uri: string): TerminalLinkTarget | null {
  if (/^https?:\/\//i.test(uri)) return { kind: "url", url: uri };
  if (/^file:\/\//i.test(uri)) {
    const path = decodeURIComponent(uri.replace(/^file:\/\/[^/]*/i, ""));
    return path ? { kind: "path", ref: { path, line: null, column: null } } : null;
  }
  return null;
}

const webLink = Platform.select<TextStyle | null>({ web: { cursor: "pointer" } as TextStyle, default: null });

export interface TerminalLineProps {
  line: AnsiLine;
  palette: AnsiPalette;
  caretColor: string | null;
  fontSize: number;
  lineHeight: number;
  onOpenLink: (target: TerminalLinkTarget) => void;
  /** Matches on this line while Find is open on the terminal. */
  highlights?: readonly LineHighlight[];
  /** Attached to the line holding the active match so the dock can scroll to it. */
  activeRef?: React.Ref<Text>;
}

function marked(text: string, offset: number, highlights: readonly LineHighlight[] | undefined, palette: AnsiPalette): React.ReactNode {
  if (!highlights) return text;
  return splitByHighlights(text, offset, highlights).map((piece, index) =>
    piece.active === null ? (
      piece.text
    ) : (
      <Text
        key={index}
        style={piece.active ? { backgroundColor: palette.matchActive, color: palette.matchActiveInk } : { backgroundColor: palette.match }}
      >
        {piece.text}
      </Text>
    ),
  );
}

function LinkText({
  text,
  target,
  base,
  palette,
  onOpenLink,
}: {
  text: React.ReactNode;
  target: TerminalLinkTarget;
  base: TextStyle;
  palette: AnsiPalette;
  onOpenLink: (target: TerminalLinkTarget) => void;
}) {
  return (
    <Text
      onPress={() => onOpenLink(target)}
      accessibilityRole="link"
      accessibilityLabel={target.kind === "url" ? `Open ${target.url}` : `Open ${target.ref.path}`}
      style={[base, { color: palette.link, textDecorationLine: "underline" }, webLink]}
    >
      {text}
    </Text>
  );
}

/** Memoised per line: every output chunk re-renders the list, but only the tail line changed. */
export const TerminalLine = React.memo(function TerminalLine({
  line,
  palette,
  caretColor,
  fontSize,
  lineHeight,
  onOpenLink,
  highlights,
  activeRef,
}: TerminalLineProps) {
  let spanOffset = 0;
  return (
    <Text ref={activeRef} style={[styles.line, { color: palette.fg, fontSize, lineHeight }]} selectable>
      {line.spans.map((span, index) => {
        const base = spanStyle(span, palette);
        const offset = spanOffset;
        spanOffset += span.text.length;
        const fromProgram = span.link ? osc8Target(span.link) : null;
        if (fromProgram) {
          return (
            <LinkText
              key={index}
              text={marked(span.text, offset, highlights, palette)}
              target={fromProgram}
              base={base}
              palette={palette}
              onOpenLink={onOpenLink}
            />
          );
        }
        let partOffset = offset;
        return (
          <Text key={index} style={base}>
            {detectTerminalLinks(span.text).map((part, partIndex) => {
              const at = partOffset;
              partOffset += part.text.length;
              const content = marked(part.text, at, highlights, palette);
              return part.type === "link" ? (
                <LinkText key={partIndex} text={content} target={part.target} base={base} palette={palette} onOpenLink={onOpenLink} />
              ) : (
                <React.Fragment key={partIndex}>{content}</React.Fragment>
              );
            })}
          </Text>
        );
      })}
      {caretColor ? <Text style={{ backgroundColor: caretColor, color: caretColor }}> </Text> : null}
    </Text>
  );
});

const styles = StyleSheet.create({
  line: { fontFamily: monoFamily.regular },
});
