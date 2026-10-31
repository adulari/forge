import React, { useMemo } from "react";
import { StyleSheet, Text, View } from "react-native";

import { useRunningActions } from "../../lib/queriesProjectActions";
import { useTokens } from "../../theme/ThemeProvider";
import { hexToRgba, radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { StatusDot } from "../ds/StatusDot";
import { runningBySession, runningLabel } from "./portsModel";

/**
 * A pulsing dot (or a labelled chip) when a project action is running in the session. The list is
 * one host-wide query shared by every row, so adding it to a long list costs one request.
 */
export function ActionRunningIndicator({ sessionId, variant }: { sessionId: string; variant: "dot" | "chip" }) {
  const tokens = useTokens();
  const { data } = useRunningActions();
  const ids = useMemo(() => (data ? (runningBySession(data).get(sessionId) ?? []) : []), [data, sessionId]);
  if (ids.length === 0) return null;
  const label = runningLabel(ids);
  if (variant === "dot") {
    return (
      <View accessibilityLabel={label} accessibilityRole="image">
        <StatusDot state="busy" size={5} accessibilityLabel={label} />
      </View>
    );
  }
  return (
    <View
      accessibilityRole="image"
      accessibilityLabel={label}
      style={[styles.chip, { backgroundColor: hexToRgba(tokens.accent, 0.12) }]}
    >
      <StatusDot state="busy" size={5} accessibilityLabel={label} />
      <Text style={[typeScale.monoMeta, { color: tokens.accent }]} numberOfLines={1}>
        {ids.length > 1 ? `${ids.length} running` : ids[0]}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  chip: { flexDirection: "row", alignItems: "center", gap: space.space4, paddingHorizontal: space.space8, paddingVertical: 2, borderRadius: radii.radius4 },
});
