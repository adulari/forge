// DESIGN_SYSTEM.md §6 Status & data: `CostMetric` — tabular numerals, success
// color, count-up via `useCountUp` (§5.2 Gaugeflow), formatted with `formatCost`.
import React from "react";
import { Text } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { useCountUp } from "../../theme/motion";
import { formatCost, tabularNums, type as typeScale } from "../../theme/typography";

export interface CostMetricProps {
  valueUsd: number;
  /** Typographic scale to render at — callers embed this in different contexts
   *  (AgentCard row vs. SessionCard third line vs. a standalone stat). */
  variant?: "meta" | "sub" | "bodyBold";
}

export function CostMetric({ valueUsd, variant = "meta" }: CostMetricProps) {
  const tokens = useTokens();
  const display = useCountUp(valueUsd);
  if (!(valueUsd > 0)) return null;

  return (
    <Text
      style={[typeScale[variant], tabularNums, { color: tokens.success }]}
      numberOfLines={1}
      accessibilityRole="text"
      accessibilityLabel={`cost ${formatCost(valueUsd)}`}
    >
      {formatCost(display)}
    </Text>
  );
}
