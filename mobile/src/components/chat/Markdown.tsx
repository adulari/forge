// DESIGN_SYSTEM.md §6 Markdown: headings h3-cap, paragraphs, lists, inline code (bg3 radius4
// mono), bold, italic, links (ink + underline; open externally on tap, real `<a>` on web).
// Dependency-light: a small block+inline parser rather than a full markdown library — chat
// content is controlled (assistant/tool output), not arbitrary user markdown.
import React from "react";
import { Linking, Platform, StyleSheet, Text, View, type StyleProp, type TextStyle } from "react-native";

import type { ColorTokens } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";
import { useTheme } from "../../theme/ThemeProvider";
import { CodeBlock } from "./CodeBlock";

// ---------------------------------------------------------------------------
// Block parsing
// ---------------------------------------------------------------------------

type Block =
  | { kind: "heading"; level: number; text: string }
  | { kind: "paragraph"; text: string }
  | { kind: "list"; ordered: boolean; items: string[] }
  | { kind: "code"; language: string; code: string };

function parseBlocks(content: string): Block[] {
  const lines = content.replace(/\r\n/g, "\n").split("\n");
  const blocks: Block[] = [];
  let paragraphLines: string[] = [];
  let listItems: string[] = [];
  let listOrdered = false;

  const flushParagraph = () => {
    if (paragraphLines.length) {
      blocks.push({ kind: "paragraph", text: paragraphLines.join("\n") });
      paragraphLines = [];
    }
  };
  const flushList = () => {
    if (listItems.length) {
      blocks.push({ kind: "list", ordered: listOrdered, items: listItems });
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
      blocks.push({ kind: "heading", level: heading[1].length, text: heading[2].trim() });
      i++;
      continue;
    }

    const unordered = /^\s*[-*]\s+(.*)$/.exec(line);
    const ordered = /^\s*\d+\.\s+(.*)$/.exec(line);
    if (unordered || ordered) {
      flushParagraph();
      const nowOrdered = Boolean(ordered);
      if (listItems.length && nowOrdered !== listOrdered) flushList();
      listOrdered = nowOrdered;
      listItems.push((ordered ?? unordered)![1]);
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
  if (Platform.OS === "web") {
    // RN-Web renders a plain DOM anchor; kept out of JSX.IntrinsicElements via createElement
    // so it type-checks the same on native and web builds.
    return React.createElement("a", { href, target: "_blank", rel: "noopener noreferrer", style: linkStyle }, children);
  }
  return (
    <Text
      accessibilityRole="link"
      style={linkStyle}
      onPress={() => {
        Linking.openURL(href).catch(() => {});
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

export interface MarkdownProps {
  content: string;
  style?: StyleProp<TextStyle>;
}

export function Markdown({ content, style }: MarkdownProps) {
  const { tokens } = useTheme();
  const blocks = React.useMemo(() => parseBlocks(content), [content]);

  return (
    <View>
      {blocks.map((block, idx) => {
        const key = `b${idx}`;
        switch (block.kind) {
          case "heading":
            return (
              <Text
                key={key}
                accessibilityRole="header"
                selectable
                style={[type.heading, styles.heading, { color: tokens.ink }]}
              >
                {renderInline(parseInline(block.text), key, tokens)}
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
                    <Text style={[type.body, styles.listMarker, { color: tokens.ink3 }]}>
                      {block.ordered ? `${itemIdx + 1}.` : "•"}
                    </Text>
                    <Text selectable style={[type.body, styles.listText, { color: tokens.ink }, style]}>
                      {renderInline(parseInline(item), `${key}-${itemIdx}`, tokens)}
                    </Text>
                  </View>
                ))}
              </View>
            );
          case "paragraph":
          default:
            return (
              <Text key={key} selectable style={[type.body, styles.paragraph, { color: tokens.ink }, style]}>
                {renderInline(parseInline(block.text), key, tokens)}
              </Text>
            );
        }
      })}
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
