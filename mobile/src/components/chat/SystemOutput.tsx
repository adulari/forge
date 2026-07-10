// Tool/system output row body — MessageRow.tsx's isSystem branch used to dump `row.content`
// into a single unbounded mono <Text>, unusable for long logs. Same materials CodeBlock uses
// (header row + copy button, DESIGN_SYSTEM.md §6) plus a ReasoningDisclosure-style collapse:
// beyond COLLAPSE_LINES, show the head and a "show N more lines" expander.
import * as Clipboard from "expo-clipboard";
import { Check, ChevronDown, ChevronRight, Copy } from "lucide-react-native";
import React, { useEffect, useMemo, useRef, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";
import { IconButton } from "../ds/IconButton";

const COLLAPSE_LINES = 12;
const COPY_RESET_MS = 1200;

export interface SystemOutputProps {
  content: string;
}

export function SystemOutput({ content }: SystemOutputProps) {
  const tokens = useTokens();
  const lines = useMemo(() => content.split("\n"), [content]);
  const collapsible = lines.length > COLLAPSE_LINES;
  const [expanded, setExpanded] = useState(!collapsible);
  const [copied, setCopied] = useState(false);
  const resetTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(
    () => () => {
      if (resetTimer.current) clearTimeout(resetTimer.current);
    },
    [],
  );

  const visible = expanded ? content : lines.slice(0, COLLAPSE_LINES).join("\n");
  const hiddenCount = lines.length - COLLAPSE_LINES;

  const onCopy = async () => {
    await Clipboard.setStringAsync(content);
    setCopied(true);
    if (resetTimer.current) clearTimeout(resetTimer.current);
    resetTimer.current = setTimeout(() => setCopied(false), COPY_RESET_MS);
  };

  return (
    <View style={[styles.container, { backgroundColor: tokens.bg0, borderColor: tokens.border }]}>
      <View style={[styles.header, { borderBottomColor: tokens.border }]}>
        <Text style={[type.meta, { color: tokens.ink3 }]}>OUTPUT</Text>
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
      <View style={styles.body}>
        <Text style={[type.codeSmall, { color: tokens.ink3, fontFamily: monoFamily.regular }]} selectable>
          {visible}
        </Text>
        {collapsible ? (
          <Pressable
            onPress={() => setExpanded((e) => !e)}
            accessibilityRole="button"
            accessibilityLabel={expanded ? "collapse output" : `show ${hiddenCount} more lines`}
            accessibilityState={{ expanded }}
            style={styles.expander}
            hitSlop={8}
          >
            {expanded ? (
              <ChevronDown size={14} strokeWidth={1.75} color={tokens.ink3} />
            ) : (
              <ChevronRight size={14} strokeWidth={1.75} color={tokens.ink3} />
            )}
            <Text style={[type.meta, { color: tokens.ink3 }]}>
              {expanded ? "show less" : `show ${hiddenCount} more lines`}
            </Text>
          </Pressable>
        ) : null}
      </View>
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
    paddingHorizontal: space.space12,
    paddingVertical: space.space8,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  body: { padding: space.space12 },
  expander: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space4,
    paddingTop: space.space8,
    minHeight: 28,
  },
});
