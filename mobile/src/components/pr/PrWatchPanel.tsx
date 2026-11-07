import React, { useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import type { PrReviewAction, PrWatchMethod, PrWatchMode } from "../../lib/apiPr";
import { useShipGate } from "../../lib/prAccess";
import { usePrWatch, useReviewAction, useSetPrWatch } from "../../lib/queriesPr";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { Button } from "../ds/Button";
import { Segmented } from "../ds/Segmented";
import { useToast } from "../ds/ToastHost";
import { canMergeAnyway, findingPlace, numberedBlocking } from "./prReviewModel";
import { WATCH_METHODS, WATCH_MODES, watchStatus } from "./prWatchModel";

export function PrWatchPanel({ sessionId }: { sessionId: string }) {
  const tokens = useTokens();
  const toast = useToast();
  const query = usePrWatch(sessionId);
  const set = useSetPrWatch();
  const review = useReviewAction();
  const [confirming, setConfirming] = useState(false);
  const gate = useShipGate();
  const watch = query.data?.watch ?? null;
  const mode: PrWatchMode = watch?.mode ?? "off";
  const method: PrWatchMethod = watch?.method ?? "squash";
  const status = watchStatus(watch);

  const apply = (next: PrWatchMode, nextMethod: PrWatchMethod) => {
    if (!gate.allowed) return;
    set.mutate(
      { session: sessionId, mode: next, method: nextMethod },
      { onError: (e) => toast.show(e.message, { tone: "danger" }) },
    );
  };

  const owner = (body: PrReviewAction) => {
    review.mutate(
      { session: sessionId, by: "app", ...body },
      { onError: (e) => toast.show(e.message, { tone: "danger" }), onSettled: () => setConfirming(false) },
    );
  };

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
      {gate.message ? <Text style={[typeScale.monoMeta, { color: tokens.warn }]}>{gate.message}</Text> : null}
      {status.detail ? (
        <Text style={[typeScale.monoMeta, { color: status.tone === "danger" ? tokens.danger : tokens.ink3 }]}>{status.detail}</Text>
      ) : null}
      {numberedBlocking(watch?.review).map(({ n, finding }) => (
        <View key={`${n}:${finding.title}`} style={styles.finding}>
          <Text style={[typeScale.monoMeta, styles.flex, { color: tokens.warn }]}>
            {`${n}. [${finding.severity}] ${findingPlace(finding)} ${finding.title}`}
          </Text>
          <Button
            label="Dismiss"
            variant="ghost"
            disabled={review.isPending}
            onPress={() => owner({ action: "dismiss", finding: n })}
            accessibilityLabel={`Dismiss finding ${n}`}
          />
        </View>
      ))}
      {mode === "merge" && canMergeAnyway(watch?.review) ? (
        <Button
          label={confirming ? "Confirm: merge without a clean review" : "Merge anyway"}
          variant="danger"
          loading={review.isPending}
          onPress={() => (confirming ? owner({ action: "merge_anyway" }) : setConfirming(true))}
          accessibilityHint="Lets this pull request merge once, without waiting for the review of its current head"
        />
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  panel: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4, padding: space.space12, gap: space.space8 },
  head: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  flex: { flex: 1 },
  finding: { flexDirection: "row", alignItems: "center", gap: space.space8 },
});
