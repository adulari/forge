import { Plus, X } from "lucide-react-native";
import React from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { getPreviewPreferences, previewLabel } from "../../lib/browserPreview";
import { recentLabel } from "../../lib/previewRecents";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { type WorkbenchSurface } from "../workbench/model";

export function previewTabLabel(tab: WorkbenchSurface): string {
  const url = getPreviewPreferences(previewLabel(tab.id)).url;
  return url ? recentLabel(url) : tab.title;
}

/** The session's browser previews side by side, named by the page they show. */
export function PreviewTabStrip({
  tabs,
  activeId,
  onActivate,
  onClose,
  onAdd,
}: {
  tabs: readonly WorkbenchSurface[];
  activeId: string;
  onActivate: (id: string) => void;
  onClose: (id: string) => void;
  onAdd: () => void;
}) {
  const tokens = useTokens();
  return (
    <View style={[styles.root, { borderBottomColor: tokens.border }]} accessibilityRole="tablist">
      <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={styles.content}>
        {tabs.map((tab) => {
          const active = tab.id === activeId;
          const label = previewTabLabel(tab);
          return (
            <View
              key={tab.id}
              style={[
                styles.tab,
                {
                  borderColor: active ? tokens.accent : tokens.border,
                  backgroundColor: active ? tokens.selection : tokens.bg0,
                },
              ]}
            >
              <Pressable
                onPress={() => onActivate(tab.id)}
                accessibilityRole="tab"
                accessibilityState={{ selected: active }}
                accessibilityLabel={`Preview tab ${label}`}
                style={styles.label}
              >
                <Text
                  style={[typeScale.monoMeta, { color: active ? tokens.accent : tokens.ink3 }]}
                  numberOfLines={1}
                >
                  {label}
                </Text>
              </Pressable>
              <Pressable
                onPress={() => onClose(tab.id)}
                accessibilityRole="button"
                accessibilityLabel={`Close preview tab ${label}`}
                style={styles.close}
              >
                <X size={11} color={tokens.ink4} />
              </Pressable>
            </View>
          );
        })}
        <Pressable
          onPress={onAdd}
          accessibilityRole="button"
          accessibilityLabel="Add browser preview tab"
          style={styles.add}
        >
          <Plus size={14} color={tokens.ink3} />
        </Pressable>
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  root: { height: 34, flexShrink: 0, borderBottomWidth: StyleSheet.hairlineWidth },
  content: { alignItems: "center", gap: space.space4, paddingHorizontal: space.space8 },
  tab: {
    height: 24,
    maxWidth: 200,
    flexDirection: "row",
    alignItems: "center",
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius4,
  },
  label: { flexShrink: 1, paddingLeft: space.space8, paddingRight: space.space4, justifyContent: "center", height: 24 },
  close: { width: 20, height: 24, alignItems: "center", justifyContent: "center" },
  add: { width: 24, height: 24, alignItems: "center", justifyContent: "center" },
});
