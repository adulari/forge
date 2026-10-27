// A background job's completion notice, one quiet line like a tool row: status glyph, what ran
// (the program, never its command line) and, only when it did not succeed, how it ended. Tap or
// click opens the full command (wrapped, monospace, home as `~`), the job id, its log path with a
// copy button and the tail of its output when the notice carries one. TUI parity:
// crates/forge-tui/src/app/job_notice.rs.
import { Check, ChevronDown, ChevronRight, X } from "lucide-react-native";
import React, { useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { haptics } from "../../lib/haptics";
import {
  jobCommandText,
  jobLabel,
  jobLogPath,
  jobOutcome,
  parseJobNotices,
  type JobNoticeItem,
} from "../../lib/jobNotice";
import { shortenPath, tildify } from "../../lib/pathDisplay";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";
import { CopyButton, DetailBlock } from "./ToolCallRow";

type ExitNotice = Extract<JobNoticeItem, { type: "exit" }>;

function JobExitRow({ notice }: { notice: ExitNotice }) {
  const tokens = useTokens();
  const [open, setOpen] = useState(false);
  const label = useMemo(() => jobLabel(notice.command), [notice.command]);
  const command = useMemo(() => jobCommandText(notice.command), [notice.command]);
  const logPath = useMemo(() => jobLogPath(notice.command), [notice.command]);
  const outcome = jobOutcome(notice.code);
  const ok = notice.code === 0;
  const tail = notice.tail.map(tildify).join("\n");
  const accent = ok ? tokens.ink3 : tokens.danger;
  return (
    <View style={styles.row}>
      <View style={[styles.card, open ? { borderColor: tokens.border, backgroundColor: tokens.bg1 } : styles.flat]}>
        <Pressable
          onPress={() => {
            haptics.select();
            setOpen((v) => !v);
          }}
          accessibilityRole="button"
          accessibilityLabel={`Background job ${label}, ${ok ? "done" : (outcome ?? "failed")}, ${open ? "hide" : "show"} details`}
          accessibilityState={{ expanded: open }}
          style={({ pressed }) => [styles.summary, pressed ? { backgroundColor: tokens.bg2 } : null]}
          hitSlop={4}
        >
          {open ? (
            <ChevronDown size={13} strokeWidth={2} color={tokens.ink4} />
          ) : (
            <ChevronRight size={13} strokeWidth={2} color={tokens.ink4} />
          )}
          {ok ? (
            <Check size={13} strokeWidth={2} color={tokens.success} />
          ) : (
            <X size={13} strokeWidth={2} color={tokens.danger} />
          )}
          <Text style={[type.meta, styles.kind, { color: accent }]} numberOfLines={1}>
            Job
          </Text>
          <Text
            style={[type.codeSmall, styles.label, { color: tokens.ink2, fontFamily: monoFamily.regular }]}
            numberOfLines={1}
          >
            {label}
          </Text>
          {outcome ? <Text style={[type.meta, { color: tokens.danger }]}>{outcome}</Text> : null}
        </Pressable>
        {open ? (
          <View>
            <DetailBlock label="command" text={command} />
            <View style={[styles.block, { borderTopColor: tokens.border }]}>
              <Text style={[type.monoMeta, { color: tokens.ink3 }]} selectable>
                {`job ${notice.pid} · ${ok ? "exited 0" : (outcome ?? "failed")}`}
              </Text>
            </View>
            {logPath ? (
              <View style={[styles.block, styles.logRow, { borderTopColor: tokens.border }]}>
                <Text
                  style={[type.codeSmall, styles.logPath, { color: tokens.ink2, fontFamily: monoFamily.regular }]}
                  selectable
                >
                  {`log  ${shortenPath(logPath, 72)}`}
                </Text>
                <CopyButton text={logPath} />
              </View>
            ) : null}
            {tail ? <DetailBlock label="output" text={tail} /> : null}
          </View>
        ) : null}
      </View>
    </View>
  );
}

export function JobNoticeRows({ content }: { content: string }) {
  const tokens = useTokens();
  const items = useMemo(() => parseJobNotices(content), [content]);
  return (
    <View>
      {items.map((item, index) =>
        item.type === "dropped" ? (
          <View key={index} style={styles.row}>
            <Text style={[type.meta, styles.dropped, { color: tokens.ink3 }]}>
              {`${item.count} earlier job notice${item.count === 1 ? "" : "s"} dropped`}
            </Text>
          </View>
        ) : (
          <JobExitRow key={index} notice={item} />
        ),
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  row: { paddingLeft: space.space16, paddingRight: space.space16, paddingVertical: 1 },
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
  kind: { flexShrink: 0 },
  label: { flex: 1, minWidth: 0 },
  block: { borderTopWidth: StyleSheet.hairlineWidth, padding: space.space8 },
  logRow: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: space.space8 },
  logPath: { flex: 1, minWidth: 0 },
  dropped: { paddingHorizontal: space.space8, paddingVertical: space.space4 },
});
