// One header for every Settings sub-page: back link, 18/700 title, optional trailing
// actions and a 12px ink3 subtitle. Replaces the per-page copies that drifted apart
// (different title styles, spacing and back-link labels).
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { BackLink } from "./BackLink";

export interface PageHeaderProps {
  title: React.ReactNode;
  subtitle?: React.ReactNode;
  /** Right-aligned actions on the title row (icon buttons, "+ Add"). */
  trailing?: React.ReactNode;
  /** Small inline meta next to the title (a count, a path). */
  titleMeta?: React.ReactNode;
  backLabel?: string;
  onBack?: () => void;
  /** Rendered under the subtitle (segmented controls, filters). */
  children?: React.ReactNode;
}

export function PageHeader({ title, subtitle, trailing, titleMeta, backLabel, onBack, children }: PageHeaderProps) {
  const tokens = useTokens();
  return (
    <View style={styles.header}>
      <BackLink label={backLabel} onPress={onBack} />
      <View style={styles.titleRow}>
        <View style={styles.titleCopy}>
          <Text accessibilityRole="header" style={[type.title, { color: tokens.ink }]}>{title}</Text>
          {titleMeta}
        </View>
        {trailing ? <View style={styles.trailing}>{trailing}</View> : null}
      </View>
      {subtitle ? <Text style={[type.sub, { color: tokens.ink3 }]}>{subtitle}</Text> : null}
      {children}
    </View>
  );
}

const styles = StyleSheet.create({
  header: { gap: space.space8 },
  titleRow: { flexDirection: "row", alignItems: "center", gap: space.space12 },
  titleCopy: { flex: 1, minWidth: 0, flexDirection: "row", alignItems: "baseline", gap: space.space8, flexWrap: "wrap" },
  trailing: { flexDirection: "row", alignItems: "center", gap: space.space8 },
});
