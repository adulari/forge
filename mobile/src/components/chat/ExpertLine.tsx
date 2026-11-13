// "expert: codex-oauth::gpt-5.6-sol · 2 consults · 2.1% of weekly window": the faint line above
// the composer docks once the session has consulted its expert on call ([harness.expert]). The
// host composes the text; this only places it. Information, not a control.
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { tabularNums, type } from "../../theme/typography";

function ExpertLineImpl({ text }: { text: string }) {
  const tokens = useTokens();
  return (
    <View style={styles.row} accessibilityRole="text" accessibilityLabel={text} testID="expert-line">
      <Text style={[type.meta, tabularNums, { color: tokens.ink3 }]} numberOfLines={2}>
        {text}
      </Text>
    </View>
  );
}

export const ExpertLine = React.memo(ExpertLineImpl);

const styles = StyleSheet.create({
  row: { paddingHorizontal: space.space16, paddingVertical: space.space4 },
});
