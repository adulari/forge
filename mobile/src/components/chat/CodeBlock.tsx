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

import { type ColorTokens } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";
import { useTheme } from "../../theme/ThemeProvider";
import { IconButton } from "../ds/IconButton";

// ---------------------------------------------------------------------------
// Ported from app.js: HL_ALIAS, HL_KW, highlight() — verbatim keyword sets/alias map.
// ---------------------------------------------------------------------------

const HL_ALIAS: Record<string, string> = {
  ts: "js",
  tsx: "js",
  jsx: "js",
  javascript: "js",
  typescript: "js",
  py: "python",
  python3: "python",
  rs: "rust",
  sh: "bash",
  shell: "bash",
  zsh: "bash",
  console: "bash",
  golang: "go",
  jsonc: "json",
};

const HL_KW: Record<string, string> = {
  rust: "as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while",
  js: "async await break case catch class const continue default delete do else export extends false finally for from function if import in instanceof let new null of return static switch this throw true try typeof undefined var void while yield",
  python: "and as assert async await break class continue def del elif else except False finally for from global if import in is lambda None nonlocal not or pass raise return self True try while with yield",
  go: "break case chan const continue default defer else fallthrough false for func go goto if import interface map nil package range return select struct switch true type var",
  bash: "case do done echo elif else esac exit export fi for function if in local return set shift then until while",
  json: "false null true",
};

type TokenKind = "plain" | "keyword" | "string" | "comment" | "number";
interface HlToken {
  kind: TokenKind;
  text: string;
}

function highlightTokens(code: string, lang: string): HlToken[] {
  const tokens: HlToken[] = [];
  const resolved = HL_ALIAS[lang] ?? lang;
  const kw = HL_KW[resolved];
  if (!kw) {
    tokens.push({ kind: "plain", text: code });
    return tokens;
  }
  const kws = new Set(kw.split(" "));
  const lineComment = resolved === "python" || resolved === "bash" ? "#" : resolved === "json" ? null : "//";
  const blockComment = resolved === "rust" || resolved === "js" || resolved === "go" ? (["/*", "*/"] as const) : null;

  let i = 0;
  let plain = "";
  const flush = () => {
    if (plain) {
      tokens.push({ kind: "plain", text: plain });
      plain = "";
    }
  };
  const push = (kind: TokenKind, text: string) => {
    flush();
    tokens.push({ kind, text });
  };

  while (i < code.length) {
    const c = code[i];

    if (lineComment && code.startsWith(lineComment, i)) {
      let j = code.indexOf("\n", i);
      if (j < 0) j = code.length;
      push("comment", code.slice(i, j));
      i = j;
      continue;
    }
    if (blockComment && code.startsWith(blockComment[0], i)) {
      let j = code.indexOf(blockComment[1], i + 2);
      j = j < 0 ? code.length : j + 2;
      push("comment", code.slice(i, j));
      i = j;
      continue;
    }
    if (c === '"' || c === "'" || c === "`") {
      let j = i + 1;
      while (j < code.length && code[j] !== c && code[j] !== "\n") {
        if (code[j] === "\\") j++;
        j++;
      }
      j = Math.min(j + 1, code.length);
      push("string", code.slice(i, j));
      i = j;
      continue;
    }
    if (/[0-9]/.test(c) && !/[A-Za-z0-9_]/.test(code[i - 1] ?? "")) {
      let j = i;
      while (j < code.length && /[0-9a-fA-FxXoObB._]/.test(code[j])) j++;
      push("number", code.slice(i, j));
      i = j;
      continue;
    }
    if (/[A-Za-z_]/.test(c)) {
      let j = i;
      while (j < code.length && /[A-Za-z0-9_]/.test(code[j])) j++;
      const w = code.slice(i, j);
      if (kws.has(w)) push("keyword", w);
      else plain += w;
      i = j;
      continue;
    }
    plain += c;
    i++;
  }
  flush();
  return tokens;
}

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
      <ScrollView horizontal showsHorizontalScrollIndicator={false}>
        <Text accessibilityRole="text" selectable style={[type.code, styles.code, { color: tokens.ink }]}>
          {hlTokens.map((t, idx) => (
            <Text key={idx} style={tokenStyle(t.kind, tokens, keywordColor)}>
              {t.text}
            </Text>
          ))}
        </Text>
      </ScrollView>
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
  code: {
    padding: 12,
    fontFamily: monoFamily.regular,
  },
});
