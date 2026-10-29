// Hearth desktop shell — the 36px window-chrome bar (D Main, docs/design/machined
// INVENTORY.md L29-36): flame + "Forge" leading, a centered mono title (the active
// server's name — the closest real-data equivalent of the mock's inline project/host
// label), a host-status chip + compact ⌘K chip trailing, native window controls
// (non-macOS; macOS keeps its overlay traffic lights, so only the drag region +
// content render there). The whole bar is a Tauri drag region except the
// interactive islands. "deep" makes clicks on child views count too; the bare attribute only
// matches clicks on the bar element itself, which the flex spacer covers. Double-click to
// maximize comes from Tauri's drag script, so the bar has no onDoubleClick of its own.
import { useQueries } from "@tanstack/react-query";
import { router, usePathname } from "expo-router";
import { Check, ChevronDown, Flame, Minus, Search, Square, X } from "lucide-react-native";
import React, { useEffect, useRef, useState } from "react";
import { Modal, Pressable, StyleSheet, Text, View, useWindowDimensions, type ViewProps } from "react-native";

import { getSessions } from "../lib/api";
import { useAuth } from "../lib/auth";
import { LayoutControl } from "./shell/LayoutMenu";
import { RAILLESS_ROUTES } from "./shell/railless";
import { isMacOS, isTauri } from "../lib/platform";
import { transportForBaseUrl } from "../lib/serverTargets";
import { useFleetScope } from "../lib/useScopedFleet";
import { fleetStatuses, type ServerReachability } from "./fleet/ServerScopePills";
import { useShortcutLabel } from "../lib/shortcuts";
import { usePalette } from "./overlay/CommandPalette";
import { useTokens } from "../theme/ThemeProvider";
import { radii, space } from "../theme/tokens";
import { monoFamily } from "../theme/typography";
import { useOverlayEscape } from "../lib/useOverlayEscape";

export const DESKTOP_WINDOW_CHROME_HEIGHT = 36;
const CENTERED_TITLE_MIN_WIDTH = 900;

type WebViewProps = ViewProps & { dataSet?: Record<string, string> };
const WebView = View as unknown as React.ComponentType<WebViewProps>;

interface WindowControls {
  minimize: () => Promise<void>;
  close: () => Promise<void>;
  isMaximized: () => Promise<boolean>;
  maximize: () => Promise<void>;
  unmaximize: () => Promise<void>;
}

