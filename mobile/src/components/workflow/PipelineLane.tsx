// New pattern 3 (pipeline lane). A horizontal stage ramp — one cell per declared phase,
// joined by arrows — that reads the run as a lint→fix→test-style pipeline. Each cell shows
// `phase · count` (count = the agent rows in that phase) with the running stage carrying the
// low-alpha accent border, plus a mono sub-line naming that stage's currently-running agent.
// The wire carries no per-stage source file, so the sub-line names the active agent (or the
// stage state) rather than a file path — see the workflow report's omissions.
import { ArrowRight } from "lucide-react-native";
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import type { PhaseGroup } from "./format";

function stageSubLabel(group: PhaseGroup): string {
  if (group.state === "running") {
    const current = group.rows.find((r) => !r.done);
    return current ? current.agent : "running";
  }
  return group.state; // "done" | "pending"
}

export function PipelineLane({ groups }: { groups: PhaseGroup[] }) {
  const tokens = useTokens();
  const stages = groups.filter((group) => !group.unknown);
  if (stages.length < 2) return null;

  const header = stages.map((group) => group.phase).join(" → ");

  return (
    <View style={styles.wrap}>
      <Text style={[typeScale.section, { color: tokens.ink4 }]}>{`pipeline lane · ${header}`}</Text>
      <View style={styles.lane}>
        {stages.map((group, index) => {
          const running = group.state === "running";
          return (
            <React.Fragment key={group.phase}>
              {index > 0 ? <ArrowRight size={12} strokeWidth={2} color={tokens.ink4} /> : null}
              <View
                style={[
                  styles.cell,
                  { backgroundColor: tokens.bg2, borderColor: running ? tokens.focusRing : tokens.border },
                ]}
              >
                <Text
                  style={[styles.stageLabel, tabularNums, { color: running ? tokens.accent : tokens.ink3 }]}
                  numberOfLines={1}
                >
                  {`${group.phase} · ${group.rows.length}`}
                </Text>
                <Text style={[styles.stageSub, { color: tokens.ink2 }]} numberOfLines={1}>
                  {stageSubLabel(group)}
                </Text>
              </View>
            </React.Fragment>
          );
        })}
      </View>
      <Text style={[styles.caption, { color: tokens.ink4 }]}>items flow between stages without a barrier</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { marginTop: space.space24 },
  lane: { flexDirection: "row", alignItems: "center", gap: space.space8, marginTop: space.space8 },
  cell: {
    flex: 1,
    minWidth: 0,
    minHeight: 44,
    borderWidth: 1,
    borderRadius: radii.radius8,
    paddingHorizontal: 9,
    paddingVertical: 6,
    justifyContent: "center",
  },
  stageLabel: { fontFamily: monoFamily.regular, fontSize: 10 },
  stageSub: { fontFamily: monoFamily.regular, fontSize: 10.5, marginTop: 2 },
  caption: { fontSize: 11.5, marginTop: 7 },
});
