// Workflow result view — reachable once a workflow has finished (present but not active).
// Shows the outcome banner, the return value (structured-output block when the summary embeds
// JSON, otherwise prose), and a per-phase agent count + cost derived from the rows. Per-phase
// DURATIONS and token totals are intentionally absent: the wire carries neither.
import { Check, X } from "lucide-react-native";
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import type { SnapshotSubagent, SnapshotWorkflow } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { formatCost, monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { extractJson, groupByPhase, totalCost } from "./format";
import { StructuredOutput } from "./StructuredOutput";

function SectionLabel({ children }: { children: string }) {
  const tokens = useTokens();
  return <Text style={[typeScale.section, styles.sectionLabel, { color: tokens.ink4 }]}>{children}</Text>;
}

export function WorkflowResult({
  workflow,
  rows,
}: {
  workflow: SnapshotWorkflow;
  rows: SnapshotSubagent[];
}) {
  const tokens = useTokens();
  const ok = workflow.finished_ok;
  const groups = groupByPhase(rows, workflow.phases);
  const summaryJson = extractJson(workflow.summary);
  const cost = totalCost(rows);

  const bannerBg = ok === false ? tokens.dangerBg : ok === true ? tokens.successBg : tokens.bg3;
  const bannerInk = ok === false ? tokens.danger : ok === true ? tokens.success : tokens.ink2;
  const bannerText =
    ok === true ? "Workflow completed" : ok === false ? "Workflow failed" : "Workflow finished";

  return (
    <View>
      <View style={[styles.banner, { backgroundColor: bannerBg }]}>
        {ok === false ? (
          <X size={16} strokeWidth={2.5} color={bannerInk} />
        ) : (
          <Check size={16} strokeWidth={2.5} color={bannerInk} />
        )}
        <Text style={[typeScale.bodyBold, styles.bannerText, { color: bannerInk }]}>{bannerText}</Text>
      </View>

      {summaryJson != null ? (
        <>
          <SectionLabel>return value</SectionLabel>
          <StructuredOutput data={summaryJson} label="return value" />
        </>
      ) : workflow.summary ? (
        <>
          <SectionLabel>summary</SectionLabel>
          <Text style={[typeScale.body, styles.summary, { color: tokens.ink2 }]}>{workflow.summary}</Text>
        </>
      ) : (
        <>
          <SectionLabel>summary</SectionLabel>
          <Text style={[typeScale.sub, styles.summary, { color: tokens.ink3 }]}>
            This run recorded no return value.
          </Text>
        </>
      )}

      <SectionLabel>phases</SectionLabel>
      <View>
        {groups.map((group, index) => (
          <View
            key={group.phase}
            style={[
              styles.phaseRow,
              index < groups.length - 1 ? { borderBottomColor: tokens.hairline, borderBottomWidth: StyleSheet.hairlineWidth } : null,
            ]}
          >
            <Text style={[typeScale.body, styles.phaseName, { color: tokens.ink2 }]} numberOfLines={1}>
              {group.unknown ? "unphased" : group.phase}
            </Text>
            <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3 }]}>
              {`${group.rows.length} agents`}
              {group.cost > 0 ? <Text style={{ color: tokens.success }}>{` · ${formatCost(group.cost)}`}</Text> : null}
            </Text>
          </View>
        ))}
      </View>

      <SectionLabel>totals</SectionLabel>
      <View style={styles.totals}>
        <View style={styles.total}>
          <Text style={[styles.totalValue, tabularNums, { color: tokens.ink }]}>{rows.length}</Text>
          <Text style={[typeScale.section, { color: tokens.ink3 }]}>agents</Text>
        </View>
        <View style={[styles.total, styles.totalDivider, { borderLeftColor: tokens.border }]}>
          <Text style={[styles.totalValue, tabularNums, { color: tokens.success }]}>{formatCost(cost)}</Text>
          <Text style={[typeScale.section, { color: tokens.ink3 }]}>cost</Text>
        </View>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  banner: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space16,
    paddingVertical: space.space12,
    borderRadius: radii.radius12,
    marginTop: space.space8,
  },
  bannerText: { flex: 1 },
  sectionLabel: { marginTop: space.space20, marginBottom: space.space4 },
  summary: { marginTop: space.space4 },
  phaseRow: {
    minHeight: 44,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: space.space12,
  },
  phaseName: { flex: 1 },
  totals: { flexDirection: "row", marginTop: space.space8 },
  total: { flex: 1, gap: space.space4 },
  totalDivider: { borderLeftWidth: StyleSheet.hairlineWidth, paddingLeft: space.space16 },
  totalValue: { fontFamily: monoFamily.bold, fontSize: 17, fontWeight: "700" },
});
