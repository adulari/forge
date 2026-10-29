// An answered `ask_user`, kept in the transcript as a quiet record instead of a tool row: a
// header chip and the question, the chosen label in bold with its description under it. It
// reads the call and result the daemon already stores (lib/decisionRecord.ts), so it is still
// there after a reload. The live question stays in CardSlot until it is answered.
import { Check, Minus } from "lucide-react-native";
import React, { useMemo } from "react";
import { StyleSheet, Text, View } from "react-native";

import { type DecisionItem, parseDecision } from "../../lib/decisionRecord";
import type { ToolInvocation } from "../../lib/toolRows";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type } from "../../theme/typography";

export function decisionLabel(item: DecisionItem): string {
  const asked = item.question || item.header || "Question";
  return item.answered ? `${asked}: ${item.chosen}${item.note ? `, note: ${item.note}` : ""}` : `${asked}: no answer`;
}

function Item({ item }: { item: DecisionItem }) {
  const tokens = useTokens();
  return (
    <View style={styles.item} accessible accessibilityLabel={decisionLabel(item)}>
      <View style={styles.asked}>
        {item.header ? (
          <View style={[styles.chip, { backgroundColor: tokens.bg3 }]}>
            <Text style={[type.meta, { color: tokens.ink3 }]} numberOfLines={1}>
              {item.header}
            </Text>
          </View>
        ) : null}
        {item.question ? (
          <Text style={[type.sub, styles.question, { color: tokens.ink3 }]} numberOfLines={2}>
            {item.question}
          </Text>
        ) : null}
      </View>
      <View style={styles.chosenRow}>
        {item.answered ? (
          <Check size={13} strokeWidth={2} color={tokens.success} />
        ) : (
          <Minus size={13} strokeWidth={2} color={tokens.ink4} />
        )}
        {item.answered ? (
          <Text style={[type.bodyBold, styles.chosen, { color: tokens.ink }]} numberOfLines={3}>
            {item.chosen}
          </Text>
        ) : (
          <Text style={[type.sub, { color: tokens.ink4, fontStyle: "italic" }]}>No answer</Text>
        )}
      </View>
      {item.description ? (
        <Text style={[type.sub, styles.detail, { color: tokens.ink3 }]} numberOfLines={2}>
          {item.description}
        </Text>
      ) : null}
      {item.note ? (
        <Text style={[type.sub, styles.detail, { color: tokens.ink3 }]} numberOfLines={3}>
          {`Note: ${item.note}`}
        </Text>
      ) : null}
    </View>
  );
}

function DecisionRecordImpl({ invocation }: { invocation: ToolInvocation }) {
  const tokens = useTokens();
  const items = useMemo(() => parseDecision(invocation), [invocation]);
  return (
    <View style={styles.row}>
      <View style={[styles.rule, { backgroundColor: tokens.border }]} />
      <View style={styles.items}>
        {items.map((item, i) => (
          <Item key={i} item={item} />
        ))}
      </View>
    </View>
  );
}

export const DecisionRecord = React.memo(DecisionRecordImpl);

const styles = StyleSheet.create({
  row: { flexDirection: "row", paddingHorizontal: space.space16, paddingVertical: space.space4 },
  rule: { width: 2, borderRadius: 1, marginRight: space.space12 },
  items: { flex: 1, minWidth: 0, gap: space.space8 },
  item: { gap: 2 },
  asked: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  chip: { flexShrink: 0, maxWidth: 140, paddingHorizontal: space.space8, height: 20, justifyContent: "center", borderRadius: radii.radiusPill },
  question: { flex: 1, minWidth: 0 },
  chosenRow: { flexDirection: "row", alignItems: "flex-start", gap: space.space8 },
  chosen: { flex: 1, minWidth: 0 },
  detail: { paddingLeft: 21 },
});
