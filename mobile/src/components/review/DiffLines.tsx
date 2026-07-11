import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";

export function DiffLines({ lines }: { lines: readonly string[] }) {
  const tokens = useTokens();
  return <>
    {lines.map((line, index) => {
      const gutter = line[0] ?? " ";
      const backgroundColor = gutter === "+" ? tokens.successBg : gutter === "-" ? tokens.dangerBg : "transparent";
      const color = gutter === "+" ? tokens.success : gutter === "-" ? tokens.danger : tokens.ink2;
      return <View key={`${index}:${line}`} style={[styles.row, { backgroundColor }]}>
        <Text selectable style={[typeScale.codeSmall, { color }]}>{line || " "}</Text>
      </View>;
    })}
  </>;
}

const styles = StyleSheet.create({ row: { paddingHorizontal: space.space12, minWidth: "100%" } });
