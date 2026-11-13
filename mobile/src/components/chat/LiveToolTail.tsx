// The rolling output of the tool call that is running right now (a long `cargo test`, a build),
// dim and monospace under the turn's activity line, so a slow command reads as progressing, hung
// or failing early instead of as a frozen spinner. The host already strips ANSI and caps the tail;
// this only trims it to what fits a phone.
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import type { SnapshotLiveTail } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, tabularNums, type } from "../../theme/typography";
import { formatElapsed } from "./LiveTurn";

const MAX_ROWS = 10;
const MAX_ROW_CHARS = 200;

/** The rows to draw: the last `MAX_ROWS` non-blank lines, each cut to a phone-sized width. */
export function liveTailRows(lines: readonly string[]): string[] {
  return lines
    .filter((line) => line.trim() !== "")
    .slice(-MAX_ROWS)
    .map((line) => (line.length > MAX_ROW_CHARS ? `${line.slice(0, MAX_ROW_CHARS - 1)}…` : line));
}

function LiveToolTailImpl({ tail }: { tail: SnapshotLiveTail }) {
  const tokens = useTokens();
  const rows = liveTailRows(tail.lines);
  if (rows.length === 0) return null;
  return (
    <View
      style={[styles.box, { backgroundColor: tokens.bg0, borderColor: tokens.border }]}
      accessibilityLabel={`Live output, ${formatElapsed(tail.elapsed_ms)}: ${rows[rows.length - 1]}`}
    >
      <Text style={[type.meta, tabularNums, { color: tokens.ink4 }]}>{`output · ${formatElapsed(tail.elapsed_ms)}`}</Text>
      {rows.map((row, i) => (
        <Text
          key={`${i}:${row}`}
          style={[type.codeSmall, { color: tokens.ink3, fontFamily: monoFamily.regular }]}
          numberOfLines={1}
        >
          {row}
        </Text>
      ))}
    </View>
  );
}

export const LiveToolTail = React.memo(LiveToolTailImpl);

const styles = StyleSheet.create({
  box: {
    marginTop: space.space4,
    paddingHorizontal: space.space8,
    paddingVertical: space.space4,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 6,
    gap: 1,
  },
});
