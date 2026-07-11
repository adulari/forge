// DESIGN_SYSTEM.md §6 OverlayPanel — the TUI overlay mirror: title bar + close,
// SearchField when `filter != null` (debounced 150ms), grouped rows (section
// headers) with server-authoritative selection highlight, mono `body` view,
// free-text commit row. Rendered inside a Sheet (compact) / centered modal
// 560pt (medium+), per §7 useBreakpoint. DESIGN_ELEVATION.md Move 2/3 — rows
// are hairline-separated, not boxed; selection reads as a tint wash.
//
// FEATURES.md §1.2/§1.3 wiring: rows -> `overlay_select{id}`, filter ->
// `overlay_filter{text}` (debounced), free-text commit -> `overlay_filter{text}`
// then `key{key:"Enter"}`, any close path -> `overlay_cancel` (owned by the
// caller's `onClose`, so scrim/Esc/back-gesture/title-X all funnel through it).
//
// Keyboard passthrough (arrows/Enter/Esc/Tab/PageUp/PageDown while open, beyond this
// panel's own Esc-to-close) is implemented below (T5.1).
import { Send, X } from "lucide-react-native";
import React, { useEffect, useMemo, useState } from "react";
import {
  Modal,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
  useWindowDimensions,
} from "react-native";
import Animated, {
  runOnJS,
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withTiming,
} from "react-native-reanimated";

