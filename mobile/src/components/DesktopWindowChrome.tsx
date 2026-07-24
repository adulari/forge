// Hearth desktop shell — the 36px window-chrome bar (D Main, docs/design/machined
// INVENTORY.md L29-36): flame + "Forge" leading, a centered mono title (the active
// server's name — the closest real-data equivalent of the mock's inline project/host
// label), a host-status chip + compact ⌘K chip trailing, native window controls
// (non-macOS; macOS keeps its overlay traffic lights, so only the drag region +
// content render there). The whole bar is a Tauri drag region except the
// interactive islands.
import { Flame, Minus, Search, Square, X } from "lucide-react-native";
import React, { useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View, type ViewProps } from "react-native";

import { useAuth } from "../lib/auth";
import { isMacOS, isTauri } from "../lib/platform";
import { usePalette } from "./overlay/CommandPalette";
import { useTokens } from "../theme/ThemeProvider";
import { radii, space } from "../theme/tokens";
import { monoFamily } from "../theme/typography";

export const DESKTOP_WINDOW_CHROME_HEIGHT = 36;

type WebViewProps = ViewProps & { dataSet?: Record<string, string>; onDoubleClick?: () => void };
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
  const { servers, activeServerId } = useAuth();
  const activeServer = servers.find((server) => server.id === activeServerId);
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
      dataSet={{ tauriDragRegion: "" }}
      onDoubleClick={() => void toggleMaximize()}
      style={[styles.bar, { backgroundColor: tokens.bg1, borderBottomColor: tokens.border }, isMacOS && styles.macos]}
      accessible={false}
    >
      <View style={styles.brandGroup} pointerEvents="none">
        <Flame size={13} color={tokens.accent} strokeWidth={1.75} />
        <Text style={[styles.brand, { color: tokens.ink }]}>Forge</Text>
      </View>

      {activeServer ? (
        <View style={styles.titleWrap} pointerEvents="none">
          <Text style={[styles.title, { color: tokens.ink3 }]} numberOfLines={1}>
            {activeServer.name}
          </Text>
        </View>
      ) : null}

      <View style={styles.spacer} />

      {activeServer ? (
        <View style={[styles.hostChip, { borderColor: tokens.border }]} pointerEvents="none">
          <View style={[styles.hostDot, { backgroundColor: tokens.success }]} />
          <Text style={[styles.hostChipText, { color: tokens.ink2 }]} numberOfLines={1}>
            {activeServer.name} · Direct
          </Text>
        </View>
      ) : null}

      <WebView dataSet={{ tauriDragRegion: "false" }}>
        <Pressable
          onPress={() => palette.open("default")}
          accessibilityRole="button"
          accessibilityLabel="Search or command"
          style={[styles.kbdChip, { borderColor: tokens.border }]}
        >
          <Search size={11} color={tokens.ink4} strokeWidth={2} />
          <Text style={[styles.kbd, { color: tokens.ink4 }]}>⌘K</Text>
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
