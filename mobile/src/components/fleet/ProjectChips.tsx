// A horizontal chip row that narrows the Fleet to one project. Hidden when there is only one.
import React from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import type { ProjectChip } from "../../lib/fleetProjects";
import { projectAccent } from "../../lib/sidebarModel";
import { useTheme } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";

export interface ProjectChipsProps {
  chips: readonly ProjectChip[];
  selected: string | null;
  onSelect: (project: string | null) => void;
}

export function ProjectChips({ chips, selected, onSelect }: ProjectChipsProps) {
  const { scheme, tokens } = useTheme();
  if (chips.length < 2) return null;
  return (
    <ScrollView horizontal showsHorizontalScrollIndicator={false} style={styles.scroll} contentContainerStyle={styles.content} accessibilityRole="tablist">
      {[{ name: null as string | null, label: "All projects", waiting: 0 }, ...chips.map((chip) => ({ name: chip.name as string | null, label: chip.name, waiting: chip.waiting }))].map((chip) => {
        const active = chip.name === selected;
        return (
          <Pressable
            key={chip.name ?? "all"}
            onPress={() => onSelect(chip.name)}
            accessibilityRole="tab"
            accessibilityState={{ selected: active }}
            accessibilityLabel={chip.name ? `Project ${chip.label}${chip.waiting ? `, ${chip.waiting} waiting` : ""}` : "All projects"}
            style={[styles.chip, { backgroundColor: active ? tokens.selection : tokens.bg3, borderColor: active ? "transparent" : tokens.border }]}
          >
            {chip.name ? <View style={[styles.swatch, { backgroundColor: projectAccent(chip.name, scheme) }]} /> : null}
            <Text style={[typeScale.meta, { color: active ? tokens.accent : tokens.ink2 }]} numberOfLines={1}>
              {chip.label}
            </Text>
            {chip.waiting > 0 ? (
              <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.danger, fontFamily: monoFamily.regular }]}>{chip.waiting}</Text>
            ) : null}
          </Pressable>
        );
      })}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  scroll: { flexGrow: 0, flexShrink: 0 },
  content: { gap: space.space8, paddingTop: space.space12, paddingHorizontal: space.space16 },
  chip: {
    minHeight: 28,
    maxWidth: 200,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space4,
    paddingHorizontal: space.space12,
    borderRadius: radii.radiusPill,
    borderWidth: 1,
  },
  swatch: { width: 7, height: 7, borderRadius: 2 },
});
