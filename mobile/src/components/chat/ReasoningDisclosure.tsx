// Collapsed-by-default disclosure for model reasoning ("thinking"). Reasoning is NEVER shown in
// the main answer slot; it lives here behind a "Thinking" label + chevron (same chevron/press
// affordance as DiffCard's file sections). Expand state is shared by reasoning content (see
// lib/reasoning.ts) so it survives the streaming→history finalize swap without flashing.
import { ChevronDown, ChevronRight } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { reasoningKey, useReasoningExpanded } from "../../lib/reasoning";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";

export interface ReasoningDisclosureProps {
  reasoning: string;
  /** True while the reasoning block is still streaming (answer not begun) — animates the label. */
  active?: boolean;
}

function ReasoningDisclosureImpl({ reasoning, active = false }: ReasoningDisclosureProps) {
  const tokens = useTokens();
  const [expanded, toggle] = useReasoningExpanded(reasoningKey(reasoning));

  return (
    <View style={styles.container}>
      <Pressable
        onPress={toggle}
        accessibilityRole="button"
        accessibilityLabel={`${expanded ? "collapse" : "expand"} reasoning`}
        accessibilityState={{ expanded }}
        style={styles.header}
        hitSlop={8}
      >
        {expanded ? (
          <ChevronDown size={14} strokeWidth={1.75} color={tokens.ink3} />
        ) : (
          <ChevronRight size={14} strokeWidth={1.75} color={tokens.ink3} />
        )}
        <Text style={[typeScale.meta, { color: tokens.ink3 }]}>{active ? "Thinking…" : "Thinking"}</Text>
      </Pressable>

      {expanded && reasoning ? (
        <View style={[styles.body, { borderLeftColor: tokens.border }]}>
          <Text
            selectable
            style={[typeScale.codeSmall, { color: tokens.ink3, fontFamily: monoFamily.regular }]}
          >
            {reasoning.trim()}
          </Text>
        </View>
      ) : null}
    </View>
  );
}

export const ReasoningDisclosure = React.memo(ReasoningDisclosureImpl);

const styles = StyleSheet.create({
  container: { marginBottom: space.space4 },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space4,
    paddingVertical: space.space4,
    minHeight: 28,
  },
  body: {
    borderLeftWidth: StyleSheet.hairlineWidth,
    paddingLeft: space.space8,
    marginLeft: space.space8,
    marginTop: space.space4,
  },
});
