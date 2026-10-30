import { ChevronDown, ChevronRight } from "lucide-react-native";
import React, { useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import type { GitDiffFile } from "../../lib/api";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { languageForPath } from "../git/diffHighlight";
import { UnifiedBody } from "../git/diffLines";
import { toUnifiedRows } from "../git/diffModel";
import { FileTypeIcon } from "../git/FileTypeIcon";

const NO_REVIEW = {
  isSelected: () => false,
  commentCount: () => 0,
  onSelect: () => undefined,
};

/** One file of a pull request's diff, rendered with the git dock's own line renderer. */
export function PrFileCard({ file, defaultOpen }: { file: GitDiffFile; defaultOpen: boolean }) {
  const tokens = useTokens();
  const [open, setOpen] = useState(defaultOpen);
  const rows = useMemo(() => (open ? toUnifiedRows(file.hunks) : []), [open, file.hunks]);
  const language = useMemo(() => languageForPath(file.path), [file.path]);
  return (
    <View style={[styles.card, { borderColor: tokens.border, backgroundColor: tokens.bg1 }]}>
      <Pressable
        onPress={() => setOpen((v) => !v)}
        accessibilityRole="button"
        accessibilityState={{ expanded: open }}
        accessibilityLabel={`${file.path}, ${file.adds} added, ${file.dels} removed`}
        style={[styles.head, { backgroundColor: tokens.bg2 }]}
      >
        {open ? (
          <ChevronDown size={14} color={tokens.ink3} />
        ) : (
          <ChevronRight size={14} color={tokens.ink3} />
        )}
        <FileTypeIcon path={file.path} />
        <Text style={[styles.path, { color: tokens.ink }]} numberOfLines={1} ellipsizeMode="middle">
          {file.path}
        </Text>
        <Text style={[styles.count, tabularNums, { color: tokens.success }]}>+{file.adds}</Text>
        <Text style={[styles.count, tabularNums, { color: tokens.danger }]}>-{file.dels}</Text>
      </Pressable>
      {open ? (
        file.binary ? (
          <Text style={[typeScale.monoMeta, styles.note, { color: tokens.ink3 }]}>Binary file not shown</Text>
        ) : (
          <>
            <UnifiedBody rows={rows} language={language} {...NO_REVIEW} />
            {file.skipped_lines > 0 ? (
              <Text style={[typeScale.monoMeta, styles.note, { color: tokens.ink4 }]}>
                {file.skipped_lines.toLocaleString()} more lines not shown
              </Text>
            ) : null}
          </>
        )
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  card: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4, overflow: "hidden" },
  head: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space8, minHeight: 34 },
  path: { flex: 1, fontFamily: monoFamily.regular, fontSize: 12 },
  count: { fontFamily: monoFamily.regular, fontSize: 11.5 },
  note: { padding: space.space8 },
});
