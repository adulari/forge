// Machined desktop shell — the 48px collapsed icon rail (D Rail + Terminal,
// docs/design/machined INVENTORY.md L140-155): nav icons, one 30x30 icon button per
// session with a status-dot badge, a usage-pace ring column, settings, and the
// expand affordance. `Sidebar`'s dense-row twin — same session data, icon density.
import { router, usePathname } from "expo-router";
import { Bell, BellDot, Flame, Gauge, Globe, History, Plus, Search, Settings2, SquareChevronRight } from "lucide-react-native";
import React, { useMemo } from "react";
import { Pressable, ScrollView, StyleSheet, View } from "react-native";

import type { SessionRow } from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { totalUnseen, useNoticesStore } from "../../lib/notices/store";
import { openSession } from "../../lib/nav";
import { requestSessionPrewarm, useSessionActivationOverride } from "../../lib/sessionPrewarm";
import { usePrefetchSessionHistory, useSessions } from "../../lib/queries";
import { providerIdOf } from "../../lib/providerFamily";
import { failedTurn, sessionStateWord } from "../../lib/sidebarModel";
import { useScopedFleet } from "../../lib/useScopedFleet";
import { useShortcutLabel } from "../../lib/shortcuts";
import { useTokens } from "../../theme/ThemeProvider";
import { hexToRgba, radii, space, statusDotColor, type StatusDotState } from "../../theme/tokens";
import { ProviderGlyph } from "../ds/ProviderGlyph";
import { useClockSeconds } from "../ds/relativeClock";
import { usePalette } from "../overlay/CommandPalette";
import { useWorkbench } from "../workbench/WorkbenchProvider";
import { ringSummary, useProviderPace, UsageRing } from "./UsageRing";

const ICON_SIZE = 30;

function RailIconButton({
  icon,
  onPress,
  accessibilityLabel,
  accessibilityHint,
  active,
  badgeColor,
}: {
  icon: React.ReactNode;
  onPress: () => void;
  accessibilityLabel: string;
  accessibilityHint?: string;
  active?: boolean;
  badgeColor?: string;
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
      {badgeColor ? <View style={[styles.badge, { backgroundColor: badgeColor, borderColor: tokens.bg1 }]} /> : null}
    </Pressable>
  );
}

type RailTint = "working" | "needs-you" | "done" | "failed" | "idle";

function railTint(row: Pick<SessionRow, "busy" | "waiting" | "last_turn_outcome">): RailTint {
  if (row.waiting) return "needs-you";
  if (row.busy) return "working";
  if (failedTurn(row)) return "failed";
  return row.last_turn_outcome === "success" ? "done" : "idle";
}

function SessionIconButton({
  title,
  row,
  selected,
  onPress,
  onPressIn,
  onHoverIn,
}: {
  title: string;
  row: SessionRow;
  selected: boolean;
  onPress: () => void;
  onPressIn?: () => void;
  onHoverIn?: () => void;
}) {
  const tokens = useTokens();
  const tint = railTint(row);
  const state: StatusDotState = row.waiting ? "waiting" : row.busy ? "busy" : "idle";
  const tintColor =
    tint === "needs-you" ? tokens.danger : tint === "working" ? tokens.accent : tint === "done" ? tokens.success : tint === "failed" ? tokens.danger : null;
  const glyphColor = tint === "idle" ? tokens.ink4 : selected ? tokens.ink : tokens.ink2;
  return (
    <Pressable
      onPress={onPress}
      onPressIn={onPressIn}
      onHoverIn={onHoverIn}
      accessibilityRole="button"
      accessibilityLabel={`${title}, ${sessionStateWord(row)}${row.worktree ? ", worktree" : ""}`}
      style={[
        styles.sessionButton,
        tintColor && { backgroundColor: hexToRgba(tintColor, tint === "done" ? 0.1 : 0.16) },
        selected && { backgroundColor: tokens.bg3, borderWidth: 1, borderColor: tokens.borderStrong },
      ]}
    >
      <ProviderGlyph provider={providerIdOf(row)} size={14} color={row.worktree ? tokens.accent : glyphColor} accessibilityLabel={null} />
      {state !== "idle" ? (
        <View style={[styles.badge, { backgroundColor: statusDotColor(state, tokens), borderColor: tokens.bg1 }]} />
      ) : tint === "failed" ? (
        <View style={[styles.badge, { backgroundColor: tokens.danger, borderColor: tokens.bg1 }]} />
      ) : null}
    </Pressable>
  );
}

