import { useRouter } from "expo-router";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import type { PrWatch } from "../../../lib/apiPr";
import { useTokens } from "../../../theme/ThemeProvider";
import { hexToRgba, radii, space } from "../../../theme/tokens";
import { type as typeScale } from "../../../theme/typography";
import { watchStatus } from "../../pr/prWatchModel";
import { Badge } from "../../ds/Badge";

export function PrWatchList({ watches, onNavigate }: { watches: PrWatch[]; onNavigate: () => void }) {
  const tokens = useTokens();
  const router = useRouter();
  if (watches.length === 0) return null;
  return (
    <View style={styles.root}>
      <Text style={[typeScale.meta, { color: tokens.ink2 }]}>PR Watch</Text>
      {watches.map((watch) => {
        const status = watchStatus(watch);
        const title = `${watch.pr_number != null ? `Pull request #${watch.pr_number}` : "Waiting for a pull request"} - session ${watch.session_id.slice(0, 8)}`;
        return (
          <Pressable
            key={watch.session_id}
            onPress={() => {
              onNavigate();
              router.push(`/session/${watch.session_id}`);
            }}
            accessibilityRole="button"
            accessibilityLabel={`${title}, ${status.label}${status.detail ? `, ${status.detail}` : ""}. Open the session`}
            style={({ pressed, hovered }: { pressed: boolean; hovered?: boolean }) => [
              styles.row,
              { borderColor: tokens.border },
              (pressed || hovered) && { backgroundColor: hexToRgba(tokens.accent, 0.1) },
            ]}
          >
            <View style={styles.line}>
              <Text style={[typeScale.sub, styles.flex, { color: tokens.ink }]} numberOfLines={1}>
                {title}
              </Text>
              <Badge label={status.label} tone={status.tone} />
            </View>
            {status.detail ? (
              <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]} numberOfLines={2}>
                {status.detail}
              </Text>
            ) : null}
          </Pressable>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  root: { gap: space.space8 },
  row: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius8, padding: space.space12, gap: 2 },
  line: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  flex: { flex: 1, minWidth: 0 },
});
