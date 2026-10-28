// One tool invocation in the chat timeline — the call and the result the daemon serves as two
// separate rows, paired by `buildTranscript` and rendered as a single expandable card.
//
// Collapsed it is one quiet line with no box: a disclosure chevron, a short verb ("Run",
// "Read", "Script") and what the call was about (the command, the path, what a script ran),
// never the raw argument blob or a bare tool name. A failure says "failed" once, in danger ink;
// success needs no mark. Expanded it shows the full arguments and output, each copyable, with
// the long-output "show N more lines" expander SystemOutput uses.
import * as Clipboard from "expo-clipboard";
import { Check, ChevronDown, ChevronRight, Copy } from "lucide-react-native";
import React, { useEffect, useMemo, useRef, useState } from "react";
import { ActivityIndicator, Pressable, StyleSheet, Text, View } from "react-native";

import { agentsForInvocation, useAgentIndex } from "../../lib/agentLinks";
import { haptics } from "../../lib/haptics";
import { openSession } from "../../lib/nav";
import { callHeadline, callVerb, readableArgs } from "../../lib/toolLabels";
import { formatArgs, resultFailed, type ToolInvocation } from "../../lib/toolRows";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";
import { IconButton } from "../ds/IconButton";

const COLLAPSE_LINES = 12;
const COPY_RESET_MS = 1200;

export interface ToolCallRowProps {
  invocation: ToolInvocation;
}

export function CopyButton({ text }: { text: string }) {
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
export function DetailBlock({ label, text }: { label: string; text: string }) {
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
  const verb = useMemo(() => callVerb(tool, args), [tool, args]);
  const target = useMemo(() => callHeadline(tool, args, result), [tool, args, result]);
  const failed = resultFailed(result);
  // A call still awaiting its result has nothing to show but its own arguments; that is still
  // worth a row (it is what the model asked for), so `pending` only changes the status mark.
  const pending = result === null;
  const argsBody = useMemo(() => (args.trim() ? formatArgs(args) : ""), [args]);
  const argSections = useMemo(() => readableArgs(tool, args), [tool, args]);
  const [raw, setRaw] = useState(false);
  const agentIndex = useAgentIndex();
  const children = useMemo(() => agentsForInvocation(invocation, agentIndex), [invocation, agentIndex]);
  const resultBody = (result ?? "").trim();
  const hasDetail = argsBody.length > 0 || resultBody.length > 0;

  return (
    <View style={styles.row}>
      <View style={[styles.card, open ? { borderColor: tokens.border, backgroundColor: tokens.bg1 } : styles.flat]}>
        <Pressable
          onPress={hasDetail ? () => { haptics.select(); setOpen((v) => !v); } : undefined}
          disabled={!hasDetail}
          accessibilityRole={hasDetail ? "button" : undefined}
          accessibilityLabel={`${verb} ${target || name}${failed ? ", failed" : ""}${hasDetail ? `, ${open ? "hide" : "show"} details` : ""}`}
          accessibilityState={hasDetail ? { expanded: open } : undefined}
          style={({ pressed }) => [styles.summary, pressed ? { backgroundColor: tokens.bg2 } : null]}
          hitSlop={4}
        >
          {hasDetail ? (
            open ? (
              <ChevronDown size={13} strokeWidth={2} color={tokens.ink4} />
            ) : (
              <ChevronRight size={13} strokeWidth={2} color={tokens.ink4} />
            )
          ) : (
            <View style={styles.chevronSlot} />
          )}
          <Text style={[type.meta, styles.name, { color: failed ? tokens.danger : tokens.ink3 }]} numberOfLines={1}>
            {verb}
          </Text>
          <Text
            style={[type.codeSmall, styles.target, { color: tokens.ink2, fontFamily: monoFamily.regular }]}
            numberOfLines={1}
          >
            {target}
          </Text>
          {pending ? (
            <ActivityIndicator size="small" color={tokens.ink4} style={styles.spinner} />
          ) : failed ? (
            <Text style={[type.meta, { color: tokens.danger }]}>failed</Text>
          ) : null}
        </Pressable>
        {children.length > 0 ? (
          <View style={styles.agentLinks}>
            {children.map((child) => (
              <Pressable
                key={child.session_id}
                onPress={() => openSession(child.session_id)}
                accessibilityRole="button"
                accessibilityLabel={`Open ${child.agent} transcript`}
                style={[styles.agentChip, { backgroundColor: tokens.bg3 }]}
                hitSlop={6}
              >
                <Text style={[type.monoMeta, { color: tokens.ink2 }]} numberOfLines={1}>
                  {child.agent}
                </Text>
                <ChevronRight size={12} strokeWidth={1.75} color={tokens.ink3} />
              </Pressable>
            ))}
          </View>
        ) : null}
        {open && hasDetail ? (
          <View>
            {raw
              ? (argsBody ? <DetailBlock label={`${name} arguments`} text={argsBody} /> : null)
              : argSections.map((section) => <DetailBlock key={section.label} label={section.label} text={section.text} />)}
            {resultBody ? <DetailBlock label="output" text={resultBody} /> : null}
            {argsBody ? (
              <Pressable
                onPress={() => setRaw((v) => !v)}
                accessibilityRole="button"
                accessibilityLabel={raw ? "show readable arguments" : "show raw arguments"}
                style={[styles.rawToggle, { borderTopColor: tokens.border }]}
                hitSlop={8}
              >
                <Text style={[type.meta, { color: tokens.ink3 }]}>{raw ? "readable" : `raw ${name} arguments`}</Text>
              </Pressable>
            ) : null}
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
  row: { paddingLeft: space.space16, paddingRight: space.space16, paddingVertical: 1 },
  // Collapsed, a call is one quiet line with no box; opened, it gets the card its details need.
  card: { borderRadius: radii.radius8, borderWidth: StyleSheet.hairlineWidth, overflow: "hidden" },
  flat: { borderColor: "transparent" },
  summary: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space8,
    paddingVertical: space.space4,
    minHeight: 30,
  },
  agentLinks: { flexDirection: "row", flexWrap: "wrap", gap: space.space4, paddingHorizontal: space.space8, paddingBottom: space.space4 },
  agentChip: { flexDirection: "row", alignItems: "center", gap: space.space4, height: 24, paddingHorizontal: space.space8, borderRadius: radii.radiusPill },
  chevronSlot: { width: 13, height: 13 },
  name: { flexShrink: 0, minWidth: 40 },
  target: { flex: 1, minWidth: 0 },
  spinner: { transform: [{ scale: 0.6 }], width: 16, height: 16 },
  block: { borderTopWidth: StyleSheet.hairlineWidth, padding: space.space8, gap: space.space4 },
  blockHead: { flexDirection: "row", alignItems: "center", justifyContent: "space-between" },
  blockHeadRight: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  rawToggle: { borderTopWidth: StyleSheet.hairlineWidth, paddingHorizontal: space.space8, minHeight: 32, justifyContent: "center" },
  expander: { flexDirection: "row", alignItems: "center", gap: space.space4, paddingTop: space.space4, minHeight: 32 },
});
