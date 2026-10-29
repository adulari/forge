// TeX on iOS and Android: shown as its source in the code face. Typeset math is web and desktop
// only (MathView.web.tsx); a native TeX renderer would be a heavy dependency for a rare case.
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { monoFamily, type } from "../../theme/typography";

export function MathView({ tex, display }: { tex: string; display: boolean }) {
  const tokens = useTokens();
  const text = (
    <Text selectable style={[type.codeSmall, { color: tokens.ink, fontFamily: monoFamily.regular }, display ? null : { backgroundColor: tokens.bg3 }]}>
      {tex}
    </Text>
  );
  if (!display) return text;
  return <View style={[styles.display, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}>{text}</View>;
}

const styles = StyleSheet.create({
  display: { borderWidth: StyleSheet.hairlineWidth, borderRadius: 8, padding: 10, marginVertical: 4 },
});
