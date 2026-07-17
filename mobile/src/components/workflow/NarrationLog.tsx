// Collapsible narration feed from `workflow.logs` — mono, bounded height. Defaults collapsed
// so a long run's log doesn't swamp the timeline; the header shows the line count.
import { ChevronDown, ChevronRight } from "lucide-react-native";
import React, { useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";

export function NarrationLog({ logs }: { logs: string[] }) {
  const tokens = useTokens();
  const [open, setOpen] = useState(false);
  if (logs.length === 0) return null;

  return (
    <View style={styles.wrap}>
      <Pressable
        onPress={() => setOpen((v) => !v)}
        accessibilityRole="button"
        accessibilityState={{ expanded: open }}
        accessibilityLabel={`Narration, ${logs.length} lines`}
        style={styles.header}
      >
        {open ? (
          <ChevronDown size={14} strokeWidth={2} color={tokens.ink4} />
        ) : (
          <ChevronRight size={14} strokeWidth={2} color={tokens.ink4} />
        )}
        <Text style={[typeScale.section, styles.label, { color: tokens.ink4 }]}>{`narration · ${logs.length}`}</Text>
      </Pressable>
      {open ? (
        <ScrollView
          style={[styles.feed, { backgroundColor: tokens.bg0, borderColor: tokens.border }]}
          contentContainerStyle={styles.feedContent}
          nestedScrollEnabled
        >
          {logs.map((line, index) => (
            <Text key={index} style={[styles.line, { color: tokens.ink3 }]}>
              {line}
            </Text>
          ))}
        </ScrollView>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { marginTop: space.space20 },
  header: { flexDirection: "row", alignItems: "center", gap: space.space4, minHeight: 32 },
  label: { flex: 1 },
  feed: {
    maxHeight: 160,
    marginTop: space.space8,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius12,
  },
  feedContent: { padding: space.space12, gap: 3 },
  line: { fontFamily: monoFamily.regular, fontSize: 11, lineHeight: 16 },
});
