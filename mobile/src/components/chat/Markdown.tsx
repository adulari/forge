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
import { parseBlocks, type Block, type InlineNode } from "../../lib/markdownParse";
import { CodeBlock } from "./CodeBlock";
import { MarkdownImage, MarkdownMediaProvider } from "./MarkdownMedia";
import { MathView } from "./MathView";
import { PathChip } from "./PathChip";

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
      case "path":
        return <PathChip key={key} pathRef={node.ref} text={node.text} />;
      case "math":
        return <MathView key={key} tex={node.tex} display={false} />;
      case "image":
        return /^[a-z][a-z0-9+.-]*:/i.test(node.src) ? (
          <MarkdownLink key={key} href={node.src} tokens={tokens}>
            {node.alt || node.src}
          </MarkdownLink>
        ) : (
          <PathChip key={key} pathRef={{ path: node.src, line: null, column: null }} text={node.src} />
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
  /** Also turn `@path` mentions into chips: set for the user's own prompts. */
  mentions?: boolean;
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
    case "math":
      return `m:${block.tex}`;
    case "image":
      return `i:${block.src}:${block.alt}`;
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
    case "math":
      return (
        <View key={key} style={styles.block}>
          <MathView tex={block.tex} display />
        </View>
      );
    case "image":
      return (
        <View key={key} style={styles.block}>
          <MarkdownImage alt={block.alt} src={block.src} />
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

export function Markdown({ content, style, mentions = false }: MarkdownProps) {
  const blocks = React.useMemo(() => parseBlocks(content, mentions), [content, mentions]);
  return (
    <MarkdownMediaProvider>
      <View>
        {blocks.map((block, idx) => (
          <MarkdownBlock key={`b${idx}`} blockKey={`b${idx}`} block={block} signature={blockSignature(block)} style={style} />
        ))}
      </View>
    </MarkdownMediaProvider>
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
