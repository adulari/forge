// The few small pieces every IDE pane shares. Kept here so the panes stay free of one-off
// styling: 24px icon buttons, keycaps, mono section labels, the agent status dot.
import React from "react";
import { Pressable, StyleSheet, Text, View, type StyleProp, type ViewStyle } from "react-native";
import Animated from "react-native-reanimated";

import { useEmberdot } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, sansFamilyFor } from "../../theme/typography";

export const ROW_HEIGHT = 28;

export function IdeIconButton({
  icon,
  label,
  onPress,
  disabled,
  active,
  style,
}: {
  icon: React.ReactNode;
  label: string;
  onPress: () => void;
  disabled?: boolean;
  active?: boolean;
  style?: StyleProp<ViewStyle>;
}) {
  const tokens = useTokens();
  return (
    <Pressable
      onPress={disabled ? undefined : onPress}
      disabled={disabled}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled: !!disabled, selected: !!active }}
      // @ts-expect-error react-native-web tooltip
      title={label}
      style={({ hovered, pressed }: { hovered?: boolean; pressed: boolean }) => [
        styles.iconButton,
        { backgroundColor: active || hovered || pressed ? tokens.bg3 : "transparent", opacity: disabled ? 0.4 : 1 },
        style,
      ]}
    >
      {icon}
    </Pressable>
  );
}

/** A compact text button for hunk, review and panel actions: icon, label, optional key hint. */
export function IdeTextButton({
  label,
  onPress,
  icon,
  keyHint,
  tone = "neutral",
  disabled,
  loading,
  active,
}: {
  label: string;
  onPress: () => void;
  icon?: React.ReactNode;
  keyHint?: string;
  tone?: "neutral" | "accent" | "danger";
  disabled?: boolean;
  loading?: boolean;
  active?: boolean;
}) {
  const tokens = useTokens();
  const ink = tone === "accent" ? tokens.accent : tone === "danger" ? tokens.danger : tokens.ink2;
  return (
    <Pressable
      onPress={disabled || loading ? undefined : onPress}
      disabled={disabled || loading}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled: !!disabled, busy: !!loading, selected: !!active }}
      style={({ hovered, pressed }: { hovered?: boolean; pressed: boolean }) => [
        styles.textButton,
        { backgroundColor: active || hovered || pressed ? tokens.bg3 : "transparent", borderColor: tone === "accent" ? tokens.accent : tokens.border, opacity: disabled ? 0.4 : loading ? 0.6 : 1 },
      ]}
    >
      {icon}
      <Text style={[styles.textButtonLabel, { color: ink }]}>{label}</Text>
      {keyHint ? <Keycap label={keyHint} /> : null}
    </Pressable>
  );
}

/** A keyboard key as a small outlined cap. */
export function Keycap({ label }: { label: string }) {
  const tokens = useTokens();
  return (
    <View style={[styles.keycap, { borderColor: tokens.borderStrong }]}>
      <Text style={[styles.keycapText, { color: tokens.ink2 }]}>{label}</Text>
    </View>
  );
}

export function SectionLabel({ children, right }: { children: string; right?: React.ReactNode }) {
  const tokens = useTokens();
  return (
    <View style={styles.sectionLabel}>
      <Text style={[styles.sectionText, { color: tokens.ink3 }]}>{children}</Text>
      {right}
    </View>
  );
}

export type AgentDotState = "waiting" | "busy" | "idle" | "done";

/** Needs-you is the only accent in the workbench; a working agent is a quiet pulse. */
export function AgentDot({ state }: { state: AgentDotState }) {
  const tokens = useTokens();
  const { dotStyle } = useEmberdot(state === "busy" ? "busy" : "idle");
  const color = state === "waiting" ? tokens.accent : state === "busy" ? tokens.ink2 : state === "done" ? tokens.ink4 : tokens.ink3;
  return (
    <Animated.View
      style={[styles.dot, { backgroundColor: state === "idle" ? "transparent" : color, borderColor: color }, state === "busy" ? dotStyle : null]}
      accessibilityLabel={state === "waiting" ? "Needs you" : state}
    />
  );
}

export function LineDelta({ adds, dels, dim }: { adds: number; dels: number; dim?: boolean }) {
  const tokens = useTokens();
  if (adds === 0 && dels === 0) return null;
  return (
    <Text style={[styles.delta, { opacity: dim ? 0.55 : 1 }]} numberOfLines={1}>
      {adds > 0 ? <Text style={{ color: tokens.success }}>+{adds}</Text> : null}
      {adds > 0 && dels > 0 ? " " : null}
      {dels > 0 ? <Text style={{ color: tokens.danger }}>−{dels}</Text> : null}
    </Text>
  );
}

export function PaneFill({ children, background }: { children: React.ReactNode; background: string }) {
  return <View style={[styles.fill, { backgroundColor: background }]}>{children}</View>;
}

const styles = StyleSheet.create({
  textButton: { height: 22, flexDirection: "row", alignItems: "center", gap: 5, paddingHorizontal: space.space8, borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4 },
  textButtonLabel: { fontFamily: sansFamilyFor(500), fontSize: 11 },
  iconButton: { width: 24, height: 24, borderRadius: radii.radius4, alignItems: "center", justifyContent: "center" },
  keycap: {
    minWidth: 16,
    height: 16,
    paddingHorizontal: 4,
    borderWidth: 1,
    borderRadius: radii.radius4,
    alignItems: "center",
    justifyContent: "center",
  },
  keycapText: { fontFamily: monoFamily.medium, fontSize: 10, lineHeight: 12 },
  sectionLabel: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", height: 24, paddingHorizontal: space.space12 },
  sectionText: { fontFamily: sansFamilyFor(600), fontSize: 10, letterSpacing: 0.8, textTransform: "uppercase" },
  dot: { width: 7, height: 7, borderRadius: 3.5, borderWidth: 1 },
  delta: { fontFamily: monoFamily.regular, fontSize: 10.5, lineHeight: 14 },
  fill: { flex: 1, minWidth: 0, minHeight: 0 },
});