export function DesktopWindowChrome() {
  const tokens = useTokens();
  const palette = usePalette();
  const { servers, activeServerId, showAll, multiServer, selectAll, selectServer } = useFleetScope();
  const activeServer = servers.find((server) => server.id === activeServerId);
  // Observe the per-server fleet polls the Fleet/sidebar run (same query keys) without adding
  // polling of its own: the bar sits outside every screen, so it has no focus state to gate on.
  const statuses = fleetStatuses(
    servers,
    useQueries({
      queries: servers.map((server) => ({
        queryKey: ["sessions", server.baseUrl] as const,
        queryFn: () => getSessions(server.baseUrl),
        enabled: false,
      })),
    }),
  );
  const activeStatus = statuses.find((status) => status.id === activeServerId);
  const dotColor = (reachability: ServerReachability | undefined) =>
    reachability === "unreachable" ? tokens.danger : reachability === "reachable" ? tokens.success : tokens.warn;
  const { width: windowWidth } = useWindowDimensions();
  const chipRef = useRef<View>(null);
  const [menuAnchor, setMenuAnchor] = useState<{ top: number; right: number } | null>(null);
  const openMenu = () =>
    chipRef.current?.measureInWindow((x, y, width, height) =>
      setMenuAnchor({ top: y + height + 4, right: Math.max(8, windowWidth - x - width) }),
    );
  const closeMenu = () => setMenuAnchor(null);
  useOverlayEscape(menuAnchor != null, closeMenu);
  const choose = (pick: () => void) => {
    pick();
    closeMenu();
  };
  const paletteHint = useShortcutLabel("command.palette");
  // The workbench (panes, docks) only exists in the paired, wide shell, and not on the
  // settings-family routes that bring their own rail.
  const { isPaired } = useAuth();
  const pathname = usePathname();
  const showLayout = isPaired && windowWidth >= 1024 && !RAILLESS_ROUTES.test(pathname);
  const [windowControls, setWindowControls] = useState<WindowControls | null>(null);

  useEffect(() => {
    if (!isTauri || isMacOS) return;
    let active = true;
    void import("@tauri-apps/api/window").then(({ getCurrentWindow }) => {
      if (active) setWindowControls(getCurrentWindow());
    });
    return () => {
      active = false;
    };
  }, []);

  if (!isTauri) return null;
  const toggleMaximize = async () => {
    if (!windowControls) return;
    if (await windowControls.isMaximized()) await windowControls.unmaximize();
    else await windowControls.maximize();
  };

  return (
    <WebView
      dataSet={{ tauriDragRegion: "deep" }}
      style={[styles.bar, { backgroundColor: tokens.bg1, borderBottomColor: tokens.border }, isMacOS && styles.macos]}
      accessible={false}
    >
      <View style={styles.brandGroup} pointerEvents="none">
        <Flame size={13} color={tokens.accent} strokeWidth={1.75} />
        <Text style={[styles.brand, { color: tokens.ink }]}>Forge</Text>
      </View>

      {/* Centered over the whole bar, so on a narrow window it ran into the server chip. */}
      {activeServer && windowWidth >= CENTERED_TITLE_MIN_WIDTH ? (
        <View style={styles.titleWrap} pointerEvents="none">
          <Text style={[styles.title, { color: tokens.ink3 }]} numberOfLines={1}>
            {showAll ? "All servers" : activeServer.name}
          </Text>
        </View>
      ) : null}

      <View style={styles.spacer} />

      {activeServer ? (
        <WebView dataSet={{ tauriDragRegion: "false" }}>
          <Pressable
            ref={chipRef}
            onPress={openMenu}
            accessibilityRole="button"
            accessibilityLabel={showAll ? "Showing all servers. Change server" : `Server ${activeServer.name}. Change server`}
            style={({ pressed, hovered }: { pressed: boolean; hovered?: boolean }) => [
              styles.hostChip,
              { borderColor: hovered || pressed || menuAnchor ? tokens.borderStrong : tokens.border },
            ]}
          >
            <View style={[styles.hostDot, { backgroundColor: showAll ? tokens.ink3 : dotColor(activeStatus?.reachability) }]} />
            <Text style={[styles.hostChipText, { color: tokens.ink2 }]} numberOfLines={1}>
              {showAll ? `All servers · ${servers.length}` : `${activeServer.name} · ${transportLabel(activeServer.baseUrl)}`}
            </Text>
            <ChevronDown size={11} color={tokens.ink4} strokeWidth={2} />
          </Pressable>
        </WebView>
      ) : null}

      <Modal visible={menuAnchor != null} transparent animationType="none" onRequestClose={closeMenu}>
        <Pressable style={StyleSheet.absoluteFill} onPress={closeMenu} accessibilityLabel="Close server menu" />
        {menuAnchor ? (
          <View
            style={[styles.menu, { top: menuAnchor.top, right: menuAnchor.right, backgroundColor: tokens.bg2, borderColor: tokens.border }]}
            accessibilityRole="menu"
          >
            {multiServer ? (
              <MenuItem
                label="All servers"
                meta={`${servers.length} servers`}
                selected={showAll}
                onPress={() => choose(selectAll)}
              />
            ) : null}
            {servers.map((server) => {
              const status = statuses.find((item) => item.id === server.id);
              return (
                <MenuItem
                  key={server.id}
                  label={server.name}
                  meta={status && status.waiting > 0 ? `${transportLabel(server.baseUrl)} · ${status.waiting} waiting` : transportLabel(server.baseUrl)}
                  dotColor={dotColor(status?.reachability)}
                  selected={!showAll && server.id === activeServerId}
                  onPress={() => choose(() => selectServer(server.id))}
                />
              );
            })}
            <View style={[styles.menuDivider, { backgroundColor: tokens.border }]} />
            <MenuItem label="Manage servers…" onPress={() => choose(() => router.push("/settings"))} />
          </View>
        ) : null}
      </Modal>

      {showLayout ? (
        <WebView dataSet={{ tauriDragRegion: "false" }}>
          <LayoutControl />
        </WebView>
      ) : null}

      <WebView dataSet={{ tauriDragRegion: "false" }}>
        <Pressable
          onPress={() => palette.open("default")}
          accessibilityRole="button"
          accessibilityLabel="Search or command"
          style={[styles.kbdChip, { borderColor: tokens.border }]}
        >
          <Search size={11} color={tokens.ink4} strokeWidth={2} />
          <Text style={[styles.kbd, { color: tokens.ink4 }]}>{paletteHint}</Text>
        </Pressable>
      </WebView>

      {!isMacOS ? (
        <WebView style={styles.controls} dataSet={{ tauriDragRegion: "false" }}>
          <Pressable onPress={() => void windowControls?.minimize()} style={styles.control} accessibilityRole="button" accessibilityLabel="Minimize window">
            <Minus size={15} color={tokens.ink3} />
          </Pressable>
          <Pressable onPress={() => void toggleMaximize()} style={styles.control} accessibilityRole="button" accessibilityLabel="Maximize or restore window">
            <Square size={12} color={tokens.ink3} />
          </Pressable>
          <Pressable onPress={() => void windowControls?.close()} style={styles.control} accessibilityRole="button" accessibilityLabel="Close window">
            <X size={15} color={tokens.ink3} />
          </Pressable>
        </WebView>
      ) : null}
    </WebView>
  );
}

