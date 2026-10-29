// The follow-up queue above the composer, mirroring the terminal's queue band: every prompt sent
// while a turn runs stays visible with what can still be done to it — pull it back to edit,
// move it earlier or later, run it now (stops the turn, like Esc in the terminal), or drop it.
// Prompts held on this device while the socket is down show the same rows, minus "run now".
import { ArrowDown, ArrowUp, ChevronDown, ChevronUp, Clock, Pencil, X, Zap } from "lucide-react-native";
import React, { useState } from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";

import { haptics } from "../../lib/haptics";
import type { QueuedPrompt } from "../../lib/offlineQueue";
import {
  mergeIntoDraft,
  moveInList,
  moveInput,
  moveTarget,
  type QueueRow,
  queueRows,
  removeInput,
  sendNowInput,
} from "../../lib/queueEdit";
import { useSessionStable } from "../../lib/sessionContext";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { MiniPressable } from "./MiniPressable";

export interface QueueStripProps {
  queued: readonly string[];
  offline: readonly QueuedPrompt[];
  online: boolean;
  /** The daemon understands `queue_move` / `queue_send_now`; an older one only dequeues. */
  canReorder: boolean;
  onOfflineChange: (next: QueuedPrompt[]) => void;
  /** A prompt left the queue without being sent (removed, or pulled back to edit). */
  onDropped: (text: string) => void;
}

function RowAction({
  icon: Icon,
  label,
  onPress,
  disabled = false,
  tone = "default",
}: {
  icon: typeof X;
  label: string;
  onPress: () => void;
  disabled?: boolean;
  tone?: "default" | "accent";
}) {
  const tokens = useTokens();
  return (
    <MiniPressable
      onPress={disabled ? undefined : onPress}
      disabled={disabled}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled }}
      hitSlop={{ top: 6, bottom: 6, left: 1, right: 1 }}
      style={[styles.action, { opacity: disabled ? 0.35 : 1 }]}
    >
      <Icon size={14} strokeWidth={1.75} color={tone === "accent" ? tokens.accent : tokens.ink3} />
    </MiniPressable>
  );
}

export function QueueStrip({ queued, offline, online, canReorder, onOfflineChange, onDropped }: QueueStripProps) {
  const tokens = useTokens();
  const { send, getDraftText, setDraftText, focusComposer } = useSessionStable();
  const [open, setOpen] = useState(true);
  const rows = queueRows(queued, offline);
  if (rows.length === 0) return null;

  const pullBack = (text: string) => {
    setDraftText(mergeIntoDraft(getDraftText(), text));
    focusComposer();
  };

  const remove = (row: QueueRow) => {
    if (row.source === "server") {
      if (!send(removeInput(row))) return;
    } else {
      onOfflineChange(offline.filter((_, i) => i !== row.index));
    }
    onDropped(row.text);
    haptics.select();
  };

  const edit = (row: QueueRow) => {
    if (row.source === "server") {
      if (!send(removeInput(row))) return;
    } else {
      onOfflineChange(offline.filter((_, i) => i !== row.index));
    }
    onDropped(row.text);
    pullBack(row.text);
  };

  const move = (row: QueueRow, delta: -1 | 1) => {
    const length = row.source === "server" ? queued.length : offline.length;
    const to = moveTarget(row.index, delta, length);
    if (to === null) return;
    if (row.source === "server") send(moveInput(row, to));
    else onOfflineChange(moveInList(offline, row.index, to));
    haptics.select();
  };

  const sendNow = (row: QueueRow) => {
    if (send(sendNowInput(row))) haptics.select();
  };

  const serverCount = queued.length;
  return (
    <View
      style={[styles.strip, { backgroundColor: tokens.bg1, borderColor: tokens.border }]}
      accessibilityLabel={`${rows.length} queued ${rows.length === 1 ? "prompt" : "prompts"}`}
      testID="queue-strip"
    >
      <MiniPressable
        onPress={() => setOpen((v) => !v)}
        accessibilityRole="button"
        accessibilityState={{ expanded: open }}
        accessibilityLabel={open ? "Collapse the queue" : "Expand the queue"}
        wash={false}
        style={styles.header}
      >
        <Text style={[type.monoMeta, { color: tokens.ink2 }]}>{rows.length} queued</Text>
        <Text style={[type.meta, styles.headerHint, { color: tokens.ink3 }]} numberOfLines={1}>
          {online ? "sent at the next step or after this turn" : "will send on reconnect"}
        </Text>
        {open ? <ChevronDown size={14} strokeWidth={1.75} color={tokens.ink3} /> : <ChevronUp size={14} strokeWidth={1.75} color={tokens.ink3} />}
      </MiniPressable>
      {open ? (
        <ScrollView style={styles.list} keyboardShouldPersistTaps="handled" nestedScrollEnabled>
          {rows.map((row) => {
            const listLength = row.source === "server" ? serverCount : offline.length;
            const reorderable = row.source === "offline" || (online && canReorder);
            return (
              <View key={row.key} style={styles.row} testID={`queue-row-${row.key}`}>
                {row.source === "offline" ? (
                  <Clock size={12} strokeWidth={1.75} color={tokens.ink3} />
                ) : (
                  <Text style={[type.monoMeta, styles.index, { color: tokens.ink4 }]}>{row.index + 1}</Text>
                )}
                <Text style={[type.sub, styles.text, { color: tokens.ink }]} numberOfLines={2}>
                  {row.text}
                </Text>
                <View style={styles.actions}>
                  <RowAction
                    icon={ArrowUp}
                    label={`move "${row.text}" earlier`}
                    onPress={() => move(row, -1)}
                    disabled={!reorderable || row.index === 0}
                  />
                  <RowAction
                    icon={ArrowDown}
                    label={`move "${row.text}" later`}
                    onPress={() => move(row, 1)}
                    disabled={!reorderable || row.index >= listLength - 1}
                  />
                  <RowAction icon={Pencil} label={`edit "${row.text}"`} onPress={() => edit(row)} disabled={row.source === "server" && !online} />
                  {row.source === "server" && canReorder ? (
                    <RowAction icon={Zap} label={`send "${row.text}" now`} onPress={() => sendNow(row)} disabled={!online} tone="accent" />
                  ) : null}
                  <RowAction icon={X} label={`remove "${row.text}" from the queue`} onPress={() => remove(row)} disabled={row.source === "server" && !online} />
                </View>
              </View>
            );
          })}
        </ScrollView>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  strip: {
    marginHorizontal: space.space12,
    marginBottom: space.space4,
    borderWidth: StyleSheet.hairlineWidth * 2,
    borderRadius: radii.radius8,
    overflow: "hidden",
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space8,
    paddingVertical: space.space4,
    borderRadius: radii.radius8,
  },
  headerHint: { flex: 1, minWidth: 0 },
  list: { maxHeight: 156 },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingLeft: space.space8,
    paddingRight: space.space4,
    paddingVertical: space.space4,
  },
  index: { width: 14, textAlign: "right" },
  text: { flex: 1, minWidth: 0 },
  actions: { flexDirection: "row", alignItems: "center", flexShrink: 0 },
  action: {
    width: 26,
    height: 26,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radii.radius4,
  },
});
