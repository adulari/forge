import { Check } from "lucide-react-native";
import React from "react";
import { Platform, Pressable, StyleSheet, Text, type TextStyle, View } from "react-native";

import { PALETTE_CATEGORIES, type PaletteCategory } from "../../lib/palette/sections";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";
import { optionDomId, type PaletteItem } from "./paletteTypes";

/** Mono keycap hint (`esc`, `⌘N`) — web/desktop only; keyboard hints mean nothing on a touch sheet. */
export function KeyHint({ label }: { label: string }) {
  const tokens = useTokens();
  return (
    <View style={[styles.keyHint, { borderColor: tokens.border }]}>
      <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>{label}</Text>
    </View>
  );
}

function HighlightText({ text, query, style }: { text: string; query: string; style: TextStyle | TextStyle[] }) {
  const tokens = useTokens();
  const idx = query ? text.toLowerCase().indexOf(query.toLowerCase()) : -1;
  if (idx < 0) {
    return (
      <Text style={style} numberOfLines={1}>
        {text}
      </Text>
    );
  }
  return (
    <Text style={style} numberOfLines={1}>
      {text.slice(0, idx)}
      <Text style={{ color: tokens.accent }}>{text.slice(idx, idx + query.length)}</Text>
      {text.slice(idx + query.length)}
    </Text>
  );
}

function webOptionProps(domId: string, selected: boolean, disabled: boolean | undefined) {
  if (Platform.OS !== "web") return {};
  return { nativeID: domId, role: "option", "aria-selected": selected, "aria-disabled": disabled || undefined } as object;
}

export function PaletteOption({
  item,
  query,
  selected,
  onActivate,
  onHover,
}: {
  item: PaletteItem;
  query: string;
  selected: boolean;
  onActivate: () => void;
  onHover: () => void;
}) {
  const tokens = useTokens();
  const showTick = Platform.OS === "web" && selected && !item.disabled;
  const hasTrailing = item.trailing != null || showTick;
  const highlightQuery = item.highlight ? query : "";
  return (
    <Pressable
      onPress={item.disabled ? undefined : onActivate}
      disabled={item.disabled}
      accessibilityRole={Platform.OS === "web" ? undefined : "button"}
      accessibilityLabel={item.subtitle ? `${item.title}, ${item.subtitle}` : item.title}
      accessibilityState={{ selected, disabled: item.disabled }}
      {...webOptionProps(optionDomId(item.id), selected, item.disabled)}
      {...(Platform.OS === "web" ? ({ onHoverIn: onHover } as object) : {})}
      style={[
        styles.row,
        { backgroundColor: selected ? tokens.selection : "transparent" },
        item.disabled ? styles.disabled : null,
      ]}
    >
      {item.leading ? <View style={styles.leading}>{item.leading}</View> : null}
      <View style={styles.body}>
        <HighlightText text={item.title} query={highlightQuery} style={[typeScale.body, { color: tokens.ink }]} />
        {item.subtitle ? (
          <HighlightText
            text={item.subtitle}
            query={highlightQuery}
            style={[typeScale.monoMeta, { color: tokens.ink3, fontFamily: monoFamily.regular }]}
          />
        ) : null}
      </View>
      {hasTrailing ? (
        <View style={styles.trailing}>
          {item.trailing}
          {showTick ? <Check size={16} strokeWidth={2} color={tokens.accent} /> : null}
        </View>
      ) : null}
    </Pressable>
  );
}

export function MoreOption({
  id,
  hidden,
  label,
  selected,
  onPress,
  onHover,
}: {
  id: string;
  hidden: number;
  label: string;
  selected: boolean;
  onPress: () => void;
  onHover: () => void;
}) {
  const tokens = useTokens();
  const text = `${hidden} more ${label.toLowerCase()}`;
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole={Platform.OS === "web" ? undefined : "button"}
      accessibilityLabel={`Show ${text}`}
      {...webOptionProps(optionDomId(id), selected, false)}
      {...(Platform.OS === "web" ? ({ onHoverIn: onHover } as object) : {})}
      style={[styles.row, styles.moreRow, { backgroundColor: selected ? tokens.selection : "transparent" }]}
    >
      <Text style={[typeScale.monoMeta, { color: tokens.accent }]}>{text}</Text>
    </Pressable>
  );
}

export function CategoryChips({
  active,
  counts,
  onSelect,
}: {
  active: PaletteCategory;
  counts: Partial<Record<PaletteCategory, number>>;
  onSelect: (category: PaletteCategory) => void;
}) {
  const tokens = useTokens();
  return (
    <View
      style={styles.chips}
      {...(Platform.OS === "web" ? ({ role: "tablist", "aria-label": "Result categories" } as object) : {})}
    >
      {PALETTE_CATEGORIES.map(({ key, label }) => {
        const on = key === active;
        const count = counts[key];
        return (
          <Pressable
            key={key}
            onPress={() => onSelect(key)}
            accessibilityRole={Platform.OS === "web" ? undefined : "button"}
            accessibilityLabel={label}
            accessibilityState={{ selected: on }}
            {...(Platform.OS === "web" ? ({ role: "tab", "aria-selected": on, tabIndex: -1 } as object) : {})}
            style={[
              styles.chip,
              {
                backgroundColor: on ? tokens.selection : "transparent",
                borderColor: on ? tokens.accent : tokens.border,
              },
            ]}
          >
            <Text style={[typeScale.meta, { color: on ? tokens.ink : tokens.ink3 }]}>{label}</Text>
            {count ? <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>{count}</Text> : null}
          </Pressable>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  keyHint: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4, paddingHorizontal: space.space4, paddingVertical: 1 },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    minHeight: 44,
    paddingHorizontal: space.space16,
    paddingVertical: space.space4,
  },
  moreRow: { minHeight: 36 },
  disabled: { opacity: 0.45 },
  leading: { alignItems: "center", justifyContent: "center" },
  body: { flex: 1, gap: 2 },
  trailing: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  chips: { flexDirection: "row", flexWrap: "wrap", gap: space.space8, paddingHorizontal: space.space16, paddingBottom: space.space8 },
  chip: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space4,
    minHeight: 28,
    paddingHorizontal: space.space12,
    borderRadius: radii.radius16,
    borderWidth: StyleSheet.hairlineWidth,
  },
});
