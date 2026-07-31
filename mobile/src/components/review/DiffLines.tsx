import { MessageSquare } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { toUnifiedRows, type DiffCell } from "../git/diffModel";
import {
  type ReviewCommentSide,
} from "../../lib/reviewComments";
import { useTokens } from "../../theme/ThemeProvider";
import { hexToRgba, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";

export interface DiffLinesProps {
  lines: readonly string[];
  header?: string;
  isSelected?: (side: ReviewCommentSide, lineNo: number) => boolean;
  commentCount?: (side: ReviewCommentSide, lineNo: number) => number;
  onSelect?: (side: ReviewCommentSide, cell: DiffCell) => void;
}

export function DiffLines({
  lines,
  header = "@@ -1 +1 @@",
  isSelected,
  commentCount,
  onSelect,
}: DiffLinesProps) {
  const tokens = useTokens();
  const rows = toUnifiedRows([{ header, lines: [...lines] }]).filter(
    (row) => row.kind === "line",
  );

  return (
    <>
      {rows.map((row) => {
        const cell = row.cell;
        const side: ReviewCommentSide = cell.kind === "del" ? "old" : "new";
        const selected = isSelected?.(side, cell.lineNo) ?? false;
        const comments = commentCount?.(side, cell.lineNo) ?? 0;
        const gutter = cell.kind === "add" ? "+" : cell.kind === "del" ? "−" : " ";
        const backgroundColor = selected
          ? tokens.selection
          : cell.kind === "add"
            ? tokens.successBg
            : cell.kind === "del"
              ? tokens.dangerBg
              : "transparent";
        const color =
          cell.kind === "add"
            ? tokens.success
            : cell.kind === "del"
              ? tokens.danger
              : tokens.ink2;
        const content = (
          <>
            <Text selectable style={[typeScale.codeSmall, { color }]}>
              {gutter}
              {cell.segments?.length
                ? cell.segments.map((segment, index) => (
                    <Text
                      key={`${index}:${segment.text}`}
                      style={
                        segment.changed
                          ? {
                              backgroundColor:
                                cell.kind === "add"
                                  ? hexToRgba(tokens.success, 0.24)
                                  : hexToRgba(tokens.danger, 0.22),
                            }
                          : undefined
                      }
                    >
                      {segment.text}
                    </Text>
                  ))
                : cell.text || " "}
            </Text>
            {comments > 0 ? (
              <View pointerEvents="none" style={styles.commentMarker}>
                <MessageSquare size={11} strokeWidth={1.8} color={tokens.accent} />
                {comments > 1 ? (
                  <Text style={[typeScale.monoMeta, { color: tokens.accent }]}>{comments}</Text>
                ) : null}
              </View>
            ) : null}
          </>
        );
        const rowStyle = [
          styles.row,
          {
            backgroundColor,
            borderLeftColor: selected ? tokens.accent : "transparent",
          },
        ];
        return onSelect ? (
          <Pressable
            key={row.key}
            onPress={() => onSelect(side, cell)}
            accessibilityRole="button"
            accessibilityLabel={`Select ${side} line ${cell.lineNo} for review`}
            accessibilityState={{ selected }}
            style={rowStyle}
          >
            {content}
          </Pressable>
        ) : (
          <View key={row.key} style={rowStyle}>
            {content}
          </View>
        );
      })}
    </>
  );
}

const styles = StyleSheet.create({
  row: {
    position: "relative",
    paddingHorizontal: space.space12,
    paddingRight: space.space32,
    minWidth: "100%",
    borderLeftWidth: 2,
  },
  commentMarker: {
    position: "absolute",
    right: space.space8,
    top: 2,
    height: 14,
    flexDirection: "row",
    alignItems: "center",
    gap: 1,
  },
});
