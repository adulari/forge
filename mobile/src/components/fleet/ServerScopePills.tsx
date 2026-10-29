// The one server control for the fleet: "All" plus one pill per paired server (status dot +
// waiting count). The Fleet screen and the desktop sidebar render this same component, and the
// desktop title-bar chip opens the same choices as a menu. Hidden with a single server, where
// there is nothing to choose.
import React from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { fleetStatuses, type ServerFleetStatus, type ServerReachability } from "../../lib/fleetStatuses";
import { useServerFleets } from "../../lib/queries";
import { useFleetScope } from "../../lib/useScopedFleet";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";

export { fleetStatuses, type ServerFleetStatus, type ServerReachability } from "../../lib/fleetStatuses";

/** Per-server reachability + waiting count, from the same shared per-server fleet poll. */
export function useServerFleetStatuses(): ServerFleetStatus[] {
  const { servers } = useFleetScope();
  return fleetStatuses(servers, useServerFleets(servers));
}

function Pill({
  label,
  selected,
  onPress,
  accessibilityLabel,
  dotColor,
  count,
  compact,
}: {
  label: string;
  selected: boolean;
  onPress: () => void;
  accessibilityLabel: string;
  dotColor?: string;
  count?: number;
  compact: boolean;
}) {
  const tokens = useTokens();
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel}
      accessibilityState={{ selected }}
      hitSlop={compact ? undefined : space.space8}
      style={({ pressed, hovered }: { pressed: boolean; hovered?: boolean }) => [
        styles.pill,
        compact && styles.pillCompact,
        {
          backgroundColor: selected ? tokens.selection : tokens.bg3,
          borderColor: selected ? "transparent" : hovered ? tokens.borderStrong : tokens.border,
          opacity: pressed ? 0.72 : 1,
        },
      ]}
    >
      {dotColor ? <View style={[styles.dot, { backgroundColor: dotColor }]} /> : null}
      <Text style={[typeScale.meta, styles.label, { color: selected ? tokens.accent : tokens.ink2 }]} numberOfLines={1}>
        {label}
      </Text>
      {count !== undefined && count > 0 ? (
        <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.danger, fontFamily: monoFamily.regular }]}>{count}</Text>
      ) : null}
    </Pressable>
  );
}

export function ServerScopePills({ layout = "scroll" }: { layout?: "scroll" | "wrap" }) {
  const tokens = useTokens();
  const { multiServer, showAll, activeServerId, selectAll, selectServer } = useFleetScope();
  const statuses = useServerFleetStatuses();
  if (!multiServer) return null;

  const compact = layout === "wrap";
  // A daemon paired twice (direct + relay) lists the same sessions twice; "All" shows them once.
  const totalWaiting = new Set(statuses.flatMap((status) => status.waitingIds)).size;
  const dotColor = (reachability: ServerReachability) =>
    reachability === "pending" ? tokens.warn : reachability === "reachable" ? tokens.success : tokens.danger;

  const pills = (
    <>
      <Pill
        label="All"
        selected={showAll}
        onPress={selectAll}
        accessibilityLabel={`All servers, ${totalWaiting} waiting`}
        count={totalWaiting}
        compact={compact}
      />
      {statuses.map((status) => (
        <Pill
          key={status.id}
          label={status.name}
          selected={!showAll && status.id === activeServerId}
          onPress={() => selectServer(status.id)}
          accessibilityLabel={`${status.name}, ${status.reachability === "unreachable" ? "unreachable" : "reachable"}, ${status.waiting} waiting`}
          dotColor={dotColor(status.reachability)}
          count={status.waiting}
          compact={compact}
        />
      ))}
    </>
  );

  if (compact) return <View style={styles.wrap}>{pills}</View>;
  return (
    <ScrollView horizontal showsHorizontalScrollIndicator={false} style={styles.scroll} contentContainerStyle={styles.scrollContent}>
      {pills}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  // Horizontal ScrollViews stretch on their cross-axis in a flex column on web; pin to content.
  scroll: { flexGrow: 0, flexShrink: 0 },
  scrollContent: { gap: space.space8, paddingTop: space.space12, paddingHorizontal: space.space16 },
  wrap: { flexDirection: "row", flexWrap: "wrap", gap: space.space4, marginTop: space.space8 },
  pill: {
    minHeight: 28,
    maxWidth: 220,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space4,
    paddingHorizontal: space.space12,
    borderRadius: radii.radiusPill,
    borderWidth: 1,
  },
  pillCompact: { minHeight: 22, maxWidth: "100%", paddingHorizontal: space.space8 },
  dot: { width: 6, height: 6, borderRadius: 3 },
  label: { flexShrink: 1 },
});
