// What this session has cost: the total, then each model and each subagent with its tokens and
// list-price estimate. Reads the host's per-session spend; hidden until the session has called a
// provider, and when the host has no such route.
import { Coins } from "lucide-react-native";
import React from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";

import type { SessionSpend, SpendTotals } from "../../lib/api";
import { hasSpend, spendDetail, spendHeadline, totalTokens } from "../../lib/spendDock";
import { compactTokens } from "../../lib/goalState";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";
import type { MenuEntry } from "../shell/PopoverMenu";
import { DockFrame } from "./DockFrame";

function SpendRow({ label, totals }: { label: string; totals: SpendTotals }) {
  const tokens = useTokens();
  return (
    <View style={styles.row} accessibilityLabel={`${label}, ${spendHeadline(totals)}, ${spendDetail(totals)}`}>
      <View style={styles.line}>
        <Text style={[typeScale.sub, styles.label, { color: tokens.ink }]} numberOfLines={1}>
          {label}
        </Text>
        <Text style={[styles.amount, { color: tokens.ink2 }]}>{spendHeadline(totals)}</Text>
      </View>
      <Text style={[styles.detail, { color: tokens.ink3 }]} numberOfLines={1}>
        {spendDetail(totals)}
      </Text>
    </View>
  );
}

export interface SpendDockProps {
  spend: SessionSpend | undefined;
  collapsed: boolean;
  onToggle: () => void;
  menu: readonly MenuEntry[];
}

export function SpendDock({ spend, collapsed, onToggle, menu }: SpendDockProps) {
  const tokens = useTokens();
  if (!spend || !hasSpend(spend.total)) return null;
  return (
    <DockFrame
      title="Spend"
      icon={Coins}
      meta={spendHeadline(spend.total)}
      headline={`${compactTokens(totalTokens(spend.total))} tok`}
      collapsed={collapsed}
      onToggle={onToggle}
      menu={menu}
      testID="spend-dock"
    >
      <ScrollView style={styles.body} nestedScrollEnabled keyboardShouldPersistTaps="handled">
        <SpendRow label={spend.models.length === 1 ? spend.models[0].model : "This session"} totals={spend.total} />
        {spend.models.length > 1 ? spend.models.map((row) => <SpendRow key={row.model} label={row.model} totals={row} />) : null}
        {spend.subagents.length > 0 ? (
          <>
            <Text style={[typeScale.section, styles.section, { color: tokens.ink4 }]}>Subagents</Text>
            {spend.subagents.map((row) => (
              <SpendRow key={row.session_id} label={row.label || row.model || "subagent"} totals={row} />
            ))}
          </>
        ) : null}
      </ScrollView>
    </DockFrame>
  );
}

const styles = StyleSheet.create({
  body: { maxHeight: 168 },
  row: { paddingHorizontal: space.space12, paddingVertical: space.space4 },
  line: { flexDirection: "row", alignItems: "baseline", gap: space.space8 },
  label: { flex: 1, minWidth: 0 },
  amount: { fontFamily: monoFamily.regular, fontSize: 12, lineHeight: 16 },
  detail: { fontFamily: monoFamily.regular, fontSize: 11, lineHeight: 15 },
  section: { paddingHorizontal: space.space12, paddingTop: space.space8 },
});
