// "Worked for 23s · 4.1k tokens · $0.04": the faint line that closes a finished turn
// (lib/turnFooter.ts). Plain text in the message gutter; it is information, not a control.
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { tabularNums, type } from "../../theme/typography";

function TurnFooterImpl({ text }: { text: string }) {
  const tokens = useTokens();
  return (
    <View style={styles.row} accessibilityRole="text" accessibilityLabel={text}>
      <Text style={[type.meta, tabularNums, { color: tokens.ink4 }]} numberOfLines={2}>
        {text}
      </Text>
    </View>
  );
}

export const TurnFooter = React.memo(TurnFooterImpl);

const styles = StyleSheet.create({
  row: { paddingHorizontal: space.space16, paddingTop: space.space4, paddingBottom: space.space12 },
});
