// Progress of a backlog drain: the daemon's one-line summary, the spend envelope, and one row per
// issue. Shown at the top of the Issues view while a drain is going and until the next one starts.
import { Layers } from "lucide-react-native";
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import type { DrainItemStatus, IssueDrain } from "../../lib/apiIssues";
import { openSession } from "../../lib/nav";
import { isLiveDrain, useCancelDrain } from "../../lib/queriesIssues";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Badge, type BadgeTone } from "../ds/Badge";
import { Button } from "../ds/Button";
import { ListRow } from "../ds/ListRow";
import { useToast } from "../ds/ToastHost";
import { DRAIN_ITEM_LABEL } from "./issuesModel";

const TONE: Record<DrainItemStatus, BadgeTone> = {
  queued: "neutral",
  running: "accent",
  waiting: "warn",
  pr_open: "accent",
  merged: "success",
  done: "neutral",
  failed: "danger",
  blocked: "danger",
  cancelled: "neutral",
};

export function DrainCard({ drain }: { drain: IssueDrain }) {
  const tokens = useTokens();
  const toast = useToast();
  const cancel = useCancelDrain();
  const live = isLiveDrain(drain);

  return (
    <View
      style={[styles.card, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}
      accessibilityLabel={drain.line}
    >
      <View style={styles.head}>
        <Layers size={16} color={tokens.ink3} />
        <Text style={[typeScale.sub, styles.flex, { color: tokens.ink }]}>{drain.line}</Text>
        {drain.status !== "active" ? (
          <Badge label={drain.status} tone={drain.status === "paused" ? "warn" : "neutral"} lowercase />
        ) : null}
      </View>
      {drain.envelope ? (
        <View style={styles.meter}>
          <View style={[styles.track, { backgroundColor: tokens.bg3 }]}>
            <View
              style={[
                styles.fill,
                {
                  width: `${Math.round(drain.envelope.fill * 100)}%`,
                  backgroundColor: drain.envelope.spent ? tokens.danger : tokens.accent,
                },
              ]}
            />
          </View>
          <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>{drain.envelope.meter}</Text>
        </View>
      ) : null}
      {drain.status === "paused" && drain.paused_reason ? (
        <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>{drain.paused_reason}</Text>
      ) : null}
      {drain.items.map((item, i) => (
        <ListRow
          key={item.number}
          title={`#${item.number} ${item.title?.replace(/^#\d+\s*/, "") ?? ""}`.trim()}
          subtitle={item.outcome ?? undefined}
          trailing={<Badge label={DRAIN_ITEM_LABEL[item.status]} tone={TONE[item.status]} lowercase />}
          onPress={item.session_id ? () => openSession(item.session_id as string) : undefined}
          showSeparator={i < drain.items.length - 1}
          accessibilityLabel={`Issue ${item.number}: ${DRAIN_ITEM_LABEL[item.status]}`}
        />
      ))}
      {live ? (
        <Button
          label="Stop drain"
          variant="danger"
          loading={cancel.isPending}
          onPress={() =>
            cancel.mutate(drain.id, {
              onSuccess: () => toast.show("Drain stopped. Sessions already running keep going."),
              onError: (e) => toast.show(e.message, { tone: "danger" }),
            })
          }
          fullWidth
        />
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1 },
  card: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius4,
    padding: space.space12,
    gap: space.space8,
  },
  head: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  meter: { gap: space.space4 },
  track: { height: 4, borderRadius: radii.radius4, overflow: "hidden" },
  fill: { height: 4 },
});
