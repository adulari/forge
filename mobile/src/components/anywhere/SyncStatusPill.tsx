// Desktop title-bar pill for account sync: one short state, and a popover with the per-category
// detail. Hidden unless this machine is signed in to Anywhere. It reads the shared
// `useAccountSync` query, so it adds no polling of its own.
import { router } from "expo-router";
import { Check } from "lucide-react-native";
import React, { useRef, useState } from "react";
import { Modal, Pressable, StyleSheet, Text, View, useWindowDimensions } from "react-native";

import { ageLabel, categoryStatusLine, syncPillModel, type SyncPillTone } from "../../lib/accountSync";
import { useAccountSync, useRunAccountSyncNow } from "../../lib/queries";
import { useOverlayEscape } from "../../lib/useOverlayEscape";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space, type ColorTokens } from "../../theme/tokens";
import { monoFamily } from "../../theme/typography";

function toneColor(tone: SyncPillTone, tokens: ColorTokens): string {
  switch (tone) {
    case "success": return tokens.success;
    case "warn": return tokens.warn;
    case "danger": return tokens.danger;
    case "neutral": return tokens.ink2;
    default: return tokens.ink4;
  }
}

export function SyncStatusPill() {
  const tokens = useTokens();
  const { width: windowWidth } = useWindowDimensions();
  const status = useAccountSync().data;
  const syncNow = useRunAccountSyncNow();
  const ref = useRef<View>(null);
  const [anchor, setAnchor] = useState<{ top: number; right: number } | null>(null);
  const close = () => setAnchor(null);
  useOverlayEscape(anchor != null, close);
  const pill = syncPillModel(status);
  if (!pill || !status) return null;
  const color = toneColor(pill.tone, tokens);
  const open = () =>
    ref.current?.measureInWindow((x, y, width, height) =>
      setAnchor({ top: y + height + 4, right: Math.max(8, windowWidth - x - width) }),
    );
  const attention = [
    ...status.categories.flatMap((category) => category.conflicts.map((c) => `${c.item}: ${c.detail}`)),
    ...status.categories.flatMap((category) => category.skipped.map((s) => `${s.item}: ${s.reason}`)),
    ...status.sign_ins.map((signIn) => `Sign in to ${signIn.label}: ${signIn.command}`),
  ];

  return (
    <>
      <Pressable
        ref={ref}
        onPress={open}
        accessibilityRole="button"
        accessibilityLabel={`${pill.label}. ${pill.description}. Open sync details`}
        accessibilityState={{ expanded: anchor != null }}
        style={({ pressed, hovered, focused }: { pressed: boolean; hovered?: boolean; focused?: boolean }) => [
          styles.pill,
          { borderColor: focused ? tokens.accent : hovered || pressed || anchor ? tokens.borderStrong : tokens.border },
        ]}
        {...({ title: pill.description } as object)}
      >
        {pill.tone === "success" ? (
          <Check size={11} color={color} strokeWidth={2.25} />
        ) : (
          <View style={[styles.dot, { backgroundColor: color }]} />
        )}
        <Text style={[styles.label, { color: pill.tone === "muted" ? tokens.ink4 : tokens.ink2 }]} numberOfLines={1}>
          {pill.label}
        </Text>
      </Pressable>

      <Modal visible={anchor != null} transparent animationType="none" onRequestClose={close}>
        <Pressable style={StyleSheet.absoluteFill} onPress={close} accessibilityLabel="Close sync details" />
        {anchor ? (
          <View
            style={[styles.popover, { top: anchor.top, right: anchor.right, backgroundColor: tokens.bg2, borderColor: tokens.border }]}
            accessibilityViewIsModal
          >
            <Text style={[styles.heading, { color: tokens.ink }]}>Sync across machines</Text>
            {status.categories.map((category) => {
              const line = categoryStatusLine(category);
              return (
                <View key={category.id} style={styles.line}>
                  <View style={[styles.dot, { backgroundColor: category.enabled ? tokens.success : tokens.ink4 }]} />
                  <Text style={[styles.lineLabel, { color: category.enabled ? tokens.ink : tokens.ink3 }]} numberOfLines={1}>
                    {category.label}
                  </Text>
                  <Text style={[styles.lineMeta, { color: line.tone === "warn" ? tokens.warn : tokens.ink4 }]} numberOfLines={1}>
                    {category.enabled ? `${category.items} items · ${category.pending > 0 ? `${category.pending} pending · ` : ""}${ageLabel(category.last_sync_at)}` : "off"}
                  </Text>
                </View>
              );
            })}
            {attention.length > 0 ? (
              <View style={[styles.notes, { borderTopColor: tokens.border }]}>
                {attention.slice(0, 4).map((note) => (
                  <Text key={note} style={[styles.note, { color: tokens.ink3 }]} numberOfLines={2}>
                    {note}
                  </Text>
                ))}
                {attention.length > 4 ? (
                  <Text style={[styles.note, { color: tokens.ink4 }]}>{`+${attention.length - 4} more`}</Text>
                ) : null}
              </View>
            ) : null}
            <View style={[styles.footer, { borderTopColor: tokens.border }]}>
              <Pressable
                onPress={() => syncNow.mutate()}
                disabled={syncNow.isPending || !status.categories.some((category) => category.enabled)}
                accessibilityRole="button"
                accessibilityLabel="Sync now"
                style={({ hovered, focused }: { pressed: boolean; hovered?: boolean; focused?: boolean }) => [
                  styles.action,
                  { borderColor: focused ? tokens.accent : tokens.border, backgroundColor: hovered ? tokens.bg3 : "transparent" },
                  !status.categories.some((category) => category.enabled) && { opacity: 0.4 },
                ]}
              >
                <Text style={[styles.actionText, { color: tokens.ink }]}>{syncNow.isPending ? "Syncing…" : "Sync now"}</Text>
              </Pressable>
              <Pressable
                onPress={() => {
                  close();
                  router.push("/anywhere/sync");
                }}
                accessibilityRole="link"
                accessibilityLabel="Open sync settings"
                style={({ focused }: { pressed: boolean; focused?: boolean }) => [styles.link, focused && { borderColor: tokens.accent }]}
              >
                <Text style={[styles.actionText, { color: tokens.accent }]}>Open sync settings</Text>
              </Pressable>
            </View>
            <Text style={[styles.note, { color: tokens.ink4 }]}>{`Last sync ${ageLabel(status.last_sync_at)}`}</Text>
          </View>
        ) : null}
      </Modal>
    </>
  );
}

