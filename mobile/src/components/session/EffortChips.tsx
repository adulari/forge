import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { EFFORT_OPTIONS, type EffortChoice } from "../../lib/newSessionDefaults";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Chip } from "../ds/Chip";

export function EffortChips({ value, onChange }: { value: EffortChoice; onChange: (value: EffortChoice) => void }) {
  const tokens = useTokens();
  return (
    <View style={styles.wrap}>
      <Text style={[typeScale.body, { color: tokens.ink }]}>Effort</Text>
      <View style={styles.row} accessibilityRole="radiogroup" accessibilityLabel="Effort">
        {EFFORT_OPTIONS.map((option) => (
          <Chip
            key={option.value}
            label={option.label}
            selected={option.value === value}
            onPress={() => onChange(option.value)}
            size="compact"
          />
        ))}
      </View>
      <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
        {value === "auto" ? "Forge picks the thinking effort per turn." : `Starts the session at ${value} effort.`}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { gap: space.space8, paddingVertical: space.space12 },
  row: { flexDirection: "row", flexWrap: "wrap", gap: space.space8 },
});
