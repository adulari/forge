// The headline and summary chips of one release: the top of the What's New panel and of the
// post-update sheet. Renders nothing for a release that has neither, so an older daemon or a plain
// release looks exactly as it did.
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { type ChangelogRelease } from "../../lib/api";
import { useTokens } from "../../theme/ThemeProvider";
import { hexToRgba, radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";

export function ReleaseHero({ release }: { release: Pick<ChangelogRelease, "headline" | "chips"> }) {
  const tokens = useTokens();
  const chips = release.chips ?? [];
  if (!release.headline && chips.length === 0) return null;
  return (
    <View
      style={[styles.hero, { backgroundColor: hexToRgba(tokens.accent, 0.08), borderColor: hexToRgba(tokens.accent, 0.35), borderRadius: radii.radius8 }]}
      testID="release-hero"
    >
      {release.headline ? (
        <Text style={[typeScale.headingBold, { color: tokens.ink }]} accessibilityRole="header">
          {release.headline}
        </Text>
      ) : null}
      {chips.length > 0 ? (
        <View style={styles.chips}>
          {chips.map((chip) => (
            <View key={chip} style={[styles.chip, { borderColor: hexToRgba(tokens.accent, 0.5), borderRadius: radii.radiusPill }]}>
              <Text style={[typeScale.meta, { color: tokens.accent }]} numberOfLines={1}>{chip}</Text>
            </View>
          ))}
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  hero: { borderWidth: 1, padding: space.space12, gap: space.space8 },
  chips: { flexDirection: "row", flexWrap: "wrap", gap: space.space4 },
  chip: { borderWidth: 1, paddingHorizontal: space.space8, paddingVertical: 2, maxWidth: "100%" },
});
