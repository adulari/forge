// Machined desktop shell — per-provider usage-pace ring. Shared by Sidebar's footer
// row, IconRail's collapsed rail, WebTopBar's combined chip, and UsageDock: one small
// SVG arc per provider (2px stroke, r 6.5) reading the SAME quota data app/usage.tsx
// renders as bars — `useProviderPace()` is the single place that turns `useUsage()`'s
// `quota` rows into a flat 0-100 "how full is the binding window" number per provider,
// so every ring/bar in the shell agrees with the Usage screen instead of drifting.
import React, { useMemo } from "react";
import { StyleSheet, View } from "react-native";
import Svg, { Circle } from "react-native-svg";

import { useSessions, useUsage } from "../../lib/queries";
import { useTheme } from "../../theme/ThemeProvider";
import type { ColorTokens } from "../../theme/tokens";

export interface ProviderPace {
  provider: string;
  /** The fullest (binding) quota window fraction for this provider, 0-100. */
  pct: number;
}

export interface ProviderPaceInfo {
  rings: ProviderPace[];
  /** The single worst-case pct across all subscription providers, or null with none. */
  combinedPct: number | null;
}

/** Reads the same `useUsage()` data app/usage.tsx renders as bars — never invents a
 * number the Usage screen doesn't already show. Subscription (bridge/oauth) windows
 * only: metered API providers have no "% of quota" to ring. */
export function useProviderPace(): ProviderPaceInfo {
  const { data: sessions } = useSessions();
  const sessionId = sessions?.find((s) => s.busy)?.id ?? sessions?.[0]?.id;
  const query = useUsage(sessionId);
  const quotaRows = query.data?.quota;

  const rings = useMemo<ProviderPace[]>(() => {
    const byProvider = new Map<string, number>();
    for (const q of quotaRows ?? []) {
      if (q.kind === "api" || q.fraction == null) continue;
      const pct = Math.max(0, Math.min(100, Math.round(q.fraction * 100)));
      byProvider.set(q.provider, Math.max(byProvider.get(q.provider) ?? 0, pct));
    }
    return [...byProvider.entries()].map(([provider, pct]) => ({ provider, pct }));
  }, [quotaRows]);

  const combinedPct = rings.length > 0 ? Math.max(...rings.map((r) => r.pct)) : null;

  return { rings, combinedPct };
}

/** Usage rings never borrow the brand accent (that's reserved for "forging now") —
 * neutral ink2 below the pace thresholds, warn/danger above, same steps as the Usage
 * screen's bar coloring. */
export function usagePaceColor(pct: number, tokens: ColorTokens): string {
  if (pct >= 90) return tokens.danger;
  if (pct >= 70) return tokens.warn;
  return tokens.ink2;
}

export interface UsageRingProps {
  pct: number;
  size?: number;
  strokeWidth?: number;
  color?: string;
  accessibilityLabel?: string;
}

export function UsageRing({ pct, size = 16, strokeWidth = 2, color, accessibilityLabel }: UsageRingProps) {
  const { scheme, tokens } = useTheme();
  const trackColor = scheme === "light" ? "rgba(0,0,0,0.08)" : "rgba(244,244,246,0.08)";
  const fillColor = color ?? usagePaceColor(pct, tokens);
  const r = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * r;
  const clamped = Math.max(0, Math.min(100, pct));
  const dash = (clamped / 100) * circumference;

  return (
    <View
      style={styles.wrap}
      accessibilityRole="progressbar"
      accessibilityValue={{ min: 0, max: 100, now: Math.round(clamped) }}
      accessibilityLabel={accessibilityLabel ?? `usage ${Math.round(clamped)}%`}
    >
      <Svg width={size} height={size} viewBox={`0 0 ${size} ${size}`}>
        <Circle cx={size / 2} cy={size / 2} r={r} stroke={trackColor} strokeWidth={strokeWidth} fill="none" />
        <Circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          stroke={fillColor}
          strokeWidth={strokeWidth}
          fill="none"
          strokeDasharray={`${dash} ${circumference}`}
          strokeLinecap="round"
          rotation={-90}
          origin={`${size / 2}, ${size / 2}`}
        />
      </Svg>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { alignItems: "center", justifyContent: "center" },
});
