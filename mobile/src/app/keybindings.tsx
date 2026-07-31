import { Keyboard, RotateCcw } from "lucide-react-native";
import React, { useEffect, useMemo, useState } from "react";
import { Platform, Pressable, StyleSheet, Text, View } from "react-native";

import { BackLink } from "../components/ds/BackLink";
import { Button } from "../components/ds/Button";
import { IconButton } from "../components/ds/IconButton";
import { Screen } from "../components/ds/Screen";
import { SearchField } from "../components/ds/SearchField";
import { SectionHeader } from "../components/ds/SectionHeader";
import { Sheet } from "../components/ds/Sheet";
import { useToast } from "../components/ds/ToastHost";
import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import {
  APP_SHORTCUTS,
  type AppShortcutAction,
  type AppShortcutDescriptor,
  type ShortcutBinding,
  findShortcutConflicts,
  formatShortcut,
  getResolvedShortcut,
  isDefaultShortcut,
  resetAllAppShortcuts,
  resetAppShortcut,
  setAppShortcut,
  shortcutBindingFromEvent,
  useShortcutPreferences,
} from "../lib/shortcuts";
import { useTokens } from "../theme/ThemeProvider";
import { radii, space } from "../theme/tokens";
import { monoFamily, type } from "../theme/typography";
import { SettingsShell } from "./(tabs)/settings";

const GROUPS = ["Navigation", "Workbench", "Session"] as const;