const styles = StyleSheet.create({
  pill: { flexDirection: "row", alignItems: "center", gap: 5, borderWidth: 1, borderRadius: radii.radius4, paddingHorizontal: 7, paddingVertical: 2 },
  dot: { width: 5, height: 5, borderRadius: 2.5 },
  label: { fontFamily: monoFamily.regular, fontSize: 10.5 },
  popover: {
    position: "absolute",
    width: 340,
    borderWidth: 1,
    borderRadius: radii.radius8,
    padding: space.space12,
    gap: space.space8,
    shadowColor: "#000",
    shadowOpacity: 0.35,
    shadowRadius: 16,
    shadowOffset: { width: 0, height: 6 },
  },
  heading: { fontSize: 12.5, fontWeight: "600" },
  line: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  lineLabel: { fontSize: 12, width: 112 },
  lineMeta: { flex: 1, fontFamily: monoFamily.regular, fontSize: 10.5, textAlign: "right" },
  notes: { borderTopWidth: StyleSheet.hairlineWidth, paddingTop: space.space8, gap: space.space4 },
  note: { fontFamily: monoFamily.regular, fontSize: 10.5 },
  footer: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", borderTopWidth: StyleSheet.hairlineWidth, paddingTop: space.space8 },
  action: { borderWidth: 1, borderRadius: radii.radius4, paddingHorizontal: 10, paddingVertical: 4 },
  link: { borderWidth: 1, borderColor: "transparent", borderRadius: radii.radius4, paddingHorizontal: 6, paddingVertical: 4 },
  actionText: { fontSize: 12, fontWeight: "500" },
});
