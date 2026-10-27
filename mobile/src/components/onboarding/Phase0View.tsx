// Phase 0: before any daemon exists. Install Forge, start it in the background, connect this app.
import { router } from "expo-router";
import { Flame } from "lucide-react-native";
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import type { Phase0Row } from "../../lib/onboarding/flow";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Button } from "../ds/Button";
import { StatusGlyph } from "./parts";

function glyph(status: Phase0Row["status"]): "done" | "running" | "failed" | "pending" | "skipped" {
  return status === "skipped" ? "done" : status;
}

export function Phase0View({ rows, onRetry }: { rows: Phase0Row[]; onRetry: () => void }) {
  const tokens = useTokens();
  const shown = rows.filter((r) => r.status !== "skipped");
  const failed = rows.find((r) => r.status === "failed");
  return (
    <View style={[styles.card, { backgroundColor: tokens.bg1, borderColor: tokens.border }]} testID="onboarding-phase0">
      <View style={styles.title}>
        <Flame size={26} strokeWidth={1.75} color={tokens.accent} />
        <Text style={[type.display, { color: tokens.ink }]}>Getting Forge ready</Text>
      </View>
      <Text style={[type.body, { color: tokens.ink2 }]}>
        This app is a window onto Forge, which runs on this computer. Forge is being installed and started for you. It takes about a minute.
      </Text>
      <View style={styles.rows}>
        {shown.map((row) => (
          <View key={row.key} style={styles.row} testID={`onboarding-phase0-${row.key}`}>
            <StatusGlyph status={glyph(row.status)} />
            <View style={styles.rowText}>
              <Text style={[type.bodyBold, { color: row.status === "pending" ? tokens.ink3 : tokens.ink }]}>{row.title}</Text>
              {row.message ? (
                <Text style={[type.sub, { color: row.status === "failed" ? tokens.danger : tokens.ink3 }]}>{row.message}</Text>
              ) : null}
              {row.status === "running" && row.fraction != null ? (
                <View style={[styles.track, { backgroundColor: tokens.bg3 }]}>
                  <View style={[styles.fill, { width: `${Math.round(row.fraction * 100)}%`, backgroundColor: tokens.accent }]} />
                </View>
              ) : null}
            </View>
          </View>
        ))}
      </View>
      {failed ? (
        <View style={styles.actions}>
          <Button label="Try again" onPress={onRetry} testID="onboarding-retry" />
          <Button label="Connect to an existing server" variant="ghost" onPress={() => router.replace("/connect")} />
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  card: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4, padding: space.space24, gap: space.space16 },
  title: { flexDirection: "row", alignItems: "center", gap: space.space12 },
  rows: { gap: space.space12 },
  row: { flexDirection: "row", alignItems: "flex-start", gap: space.space12 },
  rowText: { flex: 1, gap: space.space4 },
  track: { height: 4, borderRadius: 2, overflow: "hidden", marginTop: space.space4 },
  fill: { height: 4, borderRadius: 2 },
  actions: { flexDirection: "row", gap: space.space8, flexWrap: "wrap" },
});
