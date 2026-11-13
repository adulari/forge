// "Local CI": the repository's GitHub workflows as the plan Forge runs before a goal finishes or a
// push happens (docs/features/ci-parity.md) - each step with its job, command and environment,
// every skip with its reason, the last result with the output tail of what failed, and a Run
// button. The host does the work; this only shows it.
import React, { useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import type { CiPlanRow } from "../../lib/apiCi";
import { CI_STATE_CHIP, ciDuration, ciGateNote, ciHeadline, ciSections } from "../../lib/ciModel";
import { useRunSessionCi, useSessionCi } from "../../lib/queriesCi";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { Button } from "../ds/Button";
import { Sheet } from "../ds/Sheet";

interface Props {
  visible: boolean;
  onClose: () => void;
  sessionId: string | null;
}

export function LocalCiSheet({ visible, onClose, sessionId }: Props) {
  const tokens = useTokens();
  const run = useRunSessionCi(sessionId);
  const plan = useSessionCi(sessionId, visible, run.isPending);
  // A failing step opens by default (its output tail is the point); a tap overrides either way.
  const [toggled, setToggled] = useState<Record<string, boolean>>({});
  const data = plan.data;
  // A run whose request failed client-side, or one started elsewhere, still holds the worktree.
  const onHost = data?.running != null;
  const busy = run.isPending || onHost;
  const { steps, skips } = ciSections(data);
  const failure = plan.error?.message ?? run.error?.message ?? null;

  return (
    <Sheet visible={visible && sessionId != null} onClose={onClose} accessibilityLabel="Local CI" snapPoints={[0.85]}>
      <ScrollView contentContainerStyle={styles.content} keyboardShouldPersistTaps="handled">
        <Text style={[typeScale.heading, { color: tokens.ink }]}>Local CI</Text>
        <Text
          style={[typeScale.sub, { color: failure || data?.ok === false ? tokens.danger : tokens.ink2 }]}
          accessibilityLiveRegion="polite"
          testID="local-ci-headline"
        >
          {failure ?? ciHeadline(data, busy)}
        </Text>
        {data?.summary && !busy ? (
          <Text style={[typeScale.meta, { color: tokens.ink3 }]}>{data.summary}</Text>
        ) : null}
        {ciGateNote(data) ? <Text style={[typeScale.meta, { color: tokens.ink3 }]}>{ciGateNote(data)}</Text> : null}
        <Button
          label={run.isPending ? "Running…" : onHost ? "Running on the host…" : "Run"}
          onPress={() => run.mutate()}
          disabled={busy || sessionId == null || steps.length === 0}
          testID="local-ci-run"
        />
        {steps.map((row) => (
          <StepRow
            key={row.key ?? row.name}
            row={row}
            expanded={toggled[row.key ?? row.name] ?? row.state === "fail"}
            onToggle={(id, next) => setToggled((prev) => ({ ...prev, [id]: next }))}
          />
        ))}
        {skips.length > 0 ? (
          <View style={styles.block}>
            <Text style={[typeScale.meta, { color: tokens.ink3 }]}>Skipped on this host</Text>
            {skips.map((row, index) => (
              <View key={`${row.name}-${index}`} style={styles.skip}>
                <Text style={[typeScale.body, { color: tokens.ink }]}>{row.name}</Text>
                <Text style={[typeScale.sub, { color: tokens.ink3 }]}>{row.detail}</Text>
              </View>
            ))}
          </View>
        ) : null}
      </ScrollView>
    </Sheet>
  );
}

function StepRow({
  row,
  expanded,
  onToggle,
}: {
  row: CiPlanRow;
  expanded: boolean;
  onToggle: (key: string, expanded: boolean) => void;
}) {
  const tokens = useTokens();
  const id = row.key ?? row.name;
  const chip = CI_STATE_CHIP[row.state];
  const took = ciDuration(row.secs);
  return (
    <View style={[styles.step, { borderColor: tokens.border }]}>
      <Pressable
        onPress={() => onToggle(id, !expanded)}
        accessibilityRole="button"
        accessibilityState={{ expanded }}
        accessibilityLabel={`${row.name}, ${chip.label}`}
        style={styles.stepHead}
      >
        <View style={styles.stepTitle}>
          <Text style={[typeScale.body, { color: tokens.ink }]} numberOfLines={expanded ? undefined : 1}>
            {row.name}
          </Text>
          <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]} numberOfLines={expanded ? undefined : 1}>
            {row.detail}
          </Text>
        </View>
        <Badge label={took ? `${chip.label} · ${took}` : chip.label} tone={chip.tone} />
      </Pressable>
      {expanded ? (
        <View style={styles.detail}>
          {row.job ? <Text style={[typeScale.meta, { color: tokens.ink3 }]}>Job: {row.job}</Text> : null}
          {row.command ? (
            <Text style={[typeScale.monoMeta, { color: tokens.ink2 }]} selectable testID="local-ci-command">
              {row.command}
            </Text>
          ) : null}
          {row.env && row.env.length > 0 ? (
            <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]} selectable testID="local-ci-env">
              {row.env.join("\n")}
            </Text>
          ) : null}
          {row.tail ? (
            <Text style={[typeScale.monoMeta, { color: tokens.danger }]} selectable testID="local-ci-tail">
              {row.tail}
            </Text>
          ) : null}
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  content: { padding: space.space16, gap: space.space12 },
  block: { gap: space.space8 },
  skip: { gap: space.space4 },
  step: { borderTopWidth: StyleSheet.hairlineWidth, paddingTop: space.space8, gap: space.space8 },
  stepHead: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  stepTitle: { flex: 1, gap: space.space4 },
  detail: { gap: space.space8 },
});
