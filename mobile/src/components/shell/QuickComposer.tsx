// Machined desktop shell — the ⌥Space quick composer (D Quick Composer, docs/design/
// machined INVENTORY.md L348-358): a single growing text row floating over the
// window, a compact chip row (project / permission tier / worktree), and a send
// button that creates a session through the SAME `useCreateSession` mutation
// new-session.tsx uses — no new endpoint. `⇧↵` bails to that full form instead,
// carrying the typed text and chosen project over as route params (the same
// `?title=` pattern the Fleet rail's TaskComposer already uses).
import { router } from "expo-router";
import React, { useEffect, useState } from "react";
import {
  Modal,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
  useWindowDimensions,
} from "react-native";
import Animated, {
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withTiming,
} from "react-native-reanimated";

import { useCreateSession, useProjects } from "../../lib/queries";
import { durations, easings } from "../../theme/motion";
import { useTheme, useTokens } from "../../theme/ThemeProvider";
import { depthDark, depthLight, radii, space } from "../../theme/tokens";
import { formatCwd, type as typeScale, webInputTextStyle } from "../../theme/typography";
import { ListRow } from "../ds/ListRow";
import { SectionHeader } from "../ds/SectionHeader";
import { Sheet } from "../ds/Sheet";

type Temper = "Read-only" | "Ask" | "Auto-edit" | "Full";
const TEMPER_CYCLE: Temper[] = ["Read-only", "Ask", "Auto-edit", "Full"];
const TEMPER_LABEL: Record<Temper, string> = { "Read-only": "READ", Ask: "ASK", "Auto-edit": "EDIT", Full: "FULL" };

const PANEL_WIDTH = 640;

function Chip({ label, onPress, tone }: { label: string; onPress?: () => void; tone?: "muted" }) {
  const tokens = useTokens();
  return (
    <Pressable
      onPress={onPress}
      disabled={!onPress}
      accessibilityRole={onPress ? "button" : undefined}
      accessibilityLabel={label}
      style={[styles.chip, { borderColor: tone === "muted" ? tokens.hairline : tokens.border }]}
    >
      <Text style={[typeScale.monoMeta, { color: tone === "muted" ? tokens.ink4 : tokens.ink2 }]}>{label}</Text>
    </Pressable>
  );
}

export interface QuickComposerProps {
  visible: boolean;
  onClose: () => void;
}

