// Machined desktop shell — "What's New" (D What's New, docs/design/machined
// INVENTORY.md L384-390). No changelog/release-notes endpoint exists yet (INVENTORY's
// gaps summary flags this as a real feature gap, not just desktop chrome) — this is
// an honest empty state, not a fabricated release list. Reached from the command
// palette's Actions group.
import { Sparkles } from "lucide-react-native";
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { EmptyState } from "../ds/EmptyState";
import { Sheet } from "../ds/Sheet";

export interface WhatsNewPanelProps {
  visible: boolean;
  onClose: () => void;
}

export function WhatsNewPanel({ visible, onClose }: WhatsNewPanelProps) {
  const tokens = useTokens();
  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="What's New">
      <View style={styles.content}>
        <Text style={[typeScale.headingBold, { color: tokens.ink }]}>What&apos;s New</Text>
        <EmptyState
          icon={Sparkles}
          message="No release notes yet — this panel will list what changed in each Forge update once that feed exists."
        />
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space16, paddingBottom: space.space32, gap: space.space8 },
});
