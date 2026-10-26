// DESIGN_SYSTEM.md §6 Markdown: headings h3-cap, paragraphs, lists, inline code (bg3 radius4
// mono), bold, italic, links (ink + underline; open externally on tap, real `<a>` on web).
// Dependency-light: a small block+inline parser rather than a full markdown library — chat
// content is controlled (assistant/tool output), not arbitrary user markdown.
import React from "react";
import { Linking, Platform, ScrollView, StyleSheet, Text, View, type StyleProp, type TextStyle } from "react-native";

import type { ColorTokens } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";
import { useTheme } from "../../theme/ThemeProvider";
import { safeExternalHref } from "../../lib/linkSafety";
import { columnWidths, fitRow, opensTable, tableAlignments, tableCells, type TableAlign } from "../../lib/markdownTable";
import { CodeBlock } from "./CodeBlock";

// ---------------------------------------------------------------------------
// Block parsing
// ---------------------------------------------------------------------------

type Block =
  | { kind: "heading"; level: number; text: string; inline: InlineNode[] }
  | { kind: "paragraph"; text: string; inline: InlineNode[] }
  | { kind: "list"; ordered: boolean; start: number; items: string[]; inlineItems: InlineNode[][] }
  | { kind: "code"; language: string; code: string }
  | { kind: "table"; header: InlineNode[][]; rows: InlineNode[][][]; aligns: (TableAlign | null)[]; widths: number[] };

function fitAligns(aligns: (TableAlign | null)[], columns: number): (TableAlign | null)[] {
  const out = aligns.slice(0, columns);
  while (out.length < columns) out.push(null);
  return out;
}

function parseBlocks(content: string): Block[] {
  const lines = content.replace(/\r\n/g, "\n").split("\n");
  const blocks: Block[] = [];
  let paragraphLines: string[] = [];
  let listItems: string[] = [];
  let listOrdered = false;
  // A loose list ("1. a", blank line, "2. b") arrives as one list per item; numbering from the
  // source's own first number keeps it 1, 2 instead of restarting at 1.
  let listStart = 1;

  const flushParagraph = () => {
    if (paragraphLines.length) {
      const text = paragraphLines.join("\n");
      blocks.push({ kind: "paragraph", text, inline: parseInline(text) });
      paragraphLines = [];
    }
  };
  const flushList = () => {
    if (listItems.length) {
      blocks.push({ kind: "list", ordered: listOrdered, start: listStart, items: listItems, inlineItems: listItems.map(parseInline) });
      listItems = [];
    }
  };

  let i = 0;
  while (i < lines.length) {
    const line = lines[i];

    const fence = /^```\s*([\w-]*)\s*$/.exec(line);
    if (fence) {
      flushParagraph();
      flushList();
      const language = fence[1] ?? "";
      const codeLines: string[] = [];
      i++;
      while (i < lines.length && !/^```\s*$/.test(lines[i])) {
        codeLines.push(lines[i]);
        i++;
      }
      i++; // skip closing fence
      blocks.push({ kind: "code", language, code: codeLines.join("\n") });
      continue;
    }

    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      flushParagraph();
      flushList();
      const text = heading[2].trim();
      blocks.push({ kind: "heading", level: heading[1].length, text, inline: parseInline(text) });
      i++;
      continue;
    }

    if (opensTable(line, lines[i + 1])) {
      flushParagraph();
      flushList();
      const header = tableCells(line);
      const aligns = fitAligns(tableAlignments(lines[i + 1]), header.length);
      const rawRows: string[][] = [];
      i += 2;
      while (i < lines.length && lines[i].includes("|") && lines[i].trim() !== "") {
        rawRows.push(fitRow(tableCells(lines[i]), header.length));
        i++;
      }
      blocks.push({
        kind: "table",
        header: header.map(parseInline),
        rows: rawRows.map((row) => row.map(parseInline)),
        aligns,
        widths: columnWidths([header, ...rawRows]),
      });
      continue;
    }

    const unordered = /^\s*[-*]\s+(.*)$/.exec(line);
    const ordered = /^\s*(\d+)\.\s+(.*)$/.exec(line);
    if (unordered || ordered) {
      flushParagraph();
      const nowOrdered = Boolean(ordered);
      if (listItems.length && nowOrdered !== listOrdered) flushList();
      if (!listItems.length) listStart = ordered ? Number(ordered[1]) : 1;
      listOrdered = nowOrdered;
      listItems.push(ordered ? ordered[2] : unordered![1]);
      i++;
      continue;
    }

    if (line.trim() === "") {
      flushParagraph();
      flushList();
      i++;
      continue;
    }

    flushList();
    paragraphLines.push(line);
    i++;
  }
  flushParagraph();
  flushList();
  return blocks;
}

