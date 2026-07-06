// Overlay mirror (modal route) — BUILD_PLAN §6 "Overlay mirror" / §7 Batch 4 W9. Mirrors
// whatever modal surface owns the TUI keyboard server-side (palette / any picker / config /
// usage / mesh / workflow), matching crates/forge-cli/src/remote_assets/app.js `renderOverlay`
// (title, tappable rows with group headers, a live filter box, a free-text commit row, or a
// pre-rendered mono body) so every slash command surface is drivable from the phone. The
// session shell (`_layout.tsx`) auto-presents this screen when `snapshot.overlay` becomes
// non-null and auto-dismisses it when the server clears it; the native header (also owned by
// `_layout.tsx`) provides the close button. This file additionally guards Android hardware
// back and any other unmount path so the server always hears `overlay_cancel` exactly once
// per presentation, never zero times and never as a race with the next snapshot.
import { useRouter } from "expo-router";
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { BackHandler, Platform, Pressable, ScrollView, Text, TextInput, View } from "react-native";
import Animated from "react-native-reanimated";

import {
  BoundedList,
  EmptyState,
  Loading,
  PrimaryButton,
  Screen,
  SearchInput,
} from "../../../components/ui";
import { usePressScale } from "../../../lib/motion";
import { theme } from "../../../lib/theme";
import { useSessionCtx } from "../../../lib/sessionContext";
import type { Overlay, OverlayRow } from "../../../lib/ws";

const FILTER_DEBOUNCE_MS = 150;

const monoStyle = {
  fontFamily: Platform.select({ ios: "Menlo", android: "monospace", default: "ui-monospace" }),
  fontSize: 12,
  lineHeight: 18,
} as const;

function lightHaptic() {
  if (Platform.OS === "web") return;
  import("expo-haptics")
    .then((H) => H.impactAsync(H.ImpactFeedbackStyle.Light))
    .catch(() => {});
}

// ---------------------------------------------------------------------------
// Row + group header — flattened into one list so the whole body is a single
// virtualized BoundedList (UI_RULES.md #7, #26-27).
// ---------------------------------------------------------------------------

type FlatItem =
  | { type: "group"; key: string; label: string }
  | { type: "row"; key: string; row: OverlayRow };

function flattenRows(rows: OverlayRow[]): FlatItem[] {
  const out: FlatItem[] = [];
  let lastGroup: string | null = null;
  for (const row of rows) {
    if (row.group && row.group !== lastGroup) {
      out.push({ type: "group", key: `group:${row.group}`, label: row.group });
      lastGroup = row.group;
    }
    out.push({ type: "row", key: `row:${row.id}`, row });
  }
  return out;
}

function rowsEqual(a: OverlayRow, b: OverlayRow): boolean {
  return (
    a.id === b.id &&
    a.label === b.label &&
    a.detail === b.detail &&
    a.selected === b.selected &&
    a.group === b.group
  );
}

function OverlayGroupHeaderBase({ label }: { label: string }) {
  return (
    <Text className="text-dim text-[11px] font-bold uppercase tracking-[0.5px] px-2 pt-8 pb-4">
      {label}
    </Text>
  );
}
const OverlayGroupHeader = React.memo(OverlayGroupHeaderBase);

function OverlayRowItemBase({
  row,
  onSelect,
}: {
  row: OverlayRow;
  onSelect: (id: string) => void;
}) {
  const { style, onPressIn, onPressOut } = usePressScale();
  const handlePress = useCallback(() => {
    lightHaptic();
    onSelect(row.id);
  }, [onSelect, row.id]);

  return (
    <Animated.View style={style}>
      <Pressable
        onPress={handlePress}
        onPressIn={onPressIn}
        onPressOut={onPressOut}
        className={`rounded-md border px-10 py-8 gap-2 mb-6 ${
          row.selected ? "bg-selBg border-accent" : "bg-chipBg border-transparent"
        }`}
        style={{ minHeight: 44 }}
      >
        <Text
          numberOfLines={1}
          className={`text-[14px] font-semibold ${row.selected ? "text-accent" : "text-ink"}`}
        >
          {row.label}
        </Text>
        {row.detail ? (
          <Text numberOfLines={1} className="text-dim text-[12px]">
            {row.detail}
          </Text>
        ) : null}
      </Pressable>
    </Animated.View>
  );
}
const OverlayRowItem = React.memo(
  OverlayRowItemBase,
  (prev, next) => prev.onSelect === next.onSelect && rowsEqual(prev.row, next.row),
);

// ---------------------------------------------------------------------------
// Screen
// ---------------------------------------------------------------------------

