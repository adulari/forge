// Several edits to one file, folded into one row inside an opened tool group:
// "Edit  src/a.ts  ×3  +12 −4". Tapping it lists the individual edits underneath, each a normal
// tool row with its own arguments. See lib/editMerge.ts for which edits merge.
import { ChevronDown, ChevronRight } from "lucide-react-native";
import React, { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import type { DisplayCall } from "../../lib/editMerge";
import { haptics } from "../../lib/haptics";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";
import { headEllipsis } from "../review/pathTruncate";
import { ChangeStat, ToolCallRow } from "./ToolCallRow";

type Merged = Extract<DisplayCall, { kind: "edits" }>;

function MergedEditRowImpl({ merged }: { merged: Merged }) {
  const tokens = useTokens();
  const [open, setOpen] = useState(false);
  const label = `Edit ${merged.path}, ${merged.calls.length} edits${open ? ", hide them" : ", show them"}`;
  return (
    <View>
      <Pressable
        onPress={() => {
          haptics.select();
          setOpen((v) => !v);
        }}
        accessibilityRole="button"
        accessibilityLabel={label}
        accessibilityState={{ expanded: open }}
        style={({ pressed }) => [styles.summary, pressed ? { backgroundColor: tokens.bg2 } : null]}
        hitSlop={4}
      >
        {open ? (
          <ChevronDown size={13} strokeWidth={2} color={tokens.ink4} />
        ) : (
          <ChevronRight size={13} strokeWidth={2} color={tokens.ink4} />
        )}
        <Text style={[type.meta, styles.verb, { color: tokens.ink3 }]} numberOfLines={1}>
          Edit
        </Text>
        <Text style={[type.codeSmall, styles.target, { color: tokens.ink2, fontFamily: monoFamily.regular }]} numberOfLines={1}>
          {headEllipsis(merged.path, 40)}
        </Text>
        <Text style={[type.monoMeta, { color: tokens.ink4 }]}>{`×${merged.calls.length}`}</Text>
        <ChangeStat adds={merged.adds} dels={merged.dels} />
      </Pressable>
      {open ? (
        <View style={styles.nested}>
          {merged.calls.map((call) => (
            <ToolCallRow key={call.key} invocation={call.invocation} />
          ))}
        </View>
      ) : null}
    </View>
  );
}

export const MergedEditRow = React.memo(MergedEditRowImpl);

const styles = StyleSheet.create({
  summary: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    marginLeft: space.space16,
    marginRight: space.space16,
    paddingHorizontal: space.space8,
    paddingVertical: space.space4,
    minHeight: 30,
  },
  verb: { flexShrink: 0, minWidth: 40 },
  target: { flex: 1, minWidth: 0 },
  nested: { paddingLeft: space.space16 },
});
