// Forge Anywhere sync status row — mono glyph + text, per the design's "sync glyph + text"
// legend (mobile.dc.html lines 1315-1326). Most rows only color the glyph and leave the
// trailing text in `ink2` (see the "✓"/"↑"/"↓"/"◌" spans in the comp); the warn/danger
// rows ("↻ retrying", "⑂ conflict", "■ over quota", "⚿ key epoch", "▢ read-only") color
// the whole line instead — that split is preserved here rather than flattening to one rule.
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { syncGlyph } from "../../lib/anywhere/format";
import type { SyncStatus } from "../../lib/anywhere/types";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { tabularNums, type as typeScale } from "../../theme/typography";

export interface SyncGlyphProps {
  status: SyncStatus;
  showText?: boolean;
}

export function SyncGlyph({ status, showText = true }: SyncGlyphProps) {
  const tokens = useTokens();
  const { glyph, colorKey, text } = syncGlyph(status);
  const color = tokens[colorKey];
  const fullLineColor = colorKey === "warn" || colorKey === "danger";

  return (
    <View style={styles.row} accessibilityRole="text" accessibilityLabel={text}>
      <Text style={[typeScale.monoMeta, tabularNums, { color }]}>{glyph}</Text>
      {showText ? (
        <Text
          style={[typeScale.monoMeta, tabularNums, styles.text, { color: fullLineColor ? color : tokens.ink2 }]}
          numberOfLines={1}
          ellipsizeMode="tail"
        >
          {text}
        </Text>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  // Shrinkable: a row like History's "18h · N msgs · $X · <glyph>" can run out of width for the
  // longest status text ("offline — device-encrypted cache") — without this it silently ran past
  // the card's `overflow: hidden` edge instead of ellipsizing.
  row: { flexDirection: "row", alignItems: "center", gap: space.space4, flexShrink: 1 },
  text: { flexShrink: 1 },
});
