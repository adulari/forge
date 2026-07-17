// Tool/system output row body — MessageRow.tsx's isSystem branch used to dump `row.content`
// into a single unbounded mono <Text>, unusable for long logs. Hearth: default view is ONE
// mono summary line (status glyph + verb + target, never raw JSON args — mobile.dc.html
// "✓ run scripts/sweep.py · 128 runs · 3m 12s" / "✓ edit strategies/volmom.py +38 −9"); tapping
// it reveals the full raw output in the same boxed/collapsible presentation CodeBlock uses
// (header row + copy button, DESIGN_SYSTEM.md §6), including the original "show N more lines"
// expander for genuinely long output.
import * as Clipboard from "expo-clipboard";
import { Check, ChevronDown, ChevronRight, Copy } from "lucide-react-native";
import React, { useEffect, useMemo, useRef, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";
import { DiffLines } from "../review/DiffLines";
import { renderAssayReport } from "../session/AssayView";
import { IconButton } from "../ds/IconButton";

const COLLAPSE_LINES = 4;
const COPY_RESET_MS = 1200;

export interface SystemOutputProps {
  content: string;
}

// Preference order for the "primary target" pulled out of a tool call's JSON args — the first
// of these present as a non-empty string wins. Covers the common shapes across Forge's tool
// surface (file ops, shell, search) without needing per-tool-name special cases.
const TARGET_KEYS = ["path", "file", "filepath", "file_path", "command", "cmd", "script", "query", "pattern", "url"];

function truncateMiddle(value: string, max = 44): string {
  const trimmed = value.trim();
  if (trimmed.length <= max) return trimmed;
  const head = Math.ceil((max - 1) / 2);
  const tail = Math.floor((max - 1) / 2);
  return `${trimmed.slice(0, head)}…${trimmed.slice(trimmed.length - tail)}`;
}

/**
 * Turns a transcript/tool-call line (e.g. `↳ write_file {"content":"…","cwd":"…"}`) into a
 * compact one-line summary. A line with no `{…}` blob is already clean and passes through
 * unchanged; a JSON blob is parsed for a recognizable target field instead of ever being
 * rendered verbatim — falls back to the bare tool name (never the raw args) if nothing
 * recognizable is found or the blob doesn't parse.
 */
export function summarizeToolLine(rawLine: string): string {
  const line = rawLine.replace(/^↳\s*/, "").trim();
  const braceIdx = line.indexOf("{");
  if (braceIdx === -1) return line;
  const name = line.slice(0, braceIdx).trim().replace(/[:(]+$/, "") || "tool";
  let argsText = line.slice(braceIdx);
  if (argsText.endsWith(")")) argsText = argsText.slice(0, -1);
  try {
    const parsed = JSON.parse(argsText) as Record<string, unknown>;
    for (const key of TARGET_KEYS) {
      const value = parsed[key];
      if (typeof value === "string" && value.length > 0) return `${name} ${truncateMiddle(value)}`;
    }
  } catch {
    // unparsable args — fall through to the bare tool name rather than leak raw text
  }
  return name;
}

// A recognizable assay report (the headless runner's `◈ ASSAY REPORT` plain block or a
// `## Forge Assay Report` markdown table) renders richly via AssayView; every other system row
// falls through to the plain summary/collapsible body below, exactly as before.
export function SystemOutput({ content }: SystemOutputProps) {
  return renderAssayReport(content) ?? <SystemOutputBody content={content} />;
}

function SystemOutputBody({ content }: SystemOutputProps) {
  const tokens = useTokens();
  const lines = useMemo(() => content.split("\n"), [content]);
  const collapsible = lines.length > COLLAPSE_LINES;
  const [innerExpanded, setInnerExpanded] = useState(!collapsible);
  const [detailShown, setDetailShown] = useState(false);
  const [copied, setCopied] = useState(false);
  const resetTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(
    () => () => {
      if (resetTimer.current) clearTimeout(resetTimer.current);
    },
    [],
  );

  const firstLine = lines.find((line) => line.trim()) ?? "output";
  const summary = useMemo(() => summarizeToolLine(firstLine), [firstLine]);
  // A tool row only ever renders once the call has settled into history — the live/busy state
  // is LiveToolActivity's job (session/[id]/index.tsx) — so the glyph is done (✓) unless the
  // line itself reports a failure.
  const failed = /\b(failed|error)\b/i.test(firstLine) && !/\bpassed\b/i.test(firstLine);
  const hasDetail = lines.length > 1 || summary !== firstLine.replace(/^↳\s*/, "").trim();

  const hiddenCount = lines.length - COLLAPSE_LINES;
  const diffLike = lines.filter((line) => line.startsWith("+") || line.startsWith("-")).length / Math.max(lines.length, 1) > 0.3;
  const visibleLines = innerExpanded ? lines : lines.slice(0, COLLAPSE_LINES);

  const onCopy = async () => {
    await Clipboard.setStringAsync(content);
    setCopied(true);
    if (resetTimer.current) clearTimeout(resetTimer.current);
    resetTimer.current = setTimeout(() => setCopied(false), COPY_RESET_MS);
  };

  return (
    <View style={styles.container}>
      <Pressable
        onPress={hasDetail ? () => setDetailShown((v) => !v) : undefined}
        disabled={!hasDetail}
        accessibilityRole={hasDetail ? "button" : undefined}
        accessibilityLabel={hasDetail ? (detailShown ? "hide tool detail" : "show full tool output") : undefined}
        accessibilityState={hasDetail ? { expanded: detailShown } : undefined}
        style={styles.summaryRow}
        hitSlop={8}
      >
        <Text style={[type.codeSmall, { color: failed ? tokens.danger : tokens.success, fontFamily: monoFamily.regular }]}>
          {failed ? "✗" : "✓"}
        </Text>
        <Text style={[type.codeSmall, { color: tokens.ink3, fontFamily: monoFamily.regular }]} numberOfLines={1} selectable>
          {summary}
        </Text>
      </Pressable>
      {detailShown ? (
        <View style={[styles.body, { backgroundColor: tokens.bg0, borderColor: tokens.border }]}>
          <View style={[styles.header, { borderBottomColor: tokens.border }]}>
            <Text style={[type.meta, { color: tokens.ink3 }]}>{lines.length} line{lines.length === 1 ? "" : "s"}</Text>
            <IconButton
              accessibilityLabel={copied ? "copied" : "copy output"}
              onPress={onCopy}
              icon={
                copied ? (
                  <Check size={16} color={tokens.success} strokeWidth={1.75} />
                ) : (
                  <Copy size={16} color={tokens.ink3} strokeWidth={1.75} />
                )
              }
            />
          </View>
          <View style={styles.bodyContent}>
            {diffLike ? (
              <DiffLines lines={visibleLines} />
            ) : (
              <Text style={[type.codeSmall, { color: tokens.ink3, fontFamily: monoFamily.regular }]} selectable>
                {visibleLines.join("\n")}
              </Text>
            )}
            {collapsible ? (
              <Pressable
                onPress={() => setInnerExpanded((e) => !e)}
                accessibilityRole="button"
                accessibilityLabel={innerExpanded ? "collapse output" : `show ${hiddenCount} more lines`}
                accessibilityState={{ expanded: innerExpanded }}
                style={styles.expander}
                hitSlop={8}
              >
                {innerExpanded ? (
                  <ChevronDown size={14} strokeWidth={1.75} color={tokens.ink3} />
                ) : (
                  <ChevronRight size={14} strokeWidth={1.75} color={tokens.ink3} />
                )}
                <Text style={[type.meta, { color: tokens.ink3 }]}>
                  {innerExpanded ? "show less" : `show ${hiddenCount} more lines`}
                </Text>
              </Pressable>
            ) : null}
          </View>
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  container: { gap: space.space4 },
  summaryRow: { flexDirection: "row", alignItems: "center", gap: space.space8, alignSelf: "flex-start" },
  body: {
    borderRadius: 12,
    borderWidth: StyleSheet.hairlineWidth,
    overflow: "hidden",
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: space.space12,
    paddingVertical: space.space8,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  bodyContent: { padding: space.space12 },
  expander: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space4,
    paddingTop: space.space8,
    minHeight: 44,
  },
});