export interface IconRailProps {
  onExpand: () => void;
  /** See Sidebar's identical prop — the ring column doubles as the ⌘U dock toggle. */
  onToggleDock?: () => void;
  dockOpen?: boolean;
}

export function IconRail({ onExpand, onToggleDock, dockOpen = false }: IconRailProps) {
  const tokens = useTokens();
  const usageHint = useShortcutLabel("shell.usage");
  const sidebarHint = useShortcutLabel("shell.sidebar");
  const newSessionHint = useShortcutLabel("session.new");
  const pathname = usePathname();
  const palette = usePalette();
  const { setActive } = useAuth();
  const prefetchHistory = usePrefetchSessionHistory();
  const { rows } = useScopedFleet();
  const { data: activeRows } = useSessions();
  const waitingCount = useMemo(() => (activeRows ?? []).filter((row) => row.waiting).length, [activeRows]);
  const noticesUnseen = totalUnseen(useNoticesStore().hosts);
  const activationOverride = useSessionActivationOverride();
  const selectedSessionId = activationOverride ?? pathname.match(/^\/session\/([^/]+)/)?.[1];
  const { rings } = useProviderPace();
  const nowSec = useClockSeconds();
  const workbench = useWorkbench();
  const browserSessionId = workbench.focusedSessionId ?? selectedSessionId;

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
      <RailIconButton
        icon={<Plus size={14} strokeWidth={2} color={tokens.ink2} />}
        onPress={() => router.push("/new-session")}
        accessibilityLabel="Forge a new session"
        accessibilityHint={newSessionHint}
      />
      {browserSessionId ? (
        <RailIconButton
          icon={<Globe size={14} strokeWidth={1.75} color={tokens.ink2} />}
          onPress={() => workbench.toggleSurface({ kind: "preview", sessionId: browserSessionId })}
          accessibilityLabel="Browser preview"
        />
      ) : null}
      <View style={[styles.divider, { backgroundColor: tokens.border }]} />

      <ScrollView style={styles.sessionList} contentContainerStyle={styles.sessionListContent} showsVerticalScrollIndicator={false}>
        {rows.map(({ row, serverId }) => {
          const title = row.title || `session ${row.id.slice(0, 8)}`;
          return (
            <SessionIconButton
              key={row.id}
              title={title}
              row={row}
              selected={row.id === selectedSessionId}
              onPress={() => {
                if (serverId) setActive(serverId);
                openSession(row.id);
              }}
              onPressIn={() => void prefetchHistory(row.id)}
              onHoverIn={() => void prefetchHistory(row.id).then(() => requestSessionPrewarm(row.id))}
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
              accessibilityHint={usageHint}
              style={styles.ringColumn}
            >
              {rings.map((ring) => (
                <UsageRing
                  key={ring.provider}
                  pct={ring.pct}
                  size={22}
                  provider={ring.provider}
                  tooltip={ringSummary(ring, nowSec)}
                  accessibilityLabel={ringSummary(ring, nowSec)}
                />
              ))}
            </Pressable>
          </>
        ) : null}
        <View style={[styles.divider, { backgroundColor: tokens.border }]} />
        {/* The sidebar footer's Inbox/Notices/History, so collapsing the sidebar does not hide them. */}
        <RailIconButton
          icon={<BellDot size={14} strokeWidth={1.75} color={tokens.ink2} />}
          onPress={() => router.push("/inbox")}
          accessibilityLabel={waitingCount > 0 ? `Inbox, ${waitingCount} needs you` : "Inbox"}
          active={pathname === "/inbox"}
          badgeColor={waitingCount > 0 ? tokens.danger : undefined}
        />
        <RailIconButton
          icon={<Bell size={14} strokeWidth={1.75} color={tokens.ink2} />}
          onPress={() => router.push("/notices")}
          accessibilityLabel={noticesUnseen > 0 ? `Notices, ${noticesUnseen} new` : "Notices"}
          active={pathname === "/notices"}
          badgeColor={noticesUnseen > 0 ? tokens.accent : undefined}
        />
        <RailIconButton
          icon={<History size={14} strokeWidth={1.75} color={tokens.ink2} />}
          onPress={() => router.push("/history")}
          accessibilityLabel="History"
          active={pathname === "/history"}
        />
        {onToggleDock ? (
          <RailIconButton
            icon={<Gauge size={14} strokeWidth={1.75} color={dockOpen ? tokens.ink : tokens.ink2} />}
            onPress={onToggleDock}
            accessibilityLabel={dockOpen ? "Hide usage panel" : "Show usage panel"}
            accessibilityHint={usageHint}
            active={dockOpen}
          />
        ) : null}
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
          accessibilityHint={sidebarHint}
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
