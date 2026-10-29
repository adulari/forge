import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { fileMark } from "./fileIcons";
import { hexToRgba } from "../../theme/tokens";
import { monoFamily } from "../../theme/typography";

/** A language-coloured tag tile for a path (see fileIcons.ts). Decorative: the path next to it is
 * the accessible name. */
export function FileTypeIcon({ path }: { path: string }) {
  const mark = fileMark(path);
  return (
    <View
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={[styles.tile, { backgroundColor: hexToRgba(mark.hue, 0.16) }]}
    >
      <Text style={[styles.tag, mark.tag.length > 3 && styles.tagLong, { color: mark.hue }]} numberOfLines={1}>
        {mark.tag}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  tile: { width: 24, height: 15, borderRadius: 3, alignItems: "center", justifyContent: "center" },
  tag: { fontFamily: monoFamily.bold, fontSize: 8, lineHeight: 10, letterSpacing: 0.2 },
  tagLong: { fontSize: 6.5, letterSpacing: 0 },
});
