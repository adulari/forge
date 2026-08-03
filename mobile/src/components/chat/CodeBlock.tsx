// DESIGN_SYSTEM.md §6 CodeBlock: bg0, radius12, mono `code`; header row = language tag
// (meta, ink3) + copy button (copy -> copied, 1.2s); horizontal scroll; syntax highlighting
// ported from crates/forge-cli/src/remote_assets/app.js's `highlight()`/`HL_KW`.
//
// The tokenizer below mirrors app.js exactly: same HL_ALIAS map, same per-language HL_KW
// keyword sets, same single-pass scan for line/block comments, quoted strings, numbers, and
// keywords. Only the output differs — app.js appends DOM text/span nodes, this emits a token
// list that CodeBlock renders as nested RN <Text> spans with theme colors instead of CSS classes.
import * as Clipboard from "expo-clipboard";
import { Check, Copy } from "lucide-react-native";
import React, { useEffect, useRef, useState } from "react";
import { ScrollView, StyleSheet, Text, View, type TextStyle } from "react-native";

import { useAppearancePreferences } from "../../lib/appearancePreferences";
import { type ColorTokens } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";
import { useTheme } from "../../theme/ThemeProvider";
import { IconButton } from "../ds/IconButton";

import { highlightTokens, type TokenKind } from "../../lib/highlightTokens";

function tokenStyle(kind: TokenKind, tokens: ColorTokens, keywordColor: string): TextStyle | undefined {
  switch (kind) {
    case "keyword":
      return { color: keywordColor };
    case "string":
      return { color: tokens.success };
    case "comment":
      return { color: tokens.ink3, fontStyle: "italic" };
    case "number":
      return { color: tokens.info };
    case "plain":
    default:
      return undefined;
  }
}

const COPY_RESET_MS = 1200;

export interface CodeBlockProps {
  code: string;
  language?: string;
}

export function CodeBlock({ code, language }: CodeBlockProps) {
  const { tokens, scheme } = useTheme();
  const { wrapCodeBlocks } = useAppearancePreferences();
  const keywordColor = scheme === "dark" ? tokens.ember.ember300 : tokens.ember.ember600;
  const hlTokens = React.useMemo(() => highlightTokens(code, (language ?? "").toLowerCase()), [code, language]);

  const [copied, setCopied] = useState(false);
  const resetTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => {
    if (resetTimer.current) clearTimeout(resetTimer.current);
  }, []);

  const onCopy = async () => {
    await Clipboard.setStringAsync(code);
    setCopied(true);
    if (resetTimer.current) clearTimeout(resetTimer.current);
    resetTimer.current = setTimeout(() => setCopied(false), COPY_RESET_MS);
  };

  const renderedCode = (
    <Text
      accessibilityRole="text"
      selectable
      style={[type.code, styles.code, wrapCodeBlocks ? styles.codeWrapped : null, { color: tokens.ink }]}
    >
      {hlTokens.map((t, idx) => (
        <Text key={idx} style={tokenStyle(t.kind, tokens, keywordColor)}>
          {t.text}
        </Text>
      ))}
    </Text>
  );

  return (
    <View style={[styles.container, { backgroundColor: tokens.bg0, borderColor: tokens.border }]}>
      <View style={[styles.header, { borderBottomColor: tokens.border }]}>
        <Text style={[type.meta, { color: tokens.ink3 }]}>{(language || "text").toUpperCase()}</Text>
        <IconButton
          accessibilityLabel={copied ? "copied" : "copy code"}
          onPress={onCopy}
          icon={
            copied ? (
              <Check size={20} color={tokens.success} strokeWidth={1.75} />
            ) : (
              <Copy size={20} color={tokens.ink3} strokeWidth={1.75} />
            )
          }
        />
      </View>
      {wrapCodeBlocks ? (
        <View style={styles.codeWrap}>{renderedCode}</View>
      ) : (
        <ScrollView horizontal showsHorizontalScrollIndicator={false} style={styles.codeScroll}>
          {renderedCode}
        </ScrollView>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    borderRadius: 12,
    borderWidth: StyleSheet.hairlineWidth,
    overflow: "hidden",
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: 12,
    paddingVertical: 8,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  codeScroll: { flexGrow: 0, flexShrink: 0 },
  codeWrap: { width: "100%" },
  code: {
    padding: 12,
    fontFamily: monoFamily.regular,
  },
  codeWrapped: {
    flexShrink: 1,
  },
});