// ---------------------------------------------------------------------------
// Inline parsing: code, bold, italic, links.
// ---------------------------------------------------------------------------

type InlineNode =
  | { type: "text"; text: string }
  | { type: "bold"; text: string }
  | { type: "italic"; text: string }
  | { type: "code"; text: string }
  | { type: "link"; text: string; href: string };

const INLINE_RE = /`([^`]+)`|\*\*([^*]+)\*\*|\*([^*]+)\*|_([^_]+)_|\[([^\]]+)\]\(([^)]+)\)/g;

function parseInline(text: string): InlineNode[] {
  const nodes: InlineNode[] = [];
  let lastIndex = 0;
  INLINE_RE.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = INLINE_RE.exec(text))) {
    if (match.index > lastIndex) nodes.push({ type: "text", text: text.slice(lastIndex, match.index) });
    if (match[1] !== undefined) nodes.push({ type: "code", text: match[1] });
    else if (match[2] !== undefined) nodes.push({ type: "bold", text: match[2] });
    else if (match[3] !== undefined) nodes.push({ type: "italic", text: match[3] });
    else if (match[4] !== undefined) nodes.push({ type: "italic", text: match[4] });
    else if (match[5] !== undefined && match[6] !== undefined) nodes.push({ type: "link", text: match[5], href: match[6] });
    lastIndex = INLINE_RE.lastIndex;
  }
  if (lastIndex < text.length) nodes.push({ type: "text", text: text.slice(lastIndex) });
  return nodes;
}

function MarkdownLink({ href, tokens, children }: { href: string; tokens: ColorTokens; children: string }) {
  const linkStyle = { color: tokens.ink, textDecorationLine: "underline" as const };
  const safeHref = safeExternalHref(href);
  if (!safeHref) return <Text>{children}</Text>;
  if (Platform.OS === "web") {
    // RN-Web renders a plain DOM anchor; kept out of JSX.IntrinsicElements via createElement
    // so it type-checks the same on native and web builds.
    return React.createElement("a", { href: safeHref, target: "_blank", rel: "noopener noreferrer", style: linkStyle }, children);
  }
  return (
    <Text
      accessibilityRole="link"
      style={linkStyle}
      onPress={() => {
        Linking.openURL(safeHref).catch(() => {});
      }}
    >
      {children}
    </Text>
  );
}

function renderInline(nodes: InlineNode[], keyPrefix: string, tokens: ColorTokens): React.ReactNode[] {
  return nodes.map((node, idx) => {
    const key = `${keyPrefix}-${idx}`;
    switch (node.type) {
      case "bold":
        return (
          <Text key={key} style={styles.bold}>
            {node.text}
          </Text>
        );
      case "italic":
        return (
          <Text key={key} style={styles.italic}>
            {node.text}
          </Text>
        );
      case "code":
        return (
          <Text
            key={key}
            style={[
              type.codeSmall,
              styles.inlineCode,
              { backgroundColor: tokens.bg3, color: tokens.ink, fontFamily: monoFamily.regular },
            ]}
          >
            {node.text}
          </Text>
        );
      case "link":
        return (
          <MarkdownLink key={key} href={node.href} tokens={tokens}>
            {node.text}
          </MarkdownLink>
        );
      case "text":
      default:
        return <Text key={key}>{node.text}</Text>;
    }
  });
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

/** An ordered list's marker column fits its widest number ("10." is wider than the bullet's
 * 18px), so the period never wraps onto a line of its own under the number. */
function orderedMarkerWidth(last: number): number {
  return Math.max(18, String(last).length * 9 + 6);
}

export interface MarkdownProps {
  content: string;
  style?: StyleProp<TextStyle>;
}

/** What a block renders from. A streamed reply re-parses on every frame; blocks whose text did
 * not change keep their element (a finished code block is not re-highlighted per token), so only
 * the block being written re-renders. */
function blockSignature(block: Block): string {
  switch (block.kind) {
    case "heading":
      return `h${block.level}:${block.text}`;
    case "paragraph":
      return `p:${block.text}`;
    case "list":
      return `l${block.ordered ? "o" : "u"}${block.start}:${block.items.join("\n")}`;
    case "code":
      return `c${block.language}:${block.code}`;
    case "table":
      return `t:${JSON.stringify([block.header, block.rows, block.aligns])}`;
  }
}

interface MarkdownBlockProps {
  block: Block;
  signature: string;
  blockKey: string;
  style?: StyleProp<TextStyle>;
}

function MarkdownBlockImpl({ block, blockKey: key, style }: MarkdownBlockProps) {
  const { tokens } = useTheme();
  switch (block.kind) {
    case "heading":
      return (
        <Text
          key={key}
          accessibilityRole="header"
          selectable
          style={[type.heading, styles.heading, { color: tokens.ink }]}
        >
          {renderInline(block.inline, key, tokens)}
        </Text>
      );
    case "code":
      return (
        <View key={key} style={styles.block}>
          <CodeBlock code={block.code} language={block.language} />
        </View>
      );
    case "list":
      return (
        <View key={key} style={styles.block}>
          {block.items.map((item, itemIdx) => (
            <View key={itemIdx} style={styles.listRow}>
              <Text
                numberOfLines={1}
                style={[type.body, styles.listMarker, block.ordered ? { width: orderedMarkerWidth(block.start + block.items.length - 1) } : null, { color: tokens.ink3 }]}
              >
                {block.ordered ? `${block.start + itemIdx}.` : "•"}
              </Text>
              <Text selectable style={[type.body, styles.listText, { color: tokens.ink }, style]}>
                {renderInline(block.inlineItems[itemIdx], `${key}-${itemIdx}`, tokens)}
              </Text>
            </View>
          ))}
        </View>
      );
    case "table":
      return (
        <ScrollView
          key={key}
          horizontal
          showsHorizontalScrollIndicator
          style={styles.block}
          contentContainerStyle={styles.tableScroll}
          accessibilityRole="none"
        >
          <View style={[styles.table, { borderColor: tokens.border }]}>
            {[block.header, ...block.rows].map((row, rowIdx) => (
              <View
                key={rowIdx}
                style={[
                  styles.tableRow,
                  rowIdx > 0 ? { borderTopColor: tokens.border, borderTopWidth: StyleSheet.hairlineWidth } : null,
                  rowIdx === 0 ? { backgroundColor: tokens.bg2 } : null,
                ]}
              >
                {row.map((cell, cellIdx) => (
                  <Text
                    key={cellIdx}
                    selectable
                    style={[
                      type.body,
                      styles.tableCell,
                      { flexBasis: block.widths[cellIdx], textAlign: block.aligns[cellIdx] ?? "left" },
                      rowIdx === 0 ? styles.bold : null,
                      cellIdx > 0 ? { borderLeftColor: tokens.border, borderLeftWidth: StyleSheet.hairlineWidth } : null,
                      { color: tokens.ink },
                      style,
                    ]}
                  >
                    {renderInline(cell, `${key}-${rowIdx}-${cellIdx}`, tokens)}
                  </Text>
                ))}
              </View>
            ))}
          </View>
        </ScrollView>
      );
    case "paragraph":
    default:
      return (
        <Text key={key} selectable style={[type.body, styles.paragraph, { color: tokens.ink }, style]}>
          {renderInline(block.inline, key, tokens)}
        </Text>
      );
  }
}

const MarkdownBlock = React.memo(
  MarkdownBlockImpl,
  (prev, next) => prev.signature === next.signature && prev.style === next.style && prev.blockKey === next.blockKey,
);

export function Markdown({ content, style }: MarkdownProps) {
  const blocks = React.useMemo(() => parseBlocks(content), [content]);
  return (
    <View>
      {blocks.map((block, idx) => (
        <MarkdownBlock key={`b${idx}`} blockKey={`b${idx}`} block={block} signature={blockSignature(block)} style={style} />
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  block: {
    marginVertical: 8,
  },
  heading: {
    marginTop: 12,
    marginBottom: 4,
  },
  paragraph: {
    marginVertical: 4,
  },
  // Columns keep a width that fits their text and grow to fill a wide pane; a table wider than the
  // pane scrolls sideways instead of crushing its columns.
  tableScroll: {
    flexGrow: 1,
  },
  table: {
    borderWidth: StyleSheet.hairlineWidth,
    flexGrow: 1,
  },
  tableRow: {
    flexDirection: "row",
  },
  tableCell: {
    flexGrow: 1,
    flexShrink: 0,
    paddingHorizontal: 8,
    paddingVertical: 4,
  },
  listRow: {
    flexDirection: "row",
    marginVertical: 2,
  },
  listMarker: {
    width: 18,
    marginRight: 6,
  },
  listText: {
    flex: 1,
  },
  bold: {
    fontWeight: "700",
  },
  italic: {
    fontStyle: "italic",
  },
  inlineCode: {
    borderRadius: 4,
    paddingHorizontal: 4,
  },
});
