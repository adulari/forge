import React, { useMemo } from "react";
import { FlatList, ScrollView, StyleSheet, Text, View, type ListRenderItemInfo, type TextStyle } from "react-native";

import { type HighlightToken, type TokenKind } from "../../lib/highlightTokens";
import { useTheme } from "../../theme/ThemeProvider";
import { type ColorTokens, space } from "../../theme/tokens";
import { monoFamily } from "../../theme/typography";
import { gutterWidth, tokensByLine } from "./codeLines";

const LINE_HEIGHT = 18;
const CHAR_WIDTH = 7.4;

function tokenStyle(kind: TokenKind, tokens: ColorTokens, keyword: string): TextStyle | undefined {
  switch (kind) {
    case "keyword":
      return { color: keyword };
    case "string":
      return { color: tokens.success };
    case "comment":
      return { color: tokens.ink3, fontStyle: "italic" };
    case "number":
      return { color: tokens.info };
    default:
      return undefined;
  }
}

/** Read-only, line-numbered, syntax-colored view of a file. Virtualized, so a long file costs
 * only the rows on screen. */
export function CodeView({ code, language }: { code: string; language: string }) {
  const { tokens, scheme } = useTheme();
  const keyword = scheme === "dark" ? tokens.ember.ember300 : tokens.ember.ember600;
  const lines = useMemo(() => tokensByLine(code, language), [code, language]);
  const width = gutterWidth(lines.length) * 8 + space.space12;
  const contentWidth = useMemo(() => {
    let longest = 0;
    for (const line of lines) {
      let length = 0;
      for (const token of line) length += token.text.length;
      if (length > longest) longest = length;
    }
    return width + space.space8 + longest * CHAR_WIDTH + space.space16;
  }, [lines, width]);

  const renderItem = ({ item, index }: ListRenderItemInfo<HighlightToken[]>) => (
    <View style={[styles.line, { width: contentWidth }]}>
      <Text
        selectable={false}
        style={[styles.gutter, { width, color: tokens.ink4, borderRightColor: tokens.border }]}
      >
        {index + 1}
      </Text>
      <Text numberOfLines={1} style={[styles.code, { color: tokens.ink }]}>
        {item.length === 0
          ? " "
          : item.map((token, tokenIndex) => (
              <Text key={tokenIndex} style={tokenStyle(token.kind, tokens, keyword)}>
                {token.text}
              </Text>
            ))}
      </Text>
    </View>
  );

  return (
    <ScrollView
      horizontal
      style={[styles.list, { backgroundColor: tokens.bg0 }]}
      contentContainerStyle={{ minWidth: "100%" }}
      accessibilityLabel="Code view"
    >
      <FlatList
        data={lines}
        renderItem={renderItem}
        keyExtractor={(_, index) => String(index)}
        getItemLayout={(_, index) => ({ length: LINE_HEIGHT, offset: LINE_HEIGHT * index, index })}
        initialNumToRender={60}
        windowSize={9}
        style={{ width: contentWidth }}
      />
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  list: { flex: 1 },
  line: { flexDirection: "row", height: LINE_HEIGHT },
  gutter: {
    fontFamily: monoFamily.regular,
    fontSize: 11,
    lineHeight: LINE_HEIGHT,
    textAlign: "right",
    paddingRight: 6,
    marginRight: space.space8,
    borderRightWidth: StyleSheet.hairlineWidth,
  },
  // `pre` keeps indentation: react-native-web collapses whitespace on a single-line Text.
  code: { flex: 1, fontFamily: monoFamily.regular, fontSize: 12, lineHeight: LINE_HEIGHT, ...({ whiteSpace: "pre" } as object) },
});