export default function OverlayScreen() {
  const router = useRouter();
  const { snapshot, send } = useSessionCtx();
  const overlay: Overlay | null = snapshot?.overlay ?? null;

  // Cancel/dismiss discipline: tell the server exactly once per presentation, whether the
  // user closes explicitly (Android hardware back here; the native header close button lives
  // in _layout.tsx and guards itself) or this screen unmounts any other way while the server
  // still believes the overlay is open (defensive fallback below).
  const cancelSentRef = useRef(false);
  const overlayActiveRef = useRef(!!overlay);
  useEffect(() => {
    overlayActiveRef.current = !!overlay;
    if (overlay) cancelSentRef.current = false;
  }, [overlay]);

  const handleBack = useCallback(() => {
    if (!cancelSentRef.current) {
      cancelSentRef.current = true;
      send({ kind: "overlay_cancel" });
    }
    router.back();
  }, [send, router]);

  useEffect(() => {
    const sub = BackHandler.addEventListener("hardwareBackPress", () => {
      handleBack();
      return true;
    });
    return () => sub.remove();
  }, [handleBack]);

  // Defensive: if this screen unmounts for any reason other than handleBack (e.g. a
  // programmatic pop) while the server still believes the overlay is active, tell it anyway.
  useEffect(() => {
    return () => {
      if (overlayActiveRef.current && !cancelSentRef.current) {
        cancelSentRef.current = true;
        send({ kind: "overlay_cancel" });
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [send]);

  // Filter box: live-updates as you type (debounced 150ms), never clobbered by a server echo
  // while focused (mirrors app.js: `document.activeElement !== f`).
  const [filterText, setFilterText] = useState(overlay?.filter ?? "");
  const filterFocusedRef = useRef(false);
  const filterDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (!filterFocusedRef.current) setFilterText(overlay?.filter ?? "");
  }, [overlay?.filter]);

  useEffect(() => {
    return () => {
      if (filterDebounceRef.current) clearTimeout(filterDebounceRef.current);
    };
  }, []);

  const onChangeFilter = useCallback(
    (text: string) => {
      setFilterText(text);
      if (filterDebounceRef.current) clearTimeout(filterDebounceRef.current);
      filterDebounceRef.current = setTimeout(() => {
        send({ kind: "overlay_filter", text });
      }, FILTER_DEBOUNCE_MS);
    },
    [send],
  );

  // Free-text row: batches the whole value, commits with overlay_filter + a synthesized Enter
  // (mirrors app.js `submitFree`) instead of live-updating on every keystroke.
  const [freeText, setFreeText] = useState("");
  const submitFreeText = useCallback(() => {
    lightHaptic();
    send({ kind: "overlay_filter", text: freeText });
    send({ kind: "key", key: "Enter" });
    setFreeText("");
  }, [freeText, send]);

  const onSelectRow = useCallback((id: string) => send({ kind: "overlay_select", id }), [send]);
  const onNav = useCallback((delta: number) => send({ kind: "overlay_nav", delta }), [send]);

  const flatItems = useMemo(() => (overlay ? flattenRows(overlay.rows) : []), [overlay]);

  const renderItem = useCallback(
    ({ item }: { item: FlatItem }) =>
      item.type === "group" ? (
        <OverlayGroupHeader label={item.label} />
      ) : (
        <OverlayRowItem row={item.row} onSelect={onSelectRow} />
      ),
    [onSelectRow],
  );

  const emptyRows = useMemo(() => <EmptyState title="No matches." />, []);

  if (!snapshot) {
    return (
      <Screen scroll={false}>
        <Loading label="Connecting to session…" />
      </Screen>
    );
  }

  if (!overlay) {
    return (
      <Screen scroll={false}>
        <EmptyState title="No overlay active." />
      </Screen>
    );
  }

  const hasFilter = overlay.filter !== null && overlay.filter !== undefined;
  const hasRows = overlay.rows.length > 0;

  return (
    <Screen scroll={false} keyboardAvoiding={overlay.free_text}>
      <View className="flex-1 gap-8 pt-8">
        {hasFilter ? (
          <View className="flex-row items-center gap-8">
            <SearchInput
              value={filterText}
              onChangeText={onChangeFilter}
              onFocus={() => {
                filterFocusedRef.current = true;
              }}
              onBlur={() => {
                filterFocusedRef.current = false;
              }}
              placeholder="filter…"
              autoCapitalize="none"
              autoCorrect={false}
              className="flex-1"
            />
            <Pressable
              onPress={() => onNav(-1)}
              hitSlop={8}
              className="bg-chipBg border border-border rounded-md items-center justify-center"
              style={{ minWidth: 44, minHeight: 44 }}
            >
              <Text className="text-ink text-[16px]">▲</Text>
            </Pressable>
            <Pressable
              onPress={() => onNav(1)}
              hitSlop={8}
              className="bg-chipBg border border-border rounded-md items-center justify-center"
              style={{ minWidth: 44, minHeight: 44 }}
            >
              <Text className="text-ink text-[16px]">▼</Text>
            </Pressable>
          </View>
        ) : null}

        {hasRows ? (
          <View className="flex-1">
            <BoundedList
              data={flatItems}
              keyExtractor={(item) => item.key}
              renderItem={renderItem}
              ListEmptyComponent={emptyRows}
              contentContainerStyle={{ paddingBottom: 12 }}
            />
          </View>
        ) : null}

        {overlay.body != null ? (
          <View className="flex-1 bg-codeBg rounded-md border border-borderSoft overflow-hidden">
            <ScrollView contentContainerStyle={{ padding: 8 }}>
              <ScrollView horizontal showsHorizontalScrollIndicator={false}>
                <Text selectable className="text-ink" style={monoStyle}>
                  {overlay.body}
                </Text>
              </ScrollView>
            </ScrollView>
          </View>
        ) : null}

        {!hasRows && overlay.body == null ? <EmptyState title="Nothing to show." /> : null}

        {overlay.free_text ? (
          <View className="flex-row items-center gap-8">
            <TextInput
              value={freeText}
              onChangeText={setFreeText}
              placeholder="type a value…"
              placeholderTextColor={theme.colors.dim}
              autoCapitalize="none"
              autoCorrect={false}
              returnKeyType="done"
              onSubmitEditing={submitFreeText}
              className="flex-1 bg-panelDeep border border-border rounded-md px-10 text-ink text-[15px]"
              style={{ minHeight: 44 }}
            />
            <PrimaryButton
              label="OK"
              onPress={submitFreeText}
              disabled={!freeText.trim()}
              fullWidth={false}
            />
          </View>
        ) : null}
      </View>
    </Screen>
  );
}
