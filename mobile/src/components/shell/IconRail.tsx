// Machined desktop shell — the 48px collapsed icon rail (D Rail + Terminal,
// docs/design/machined INVENTORY.md L140-155): nav icons, one 30x30 icon button per
// session with a status-dot badge, a usage-pace ring column, settings, and the
// expand affordance. `Sidebar`'s dense-row twin — same session data, icon density.
import { router, usePathname } from "expo-router";
import { Flame, Search, Settings2, SquareChevronRight } from "lucide-react-native";
import React, { useMemo } from "react";
import { Pressable, ScrollView, StyleSheet, View } from "react-native";

import { useSessions } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space, statusDotColor, type StatusDotState } from "../../theme/tokens";
import { usePalette } from "../overlay/CommandPalette";
import { useProviderPace, UsageRing } from "./UsageRing";

const ICON_SIZE = 30;

function RailIconButton({
  icon,
  onPress,
  accessibilityLabel,
  accessibilityHint,
  active,
}: {
  icon: React.ReactNode;
  onPress: () => void;
  accessibilityLabel: string;
  accessibilityHint?: string;
  active?: boolean;
}) {
  const tokens = useTokens();
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel}
      accessibilityHint={accessibilityHint}
      style={[styles.iconButton, active && { backgroundColor: tokens.bg3 }]}
    >
      {icon}
    </Pressable>
  );
}

function SessionIconButton({
  title,
  state,
  worktree,
  selected,
  onPress,
}: {
  title: string;
  state: StatusDotState;
  worktree: boolean;
  selected: boolean;
  onPress: () => void;
}) {
  const tokens = useTokens();
  const glyphColor = state === "idle" ? tokens.ink4 : selected ? tokens.ink : tokens.ink2;
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={`${title}, ${state}${worktree ? ", worktree" : ""}`}
      style={[styles.sessionButton, selected && { backgroundColor: tokens.bg3 }]}
    >
      <Flame size={13} strokeWidth={1.75} color={worktree ? tokens.accent : glyphColor} />
      {state !== "idle" ? (
        <View style={[styles.badge, { backgroundColor: statusDotColor(state, tokens), borderColor: tokens.bg1 }]} />
      ) : null}
    </Pressable>
  );
}

export interface IconRailProps {
  onExpand: () => void;
  /** See Sidebar's identical prop — the ring column doubles as the ⌘U dock toggle. */
  onToggleDock?: () => void;
}

export function IconRail({ onExpand, onToggleDock }: IconRailProps) {
  const tokens = useTokens();
  const pathname = usePathname();
  const palette = usePalette();
  const sessionsQuery = useSessions();
  const rows = useMemo(() => sessionsQuery.data ?? [], [sessionsQuery.data]);
  const selectedSessionId = pathname.match(/^\/session\/([^/]+)/)?.[1];
  const { rings } = useProviderPace();

  return (
    <View style={styles.rail}>
      <RailIconButton
        icon={<Flame size={15} strokeWidth={1.75} color={tokens.accent} />}
        onPress={() => router.push("/")}
        accessibilityLabel="Fleet"
        active={pathname === "/"}
      />
      <RailIconButton
        icon={<Search size={14} strokeWidth={1.75} color={tokens.ink2} />}
        onPress={() => palette.open("default")}
        accessibilityLabel="Search or command"
      />
      <View style={[styles.divider, { backgroundColor: tokens.border }]} />

      <ScrollView style={styles.sessionList} contentContainerStyle={styles.sessionListContent} showsVerticalScrollIndicator={false}>
        {rows.map((row) => {
          const title = row.title || `session ${row.id.slice(0, 8)}`;
          const state: StatusDotState = row.waiting ? "waiting" : row.busy ? "busy" : "idle";
          return (
            <SessionIconButton
              key={row.id}
              title={title}
              state={state}
              worktree={!!row.worktree}
              selected={row.id === selectedSessionId}
              onPress={() => router.push(`/session/${row.id}`)}
            />
          );
        })}
      </ScrollView>

      <View style={styles.footer}>
        {rings.length > 0 ? (
          <>
            <View style={[styles.divider, { backgroundColor: tokens.border }]} />
            <Pressable
              onPress={onToggleDock}
              disabled={!onToggleDock}
              accessibilityRole={onToggleDock ? "button" : undefined}
              accessibilityLabel="Toggle usage dock"
              accessibilityHint="Command U"
              style={styles.ringColumn}
            >
              {rings.map((ring) => (
                <UsageRing key={ring.provider} pct={ring.pct} accessibilityLabel={`${ring.provider} usage ${ring.pct}%`} />
              ))}
            </Pressable>
          </>
        ) : null}
        <View style={[styles.divider, { backgroundColor: tokens.border }]} />
        <RailIconButton
          icon={<Settings2 size={14} strokeWidth={1.75} color={tokens.ink2} />}
          onPress={() => router.push("/settings")}
          accessibilityLabel="Settings"
          active={pathname.startsWith("/settings")}
        />
        <RailIconButton
          icon={<SquareChevronRight size={14} strokeWidth={1.75} color={tokens.ink3} />}
          onPress={onExpand}
          accessibilityLabel="Expand sidebar"
          accessibilityHint="Command Backslash"
        />
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  rail: { flex: 1, alignItems: "center", paddingVertical: space.space8, gap: 4 },
  iconButton: {
    width: ICON_SIZE,
    height: ICON_SIZE,
    borderRadius: radii.radius8,
    alignItems: "center",
    justifyContent: "center",
  },
  divider: { height: StyleSheet.hairlineWidth, width: 24, marginVertical: 4 },
  sessionList: { flexGrow: 0, alignSelf: "stretch" },
  sessionListContent: { alignItems: "center", gap: 4, paddingVertical: 2 },
  sessionButton: {
    width: ICON_SIZE,
    height: ICON_SIZE,
    borderRadius: radii.radius8,
    alignItems: "center",
    justifyContent: "center",
  },
  badge: { position: "absolute", top: 4, right: 4, width: 6, height: 6, borderRadius: 3, borderWidth: 1 },
  footer: { marginTop: "auto", alignItems: "center", gap: 4, paddingBottom: 2 },
  ringColumn: { alignItems: "center", gap: space.space8, paddingVertical: 2 },
});