function ShortcutRow({
  descriptor,
  binding,
  editable,
  onEdit,
  onReset,
}: {
  descriptor: AppShortcutDescriptor;
  binding: ShortcutBinding;
  editable: boolean;
  onEdit: () => void;
  onReset: () => void;
}) {
  const tokens = useTokens();
  const conflicts = findShortcutConflicts(descriptor.action, binding);
  const isDefault = isDefaultShortcut(descriptor.action, binding);
  return (
    <View style={[styles.row, { borderBottomColor: tokens.hairline }]}>
      <Pressable
        onPress={editable ? onEdit : undefined}
        disabled={!editable}
        accessibilityRole="button"
        accessibilityLabel={`${descriptor.label}, ${formatShortcut(binding)}`}
        accessibilityHint={editable ? "Capture a different shortcut" : "Editable on desktop and web"}
        accessibilityState={{ disabled: !editable }}
        style={styles.rowMain}
      >
        <View style={styles.rowCopy}>
          <View style={styles.labelLine}>
            <Text style={[type.bodyBold, { color: tokens.ink }]}>{descriptor.label}</Text>
            {isDefault ? <Text style={[type.meta, { color: tokens.ink4 }]}>default</Text> : null}
          </View>
          <Text style={[type.sub, { color: tokens.ink3 }]}>{descriptor.description}</Text>
          {conflicts.length > 0 ? (
            <Text style={[type.sub, { color: tokens.warn }]}>
              Also used by {conflicts.map(({ label }) => label).join(", ")}
            </Text>
          ) : null}
        </View>
        <View style={[styles.chord, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}>
          <Text style={[type.monoMeta, styles.chordText, { color: tokens.ink }]}>{formatShortcut(binding)}</Text>
        </View>
      </Pressable>
      <IconButton
        icon={<RotateCcw size={17} strokeWidth={1.75} color={tokens.ink3} />}
        accessibilityLabel={`Reset ${descriptor.label}`}
        onPress={isDefault || !editable ? undefined : onReset}
        disabled={isDefault || !editable}
      />
    </View>
  );
}

export default function KeybindingsScreen() {
  const tokens = useTokens();
  const toast = useToast();
  const { loaded, overrides } = useShortcutPreferences();
  const [search, setSearch] = useState("");
  const [capturing, setCapturing] = useState<AppShortcutAction | null>(null);
  const [draft, setDraft] = useState<ShortcutBinding | null>(null);
  const editable = Platform.OS === "web" && loaded;
  const activeDescriptor = APP_SHORTCUTS.find(({ action }) => action === capturing) ?? null;
  const conflicts = capturing && draft ? findShortcutConflicts(capturing, draft, overrides) : [];

  const filtered = useMemo(() => {
    const needle = search.trim().toLocaleLowerCase();
    if (!needle) return APP_SHORTCUTS;
    return APP_SHORTCUTS.filter((descriptor) =>
      `${descriptor.label} ${descriptor.description} ${descriptor.group} ${descriptor.action}`
        .toLocaleLowerCase()
        .includes(needle),
    );
  }, [search]);

  useEffect(() => {
    if (Platform.OS !== "web" || !capturing || typeof window === "undefined") return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        setCapturing(null);
        setDraft(null);
        return;
      }
      const next = shortcutBindingFromEvent(event);
      if (!next) return;
      event.preventDefault();
      event.stopPropagation();
      setDraft(next);
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [capturing]);

  const beginCapture = (action: AppShortcutAction) => {
    setCapturing(action);
    setDraft(getResolvedShortcut(action, overrides));
  };

  const closeCapture = () => {
    setCapturing(null);
    setDraft(null);
  };

  const saveCapture = () => {
    if (!capturing || !draft) return;
    void setAppShortcut(capturing, draft)
      .then(() => {
        closeCapture();
        toast.show("Shortcut saved.", { tone: "neutral" });
      })
      .catch(() => toast.show("Couldn't save shortcut.", { tone: "danger" }));
  };

  const resetOne = (action: AppShortcutAction) => {
    void resetAppShortcut(action)
      .then(() => toast.show("Shortcut reset.", { tone: "neutral" }))
      .catch(() => toast.show("Couldn't reset shortcut.", { tone: "danger" }));
  };

  const resetAll = () => {
    void resetAllAppShortcuts()
      .then(() => toast.show("All app shortcuts reset.", { tone: "neutral" }))
      .catch(() => toast.show("Couldn't reset shortcuts.", { tone: "danger" }));
  };

  return (
    <DesktopDrillDown>
      <SettingsShell active="keybindings">
        <Screen scroll contentContainerStyle={styles.content}>
          <View style={styles.header}>
            <BackLink />
            <View style={styles.titleRow}>
              <Keyboard size={22} strokeWidth={1.75} color={tokens.accent} />
              <Text accessibilityRole="header" style={[type.title, { color: tokens.ink }]}>Keyboard shortcuts</Text>
            </View>
            <Text style={[type.sub, { color: tokens.ink3 }]}>
              These bindings control the Forge desktop and browser app. Host TUI keybindings remain in config.toml.
            </Text>
            <Text style={[type.sub, { color: tokens.ink3 }]}>
              On macOS, Forge keeps editable native menu accelerators synchronized. Standard system shortcuts stay reserved and appear as conflicts.
            </Text>
            {Platform.OS !== "web" ? (
              <Text style={[type.sub, { color: tokens.warn }]}>
                Shortcut capture is available on desktop and web; mobile keeps touch controls unchanged.
              </Text>
            ) : null}
          </View>

          <View style={styles.tools}>
            <SearchField
              value={search}
              onChangeText={setSearch}
              placeholder="Search commands"
              accessibilityLabel="Search keyboard shortcuts"
              containerStyle={styles.search}
            />
            <Button
              label="Reset all"
              variant="secondary"
              onPress={resetAll}
              disabled={!editable || Object.keys(overrides).length === 0}
              style={styles.resetAll}
            />
          </View>

          {GROUPS.map((group) => {
            const rows = filtered.filter((descriptor) => descriptor.group === group);
            if (rows.length === 0) return null;
            return (
              <View key={group}>
                <SectionHeader>{group}</SectionHeader>
                {rows.map((descriptor) => (
                  <ShortcutRow
                    key={descriptor.action}
                    descriptor={descriptor}
                    binding={getResolvedShortcut(descriptor.action, overrides)}
                    editable={editable}
                    onEdit={() => beginCapture(descriptor.action)}
                    onReset={() => resetOne(descriptor.action)}
                  />
                ))}
              </View>
            );
          })}

          {filtered.length === 0 ? (
            <Text style={[type.body, styles.empty, { color: tokens.ink3 }]}>No commands match “{search}”.</Text>
          ) : null}

          <Sheet
            visible={capturing != null}
            onClose={closeCapture}
            snapPoints={[0.55]}
            accessibilityLabel="Capture keyboard shortcut"
          >
            <View style={styles.capture}>
              <Text accessibilityRole="header" style={[type.heading, { color: tokens.ink }]}>
                {activeDescriptor?.label ?? "Keyboard shortcut"}
              </Text>
              <Text style={[type.sub, { color: tokens.ink3 }]}>
                Press the new key combination. Modifier-only presses are ignored; Escape cancels.
              </Text>
              <View style={[styles.captureChord, { backgroundColor: tokens.bg0, borderColor: tokens.borderStrong }]}>
                <Text style={[styles.captureChordText, { color: tokens.ink }]}>
                  {draft ? formatShortcut(draft) : "Waiting…"}
                </Text>
              </View>
              {conflicts.length > 0 ? (
                <Text style={[type.sub, { color: tokens.warn }]}>
                  Conflict with {conflicts.map(({ label }) => label).join(", ")}. You can still save; standard or native commands may take precedence.
                </Text>
              ) : (
                <Text style={[type.sub, { color: tokens.success }]}>No conflicts.</Text>
              )}
              <View style={styles.captureActions}>
                <Button label="Cancel" variant="ghost" onPress={closeCapture} style={styles.captureAction} />
                <Button label="Save shortcut" onPress={saveCapture} disabled={!draft} style={styles.captureAction} />
              </View>
            </View>
          </Sheet>
        </Screen>
      </SettingsShell>
    </DesktopDrillDown>
  );
}

const styles = StyleSheet.create({
  content: {
    paddingTop: space.space12,
    paddingBottom: space.space48,
    gap: space.space20,
  },
  header: {
    gap: space.space8,
  },
  titleRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
  },
  tools: {
    flexDirection: "row",
    alignItems: "center",
    flexWrap: "wrap",
    gap: space.space12,
  },
  search: {
    flex: 1,
    minWidth: 220,
  },
  resetAll: {
    flexShrink: 0,
  },
  row: {
    minHeight: 62,
    flexDirection: "row",
    alignItems: "center",
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  rowMain: {
    flex: 1,
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space12,
    paddingVertical: space.space8,
    paddingLeft: space.space12,
  },
  rowCopy: {
    flex: 1,
    minWidth: 0,
    gap: 2,
  },
  labelLine: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
  },
  chord: {
    minWidth: 72,
    minHeight: 30,
    alignItems: "center",
    justifyContent: "center",
    paddingHorizontal: space.space8,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius4,
  },
  chordText: {
    fontFamily: monoFamily.bold,
  },
  empty: {
    padding: space.space24,
    textAlign: "center",
  },
  capture: {
    paddingHorizontal: space.space20,
    paddingBottom: space.space24,
    gap: space.space12,
  },
  captureChord: {
    minHeight: 72,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 1,
    borderRadius: radii.radius8,
  },
  captureChordText: {
    fontFamily: monoFamily.bold,
    fontSize: 24,
  },
  captureActions: {
    flexDirection: "row",
    justifyContent: "flex-end",
    gap: space.space8,
  },
  captureAction: {
    minWidth: 116,
  },
});