import { IconButton } from "../ds/IconButton";
import { Input } from "../ds/Input";
import { ListRow } from "../ds/ListRow";
import { SearchField } from "../ds/SearchField";
import { SectionHeader } from "../ds/SectionHeader";
import { isNativeOverlayKind, NativeOverlayContent } from "./NativeOverlayContent";
import { Sheet } from "../ds/Sheet";
import { haptics } from "../../lib/haptics";
import type { Overlay, OverlayRow, RemoteInput } from "../../lib/ws";
import { durations, easings } from "../../theme/motion";
import { useTheme, useTokens } from "../../theme/ThemeProvider";
import { depthDark, depthLight, radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";

export interface OverlayPanelProps {
  overlay: Overlay;
  /** Drives the Anvil entrance/exit; the panel keeps rendering the last known
   * `overlay` content while this transitions to false so the close animation
   * has something to animate away from (see OverlayHost). */
  visible: boolean;
  send: (input: RemoteInput) => void;
  onClose: () => void;
}

interface RowGroup {
  group: string | null;
  rows: OverlayRow[];
}

function groupRows(rows: OverlayRow[]): RowGroup[] {
  const groups: RowGroup[] = [];
  for (const row of rows) {
    const last = groups[groups.length - 1];
    if (last && last.group === row.group) {
      last.rows.push(row);
    } else {
      groups.push({ group: row.group, rows: [row] });
    }
  }
  return groups;
}

const PASSTHROUGH_KEYS = new Set(["ArrowUp", "ArrowDown", "Enter", "Escape", "Tab", "PageUp", "PageDown"]);

function isTypingTarget(target: EventTarget | null): boolean {
  if (typeof HTMLElement === "undefined" || !(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || target.isContentEditable;
}

export function OverlayPanel({ overlay, visible, send, onClose }: OverlayPanelProps) {
  const tokens = useTokens();
  const { isCompact } = useBreakpoint();

  // T5.1 — overlay keyboard passthrough (closes the T4.1 deferral noted above): while open
  // on web/desktop, forward navigation keys to the daemon as raw `key` inputs so the
  // server-authoritative selection can be driven from the keyboard, not just row taps. Skips
  // forwarding (except Esc, which should always reach the daemon) while focus is in the
  // filter/free-text Input — those already handle their own Enter/typing locally.
  useEffect(() => {
    if (Platform.OS !== "web" || !visible) return;
    const handler = (e: KeyboardEvent) => {
      if (!PASSTHROUGH_KEYS.has(e.key)) return;
      if (e.key !== "Escape" && isTypingTarget(e.target)) return;
      send({ kind: "key", key: e.key });
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [visible, send]);

  // Local echo of the filter/free-text inputs — intentionally not resynced
  // from later snapshots (same pattern as QuestionCard's free-text row):
  // OverlayHost remounts this component (via `key={overlay.kind}`) whenever a
  // genuinely different overlay opens, which is the only time this should reset.
  const [filterText, setFilterText] = useState(overlay.filter ?? "");
  const [freeText, setFreeText] = useState("");

  const groups = useMemo(() => groupRows(overlay.rows), [overlay.rows]);

  const selectRow = (id: string) => {
    haptics.select();
    send({ kind: "overlay_select", id });
  };

  const commitFreeText = () => {
    const text = freeText.trim();
    if (text.length === 0) return;
    send({ kind: "overlay_filter", text });
    send({ kind: "key", key: "Enter" });
    setFreeText("");
  };

  const showEmpty = overlay.rows.length === 0 && overlay.body == null && !overlay.free_text;

  const content = (
    <View style={styles.container}>
      <View style={styles.titleRow}>
        <Text style={[typeScale.heading, { color: tokens.ink }]} numberOfLines={1}>
          {overlay.title}
        </Text>
        <IconButton
          icon={<X size={20} strokeWidth={1.75} color={tokens.ink2} />}
          onPress={onClose}
          accessibilityLabel="Close"
        />
      </View>

      {overlay.filter != null ? (
        <View style={styles.searchRow}>
          <SearchField
            value={filterText}
            onChangeText={setFilterText}
            onDebouncedChange={(text) => send({ kind: "overlay_filter", text })}
            debounceMs={150}
            accessibilityLabel="Filter"
          />
        </View>
      ) : null}

      {isNativeOverlayKind(overlay.kind) ? (
        <NativeOverlayContent overlay={overlay} onSelect={selectRow} />
      ) : (
        <ScrollView style={styles.rows} keyboardShouldPersistTaps="handled">
          {groups.map((g, gi) => (
            <View key={g.group ?? `_row_group_${gi}`}>
              {g.group ? <SectionHeader>{g.group}</SectionHeader> : null}
              {g.rows.map((row) => (
                <View key={row.id} style={row.selected ? { backgroundColor: tokens.selection } : undefined}>
                  <ListRow
                    title={row.label}
                    subtitle={row.detail || undefined}
                    onPress={() => selectRow(row.id)}
                    accessibilityRole="menuitem"
                    accessibilityLabel={row.detail ? `${row.label} — ${row.detail}` : row.label}
                  />
                </View>
              ))}
            </View>
          ))}
          {showEmpty ? (
            <Text style={[typeScale.sub, styles.emptyText, { color: tokens.ink3 }]}>no matches</Text>
          ) : null}
        </ScrollView>
      )}

      {overlay.body != null && !isNativeOverlayKind(overlay.kind) ? (
        <ScrollView style={[styles.bodyWell, { backgroundColor: tokens.bg0, borderRadius: radii.radius12 }]}>
          <Text style={[typeScale.codeSmall, { color: tokens.ink2 }]} selectable>
            {overlay.body}
          </Text>
        </ScrollView>
      ) : null}

      {overlay.free_text ? (
        <View style={styles.freeTextRow}>
          <Input
            value={freeText}
            onChangeText={setFreeText}
            placeholder="type and press enter…"
            onSubmitEditing={commitFreeText}
            returnKeyType="send"
            containerStyle={styles.freeTextInput}
            accessibilityLabel="free-text command"
          />
          <IconButton
            icon={<Send size={20} strokeWidth={1.75} color={tokens.ink} />}
            onPress={commitFreeText}
            disabled={freeText.trim().length === 0}
            accessibilityLabel="commit free-text command"
          />
        </View>
      ) : null}
    </View>
  );

  if (isCompact) {
    return (
      <Sheet visible={visible} onClose={onClose} accessibilityLabel={overlay.title}>
        {content}
      </Sheet>
    );
  }

  return (
    <CenteredModal visible={visible} onClose={onClose} accessibilityLabel={overlay.title}>
      {content}
    </CenteredModal>
  );
}

const MODAL_WIDTH = 560;

/** DESIGN_SYSTEM §7 "centered modal 560pt (wide)" twin of Sheet's Anvil pattern
 * (fade + scale in/out rather than translateY, since there's no bottom edge to
 * follow on medium/expanded layouts). Web: Esc closes, matching Sheet's parity. */
function CenteredModal({
  children,
  visible,
  onClose,
  accessibilityLabel,
}: {
  children: React.ReactNode;
  visible: boolean;
  onClose: () => void;
  accessibilityLabel?: string;
}) {
  const tokens = useTokens();
  const { scheme } = useTheme();
  const { height: windowHeight } = useWindowDimensions();
  const reduced = useReducedMotion();
  const depth = scheme === "dark" ? depthDark : depthLight;

  const [mounted, setMounted] = useState(visible);
  const opacity = useSharedValue(0);
  const scale = useSharedValue(0.96);

  useEffect(() => {
    if (visible) setMounted(true);
  }, [visible]);

  useEffect(() => {
    if (!mounted) return;
    if (visible) {
      if (reduced) {
        opacity.value = 1;
        scale.value = 1;
        return;
      }
      opacity.value = withTiming(1, { duration: durations.fast, easing: easings.standard });
      scale.value = withTiming(1, { duration: durations.gentle, easing: easings.standard });
    } else {
      if (reduced) {
        opacity.value = 0;
        scale.value = 0.96;
        setMounted(false);
        return;
      }
      opacity.value = withTiming(0, { duration: durations.fast, easing: easings.exit }, (finished) => {
        if (finished) runOnJS(setMounted)(false);
      });
      scale.value = withTiming(0.96, { duration: durations.fast, easing: easings.exit });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible, mounted, reduced]);

  useEffect(() => {
    if (Platform.OS !== "web") return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose]);

  const scrimStyle = useAnimatedStyle(() => ({ opacity: opacity.value }));
  const cardStyle = useAnimatedStyle(() => ({
    opacity: opacity.value,
    transform: [{ scale: scale.value }],
  }));

  if (!mounted) return null;

  return (
    <Modal visible transparent animationType="none" onRequestClose={onClose} statusBarTranslucent>
      <View style={styles.centeredWrap}>
        <Animated.View style={[StyleSheet.absoluteFill, { backgroundColor: tokens.overlayScrim }, scrimStyle]}>
          <Pressable
            style={StyleSheet.absoluteFill}
            onPress={onClose}
            accessibilityRole="button"
            accessibilityLabel="Close"
          />
        </Animated.View>
        <Animated.View
          style={[
            styles.centeredCard,
            { backgroundColor: tokens.bg2, borderRadius: radii.radius16, maxHeight: windowHeight * 0.8 },
            depth.raised ?? depth.sheet,
            cardStyle,
          ]}
          accessibilityViewIsModal
          accessibilityLabel={accessibilityLabel}
        >
          {children}
        </Animated.View>
      </View>
    </Modal>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, paddingHorizontal: space.space16, paddingBottom: space.space16, gap: space.space8 },
  titleRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingVertical: space.space8,
  },
  searchRow: { paddingBottom: space.space4 },
  rows: { flex: 1 },
  emptyText: { textAlign: "center", paddingVertical: space.space24 },
  bodyWell: { maxHeight: 240, padding: space.space12, marginTop: space.space4 },
  freeTextRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingTop: space.space8 },
  freeTextInput: { flex: 1 },
  centeredWrap: { flex: 1, alignItems: "center", justifyContent: "center", padding: space.space24 },
  centeredCard: { width: MODAL_WIDTH, maxWidth: "100%", overflow: "hidden" },
});