function transportLabel(baseUrl: string): string {
  return transportForBaseUrl(baseUrl) === "anywhere" ? "Anywhere" : "Direct";
}

function MenuItem({
  label,
  meta,
  dotColor,
  selected = false,
  onPress,
}: {
  label: string;
  meta?: string;
  dotColor?: string;
  selected?: boolean;
  onPress: () => void;
}) {
  const tokens = useTokens();
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="menuitem"
      accessibilityState={{ selected }}
      style={({ pressed, hovered, focused }: { pressed: boolean; hovered?: boolean; focused?: boolean }) => [
        styles.menuItem,
        (hovered || focused || pressed) && { backgroundColor: tokens.bg3 },
      ]}
    >
      <View style={[styles.hostDot, { backgroundColor: dotColor ?? "transparent" }]} />
      <Text style={[styles.menuLabel, { color: selected ? tokens.accent : tokens.ink }]} numberOfLines={1}>
        {label}
      </Text>
      {meta ? <Text style={[styles.menuMeta, { color: tokens.ink4 }]}>{meta}</Text> : null}
      <View style={styles.menuCheck}>{selected ? <Check size={12} color={tokens.accent} strokeWidth={2} /> : null}</View>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  bar: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    height: DESKTOP_WINDOW_CHROME_HEIGHT,
    borderBottomWidth: StyleSheet.hairlineWidth,
    zIndex: 1000,
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: 14,
    gap: space.space8,
  },
  macos: { paddingLeft: 76 },
  brandGroup: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  brand: { fontSize: 12.5, fontWeight: "600", letterSpacing: -0.2 },
  // Centered title mono — absolutely positioned so it stays visually centered in the
  // bar regardless of the (asymmetric) brand/traffic-light padding on either side.
  titleWrap: { position: "absolute", top: 0, left: 0, right: 0, bottom: 0, alignItems: "center", justifyContent: "center" },
  title: { fontFamily: monoFamily.regular, fontSize: 10.5 },
  spacer: { flex: 1 },
  hostChip: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    borderWidth: 1,
    borderRadius: radii.radius4,
    paddingHorizontal: 7,
    paddingVertical: 2,
  },
  hostDot: { width: 5, height: 5, borderRadius: 2.5 },
  hostChipText: { fontFamily: monoFamily.regular, fontSize: 10.5 },
  menu: {
    position: "absolute",
    minWidth: 240,
    maxWidth: 360,
    borderWidth: 1,
    borderRadius: radii.radius8,
    paddingVertical: space.space4,
    shadowColor: "#000",
    shadowOpacity: 0.35,
    shadowRadius: 16,
    shadowOffset: { width: 0, height: 6 },
  },
  menuItem: { flexDirection: "row", alignItems: "center", gap: space.space8, height: 30, paddingHorizontal: space.space12 },
  menuLabel: { flex: 1, fontSize: 12.5 },
  menuMeta: { fontFamily: monoFamily.regular, fontSize: 10.5 },
  menuCheck: { width: 12 },
  menuDivider: { height: StyleSheet.hairlineWidth, marginVertical: space.space4 },
  kbdChip: {
    flexDirection: "row",
    alignItems: "center",
    gap: 5,
    borderWidth: 1,
    borderRadius: radii.radius4,
    paddingHorizontal: 6,
    paddingVertical: 2,
  },
  kbd: { fontFamily: monoFamily.regular, fontSize: 10 },
  controls: { flexDirection: "row" },
  control: { width: 40, height: DESKTOP_WINDOW_CHROME_HEIGHT, alignItems: "center", justifyContent: "center" },
});
