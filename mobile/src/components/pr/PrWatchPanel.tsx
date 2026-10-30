import React from "react";
import { StyleSheet, Text, View } from "react-native";

import type { PrWatchMethod, PrWatchMode } from "../../lib/apiPr";
import { usePrWatch, useSetPrWatch } from "../../lib/queriesPr";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { Segmented } from "../ds/Segmented";
import { useToast } from "../ds/ToastHost";
import { WATCH_METHODS, WATCH_MODES, watchStatus } from "./prWatchModel";

export function PrWatchPanel({ sessionId }: { sessionId: string }) {
  const tokens = useTokens();
  const toast = useToast();
  const query = usePrWatch(sessionId);
  const set = useSetPrWatch();
  const watch = query.data?.watch ?? null;
  const mode: PrWatchMode = watch?.mode ?? "off";
  const method: PrWatchMethod = watch?.method ?? "squash";
  const status = watchStatus(watch);

  const apply = (next: PrWatchMode, nextMethod: PrWatchMethod) =>
    set.mutate(
      { session: sessionId, mode: next, method: nextMethod },
      { onError: (e) => toast.show(e.message, { tone: "danger" }) },
    );

  return (
    <View style={[styles.panel, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}>
      <View style={styles.head}>
        <Text style={[typeScale.meta, styles.flex, { color: tokens.ink2 }]}>PR Watch</Text>
        {mode !== "off" ? <Badge label={status.label} tone={status.tone} /> : null}
      </View>
      <Segmented options={WATCH_MODES} value={mode} onChange={(next) => apply(next, method)} />
      {mode === "merge" ? (
        <Segmented options={WATCH_METHODS} value={method} onChange={(next) => apply(mode, next)} />
      ) : null}
      <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>
        {mode === "off"
          ? "Forge fixes failing checks and new review comments, and can merge when green. Pushes are never forced."
          : "Runs in the Auto profile: normal pushes are pre-approved, force pushes are refused."}
      </Text>
      {status.detail ? (
        <Text style={[typeScale.monoMeta, { color: status.tone === "danger" ? tokens.danger : tokens.ink3 }]}>{status.detail}</Text>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  panel: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4, padding: space.space12, gap: space.space8 },
  head: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  flex: { flex: 1 },
});
