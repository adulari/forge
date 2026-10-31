import { History } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { recentLabel } from "../../lib/previewRecents";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";

/** Addresses opened before, newest first. Rows live in the layout flow: the native preview
 * webview paints over anything that floats above the page body. */
export function RecentUrls({
  urls,
  onPick,
}: {
  urls: readonly string[];
  onPick: (url: string) => void;
}) {
  const tokens = useTokens();
  if (urls.length === 0) return null;
  return (
    <View style={[styles.root, { borderBottomColor: tokens.border, backgroundColor: tokens.bg1 }]} accessibilityLabel="Recent preview addresses">
      {urls.map((url) => (
        <Pressable
          key={url}
          onPress={() => onPick(url)}
          accessibilityRole="button"
          accessibilityLabel={`Open ${url}`}
          style={({ pressed }) => [styles.row, pressed && { backgroundColor: tokens.bg3 }]}
        >
          <History size={13} color={tokens.ink4} />
          <Text style={[typeScale.monoMeta, { color: tokens.ink2 }]} numberOfLines={1}>
            {recentLabel(url)}
          </Text>
        </Pressable>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  root: { flexShrink: 0, borderBottomWidth: StyleSheet.hairlineWidth, paddingVertical: space.space4 },
  row: { minHeight: 28, flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space12 },
});
