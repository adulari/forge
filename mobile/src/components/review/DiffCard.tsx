// DESIGN_SYSTEM.md §6 DiffCard: per Snapshot.diff — `pending` variant gets a warn
// banner "proposed change — review before allowing"; collapsible file sections
// (chevron), header path (mono head-ellipsis), kind badge, `+a -d` tabular
// (success/danger); hunk header info-color mono; lines mono `codeSmall` with
// successBg/dangerBg full-width fills; "+N more lines/files" ink3 footers.
//
// Used both standalone in the Review segment (any diff, pending or landed) and
// embedded inside PermissionCard when `diff.pending` (FEATURES.md §1.2).
import * as Clipboard from "expo-clipboard";
import { Check, ChevronDown, ChevronRight, Copy } from "lucide-react-native";
import React, { useEffect, useRef, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { Badge, type BadgeTone } from "../ds/Badge";
import { Banner } from "../ds/Banner";
import { IconButton } from "../ds/IconButton";
import { type Diff, type DiffFile } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";

export interface DiffCardProps {
  diff: Diff;
  /** Caps the card at this height with its own internal ScrollView, so whatever sits below it
   * (PermissionCard's Allow/Deny bar) never gets pushed off-screen by a large diff. Omitted
   * (full height, no internal scroll) on the standalone Review screen, which is already
   * scrollable end-to-end — this only matters where DiffCard is embedded in a non-scrolling
   * slot (FEATURES.md §1.2). */
  maxHeight?: number;
}

const HEAD_ELLIPSIS_MAX = 42;
const COPY_RESET_MS = 1200;

/** Mono "head-ellipsis": keeps the tail of a long path, prefixed with an ellipsis. */
function headEllipsis(path: string, max: number = HEAD_ELLIPSIS_MAX): string {
  if (path.length <= max) return path;
  return `…${path.slice(-(max - 1))}`;
}

function kindTone(kind: DiffFile["kind"]): BadgeTone {
  switch (kind) {
    case "created":
      return "success";
    case "deleted":
      return "danger";
    case "modified":
    default:
      return "neutral";
  }
}

export function DiffCard({ diff, maxHeight }: DiffCardProps) {
  const tokens = useTokens();

  const body = (
    <>
      {diff.pending ? (
        <Banner tone="warn" message="proposed change — review before allowing" />
      ) : null}

      {diff.files.map((file, idx) => (
        <DiffFileSection key={`${file.path}-${idx}`} file={file} isLast={idx === diff.files.length - 1} />
      ))}

      {diff.skipped_files > 0 ? (
        <Text style={[typeScale.sub, { color: tokens.ink3 }, styles.footer]}>
          +{diff.skipped_files} more file{diff.skipped_files === 1 ? "" : "s"}
        </Text>
      ) : null}
    </>
  );

  return (
    <View style={[styles.container, { backgroundColor: tokens.bg0, borderColor: tokens.border }]}>
      {maxHeight != null ? (
        <ScrollView style={{ maxHeight }} nestedScrollEnabled showsVerticalScrollIndicator>
          {body}
        </ScrollView>
      ) : (
        body
      )}
    </View>
  );
}

function DiffFileSection({ file, isLast }: { file: DiffFile; isLast: boolean }) {
  const tokens = useTokens();
  const [expanded, setExpanded] = useState(true);
  const [copied, setCopied] = useState(false);
  const resetTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => {
    if (resetTimer.current) clearTimeout(resetTimer.current);
  }, []);

  const onCopy = async () => {
    const patch = file.hunks.map((h) => [h.header, ...h.lines].join("\n")).join("\n");
    await Clipboard.setStringAsync(patch);
    setCopied(true);
    if (resetTimer.current) clearTimeout(resetTimer.current);
    resetTimer.current = setTimeout(() => setCopied(false), COPY_RESET_MS);
  };

  return (
    <View style={[!isLast && styles.fileDivider, { borderBottomColor: tokens.border }]}>
      <View style={styles.fileHeaderRow}>
        <Pressable
          onPress={() => setExpanded((v) => !v)}
          accessibilityRole="button"
          accessibilityLabel={`${expanded ? "collapse" : "expand"} ${file.path}`}
          accessibilityState={{ expanded }}
          style={styles.fileHeader}
          hitSlop={8}
        >
          {expanded ? (
            <ChevronDown size={16} strokeWidth={1.75} color={tokens.ink3} />
          ) : (
            <ChevronRight size={16} strokeWidth={1.75} color={tokens.ink3} />
          )}
          <Text
            selectable
            style={[typeScale.bodyBold, { color: tokens.ink, fontFamily: monoFamily.regular }, styles.filePath]}
            numberOfLines={1}
          >
            {headEllipsis(file.path)}
          </Text>
          <Badge label={file.kind} tone={kindTone(file.kind)} />
          {!file.binary ? (
            <Text style={[typeScale.meta, styles.counts]}>
              <Text style={{ color: tokens.success, fontFamily: monoFamily.regular }}>+{file.adds}</Text>
              {" "}
              <Text style={{ color: tokens.danger, fontFamily: monoFamily.regular }}>-{file.dels}</Text>
            </Text>
          ) : null}
        </Pressable>
        {!file.binary ? (
          <IconButton
            accessibilityLabel={copied ? "copied" : "copy patch"}
            onPress={onCopy}
            icon={
              copied ? (
                <Check size={20} color={tokens.success} strokeWidth={1.75} />
              ) : (
                <Copy size={20} color={tokens.ink3} strokeWidth={1.75} />
              )
            }
          />
        ) : null}
      </View>

      {expanded && !file.binary ? (
        <ScrollView horizontal showsHorizontalScrollIndicator={false} style={styles.hunkScroll}>
          <View>
            {file.hunks.map((hunk, hIdx) => (
              <View key={hIdx} style={styles.hunk}>
                <Text selectable style={[typeScale.codeSmall, { color: tokens.info }, styles.hunkHeader]}>{hunk.header}</Text>
                {hunk.lines.map((line, lIdx) => {
                  const gutter = line[0] ?? " ";
                  const bg =
                    gutter === "+" ? tokens.successBg : gutter === "-" ? tokens.dangerBg : "transparent";
                  const ink = gutter === "+" ? tokens.success : gutter === "-" ? tokens.danger : tokens.ink2;
                  return (
                    <View key={lIdx} style={[styles.lineRow, { backgroundColor: bg }]}>
                      <Text selectable style={[typeScale.codeSmall, { color: ink }]}>{line || " "}</Text>
                    </View>
                  );
                })}
              </View>
            ))}
            {file.skipped_lines > 0 ? (
              <Text style={[typeScale.sub, { color: tokens.ink3 }, styles.footer]}>
                +{file.skipped_lines} more line{file.skipped_lines === 1 ? "" : "s"}
              </Text>
            ) : null}
          </View>
        </ScrollView>
      ) : expanded && file.binary ? (
        <Text style={[typeScale.sub, { color: tokens.ink3 }, styles.footer]}>binary file</Text>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  container: { borderRadius: 12, borderWidth: StyleSheet.hairlineWidth, overflow: "hidden" },
  fileDivider: { borderBottomWidth: StyleSheet.hairlineWidth },
  fileHeaderRow: { flexDirection: "row", alignItems: "center" },
  fileHeader: {
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space12,
    paddingVertical: space.space8,
    minHeight: 44,
  },
  filePath: { flex: 1 },
  counts: { flexShrink: 0 },
  hunkScroll: { marginBottom: space.space4 },
  hunk: { paddingBottom: space.space8 },
  hunkHeader: { paddingHorizontal: space.space12, paddingVertical: space.space4 },
  lineRow: { paddingHorizontal: space.space12, minWidth: "100%" },
  footer: { paddingHorizontal: space.space12, paddingVertical: space.space8 },
});
