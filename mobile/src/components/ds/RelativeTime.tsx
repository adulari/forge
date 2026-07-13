// DESIGN_SYSTEM.md §6 Status & data: `RelativeTime` — self-refreshing every 30s,
// meta style, `12s · 4m · 2h · 3d` via `formatRelativeTime` (§2).
import React from "react";
import { Text, type TextStyle } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { formatRelativeTime, type as typeScale } from "../../theme/typography";
import { useRelativeClock } from "./relativeClock";

export interface RelativeTimeProps {
  timestampMs: number;
  style?: TextStyle;
}

export function RelativeTime({ timestampMs, style }: RelativeTimeProps) {
  const tokens = useTokens();
  useRelativeClock();
  const label = formatRelativeTime(timestampMs);

  return (
    <Text
      style={[typeScale.meta, { color: tokens.ink3 }, style]}
      numberOfLines={1}
      accessibilityRole="text"
      accessibilityLabel={`${label} ago`}
    >
      {label}
    </Text>
  );
}