export function QuickComposer({ visible, onClose }: QuickComposerProps) {
  const tokens = useTokens();
  const { scheme } = useTheme();
  const { height: windowHeight } = useWindowDimensions();
  const reduced = useReducedMotion();
  const depth = scheme === "dark" ? depthDark : depthLight;

  const projects = useProjects();
  const createSession = useCreateSession();

  const [text, setText] = useState("");
  const [cwd, setCwd] = useState("");
  const [worktree, setWorktree] = useState(true);
  const [temper, setTemper] = useState<Temper>("Ask");
  const [projectPickerVisible, setProjectPickerVisible] = useState(false);

  useEffect(() => {
    if (visible) {
      setText("");
      setTemper("Ask");
      setWorktree(true);
      setCwd(projects.data?.default_cwd ?? "");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible]);

  const [mounted, setMounted] = useState(visible);
  const opacity = useSharedValue(0);
  const translateY = useSharedValue(8);

  useEffect(() => {
    if (visible) setMounted(true);
  }, [visible]);

  useEffect(() => {
    if (!mounted) return;
    if (visible) {
      if (reduced) {
        opacity.value = 1;
        translateY.value = 0;
        return;
      }
      opacity.value = withTiming(1, { duration: durations.fast, easing: easings.standard });
      translateY.value = withTiming(0, { duration: durations.fast, easing: easings.standard });
    } else if (reduced) {
      opacity.value = 0;
      setMounted(false);
    } else {
      opacity.value = withTiming(0, { duration: durations.fast, easing: easings.exit });
      translateY.value = withTiming(8, { duration: durations.fast, easing: easings.exit });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible, mounted, reduced]);

  const panelStyle = useAnimatedStyle(() => ({
    opacity: opacity.value,
    transform: [{ translateY: translateY.value }],
  }));

  const cycleTemper = () => setTemper((t) => TEMPER_CYCLE[(TEMPER_CYCLE.indexOf(t) + 1) % TEMPER_CYCLE.length]);

  const forgeNow = () => {
    const trimmed = text.trim();
    if (!trimmed || createSession.isPending) return;
    createSession.mutate(
      { cwd: cwd || undefined, title: trimmed, worktree, temper },
      { onSuccess: (res) => router.push(`/session/${res.id}`) },
    );
    onClose();
  };

  const openFull = () => {
    const trimmed = text.trim();
    onClose();
    router.push({ pathname: "/new-session", params: { title: trimmed || undefined, cwd: cwd || undefined } });
  };

  useEffect(() => {
    if (!visible || Platform.OS !== "web") return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      } else if (e.key === "Enter" && e.shiftKey) {
        e.preventDefault();
        openFull();
      } else if (e.key === "Enter") {
        e.preventDefault();
        forgeNow();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible, text, cwd, worktree, temper]);

  if (!mounted) return null;

  const projectLabel = cwd ? formatCwd(cwd) : "project";
  const recentProjects = projects.data?.recent ?? [];

  return (
    <Modal visible={mounted} transparent animationType="none" onRequestClose={onClose} statusBarTranslucent>
      <View style={styles.centerWrap}>
        <Pressable
          style={StyleSheet.absoluteFill}
          onPress={onClose}
          accessibilityRole="button"
          accessibilityLabel="Close quick composer"
        />
        <Animated.View
          style={[
            styles.panel,
            {
              backgroundColor: tokens.bg2,
              borderColor: tokens.borderStrong,
              borderRadius: radii.radius16,
              width: Math.min(PANEL_WIDTH, windowHeight),
            },
            depth.sheet,
            panelStyle,
          ]}
          accessibilityViewIsModal
          accessibilityLabel="Quick composer"
        >
          <TextInput
            value={text}
            onChangeText={setText}
            placeholder="Forge a task…"
            placeholderTextColor={tokens.ink3}
            multiline
            autoFocus
            cursorColor={tokens.accent}
            selectionColor={tokens.accent}
            style={[styles.input, webInputTextStyle, { color: tokens.ink }]}
            accessibilityLabel="Describe a task to forge"
          />
          <View style={styles.chipRow}>
            <Chip label={projectLabel} onPress={() => setProjectPickerVisible(true)} />
            <Chip label="Automatic" />
            <Chip label={TEMPER_LABEL[temper]} onPress={cycleTemper} />
            <Chip label={worktree ? "⑂ worktree" : "no worktree"} onPress={() => setWorktree((w) => !w)} tone={worktree ? undefined : "muted"} />
            <View style={styles.chipSpacer} />
            <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>↵ forge · ⇧↵ open full</Text>
            <Pressable
              onPress={forgeNow}
              disabled={!text.trim() || createSession.isPending}
              accessibilityRole="button"
              accessibilityLabel="Forge session"
              style={[styles.send, { backgroundColor: tokens.accent, opacity: text.trim() ? 1 : 0.5 }]}
            >
              <Text style={[typeScale.bodyBold, { color: tokens.onAccent }]}>↑</Text>
            </Pressable>
          </View>
        </Animated.View>
      </View>

      <Sheet visible={projectPickerVisible} onClose={() => setProjectPickerVisible(false)} accessibilityLabel="Choose project">
        <View style={styles.pickerContent}>
          <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Choose project</Text>
          <SectionHeader>Recent</SectionHeader>
          {recentProjects.length === 0 ? (
            <Text style={[typeScale.sub, styles.pickerEmpty, { color: tokens.ink3 }]}>No recent projects yet.</Text>
          ) : (
            recentProjects.map((project, index) => (
              <ListRow
                key={project.path}
                title={project.name}
                subtitle={project.path}
                onPress={() => {
                  setCwd(project.path);
                  setProjectPickerVisible(false);
                }}
                showSeparator={index < recentProjects.length - 1}
              />
            ))
          )}
        </View>
      </Sheet>
    </Modal>
  );
}

const styles = StyleSheet.create({
  centerWrap: { flex: 1, alignItems: "center", justifyContent: "center", padding: space.space24 },
  panel: { maxWidth: "100%", borderWidth: 1, overflow: "hidden" },
  input: {
    fontSize: 15,
    lineHeight: 22,
    minHeight: 26,
    maxHeight: 140,
    paddingHorizontal: space.space16,
    paddingTop: space.space16,
    paddingBottom: space.space8,
  },
  chipRow: {
    flexDirection: "row",
    alignItems: "center",
    flexWrap: "wrap",
    gap: space.space8,
    paddingHorizontal: space.space12,
    paddingBottom: space.space12,
  },
  chip: { borderWidth: 1, borderRadius: radii.radius4, paddingHorizontal: space.space8, paddingVertical: 4 },
  chipSpacer: { flex: 1, minWidth: space.space8 },
  send: {
    width: 23,
    height: 23,
    borderRadius: radii.radius4,
    alignItems: "center",
    justifyContent: "center",
  },
  pickerContent: { paddingHorizontal: space.space16, paddingBottom: space.space32, gap: space.space4 },
  pickerEmpty: { paddingHorizontal: space.space16, paddingVertical: space.space12 },
});
