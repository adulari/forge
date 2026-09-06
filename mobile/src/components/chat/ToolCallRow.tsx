// One tool invocation in the chat timeline — the call and the result the daemon serves as two
// separate rows, paired by `buildTranscript` and rendered as a single expandable card.
//
// Collapsed it is one quiet mono line: a disclosure chevron, the tool NAME, and the call's
// primary target ("shell scripts/ci/rust-checks.sh"), never the raw argument blob. The chevron is
// the point — a bare "shell" with no affordance gave no hint that the arguments and the output
// were one tap away. Expanded it shows the full arguments and the full output, each copyable,
// with the long-output "show N more lines" expander SystemOutput uses.
import * as Clipboard from "expo-clipboard";
import { Check, ChevronDown, ChevronRight, Copy } from "lucide-react-native";
import React, { useEffect, useMemo, useRef, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { haptics } from "../../lib/haptics";
import { formatArgs, resultFailed, summarizeToolArgs, type ToolInvocation } from "../../lib/toolRows";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";
import { IconButton } from "../ds/IconButton";

const COLLAPSE_LINES = 12;
const COPY_RESET_MS = 1200;

export interface ToolCallRowProps {
  invocation: ToolInvocation;
}

function CopyButton({ text }: { text: string }) {
  const tokens = useTokens();
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => { if (timer.current) clearTimeout(timer.current); }, []);
  return (
    <IconButton
      accessibilityLabel={copied ? "copied" : "copy"}
      onPress={async () => {
        await Clipboard.setStringAsync(text);
        setCopied(true);
        if (timer.current) clearTimeout(timer.current);
        timer.current = setTimeout(() => setCopied(false), COPY_RESET_MS);
      }}
      icon={
        copied ? (
          <Check size={14} color={tokens.success} strokeWidth={1.75} />
        ) : (
          <Copy size={14} color={tokens.ink3} strokeWidth={1.75} />
        )
      }
    />
  );
}

/** A titled mono block inside the expanded card: header (label + line count + copy), body, and
 * a "show N more lines" expander once the body is longer than `COLLAPSE_LINES`. */
function DetailBlock({ label, text }: { label: string; text: string }) {
  const tokens = useTokens();
  const lines = useMemo(() => text.split("\n"), [text]);
  const collapsible = lines.length > COLLAPSE_LINES;
  const [expanded, setExpanded] = useState(!collapsible);
  const visible = expanded ? lines : lines.slice(0, COLLAPSE_LINES);
  const hidden = lines.length - COLLAPSE_LINES;
  return (
    <View style={[styles.block, { borderTopColor: tokens.border }]}>
      <View style={styles.blockHead}>
        <Text style={[type.section, { color: tokens.ink4 }]}>{label}</Text>
        <View style={styles.blockHeadRight}>
          <Text style={[type.monoMeta, { color: tokens.ink4 }]}>
            {lines.length} line{lines.length === 1 ? "" : "s"}
          </Text>
          <CopyButton text={text} />
        </View>
      </View>
      <Text style={[type.codeSmall, { color: tokens.ink2, fontFamily: monoFamily.regular }]} selectable>
        {visible.join("\n")}
      </Text>
      {collapsible ? (
        <Pressable
          onPress={() => setExpanded((e) => !e)}
          accessibilityRole="button"
          accessibilityLabel={expanded ? "collapse" : `show ${hidden} more lines`}
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
            {expanded ? "show less" : `show ${hidden} more lines`}
          </Text>
        </Pressable>
      ) : null}
    </View>
  );
}

function ToolCallRowImpl({ invocation }: ToolCallRowProps) {
  const tokens = useTokens();
  const [open, setOpen] = useState(false);
  const { tool, args, result } = invocation;
  const name = tool ?? "tool";
  const target = useMemo(() => summarizeToolArgs(args, 52), [args]);
  const failed = resultFailed(result);
  // A call still awaiting its result has nothing to show but its own arguments; that is still
  // worth a card (it is what the model asked for), so `pending` only changes the status glyph.
  const pending = result === null;
  const argsBody = useMemo(() => (args.trim() ? formatArgs(args) : ""), [args]);
  const resultBody = (result ?? "").trim();
  const hasDetail = argsBody.length > 0 || resultBody.length > 0;
  const statusColor = pending ? tokens.ink4 : failed ? tokens.danger : tokens.success;

  return (
    <View style={styles.row}>
      <View style={[styles.card, { borderColor: tokens.border, backgroundColor: tokens.bg1 }]}>
        <Pressable
          onPress={hasDetail ? () => { haptics.select(); setOpen((v) => !v); } : undefined}
          disabled={!hasDetail}
          accessibilityRole={hasDetail ? "button" : undefined}
          accessibilityLabel={
            hasDetail ? `${open ? "hide" : "show"} details for ${name}` : `${name} tool call`
          }
          accessibilityState={hasDetail ? { expanded: open } : undefined}
          style={styles.summary}
          hitSlop={4}
        >
          {hasDetail ? (
            open ? (
              <ChevronDown size={14} strokeWidth={2} color={tokens.ink3} />
            ) : (
              <ChevronRight size={14} strokeWidth={2} color={tokens.ink3} />
            )
          ) : (
            <View style={styles.chevronSlot} />
          )}
          <Text
            style={[type.codeSmall, styles.name, { color: tokens.accent, fontFamily: monoFamily.regular }]}
            numberOfLines={1}
          >
            {name}
          </Text>
          {target ? (
            <Text
              style={[type.codeSmall, styles.target, { color: tokens.ink2, fontFamily: monoFamily.regular }]}
              numberOfLines={1}
            >
              {target}
            </Text>
          ) : (
            <View style={styles.target} />
          )}
          <View style={[styles.status, { backgroundColor: statusColor }]} />
        </Pressable>
        {open && hasDetail ? (
          <View>
            {argsBody ? <DetailBlock label="arguments" text={argsBody} /> : null}
            {resultBody ? <DetailBlock label="output" text={resultBody} /> : null}
          </View>
        ) : null}
      </View>
    </View>
  );
}

export const ToolCallRow = React.memo(ToolCallRowImpl);

const styles = StyleSheet.create({
  // Indented one gutter step past a message row so tool activity reads as subordinate to the
  // turn that produced it, not as another speaker.
  row: { paddingLeft: space.space24, paddingRight: space.space16, paddingVertical: space.space2 },
  card: { borderRadius: radii.radius8, borderWidth: StyleSheet.hairlineWidth, overflow: "hidden" },
  summary: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space8,
    paddingVertical: space.space8,
    minHeight: 32,
  },
  chevronSlot: { width: 14, height: 14 },
  name: { flexShrink: 0 },
  target: { flex: 1, minWidth: 0 },
  status: { width: 5, height: 5, borderRadius: radii.radiusPill },
  block: { borderTopWidth: StyleSheet.hairlineWidth, padding: space.space8, gap: space.space4 },
  blockHead: { flexDirection: "row", alignItems: "center", justifyContent: "space-between" },
  blockHeadRight: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  expander: { flexDirection: "row", alignItems: "center", gap: space.space4, paddingTop: space.space4, minHeight: 32 },
});
