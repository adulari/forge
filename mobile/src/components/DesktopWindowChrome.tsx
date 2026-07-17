// Hearth desktop shell — the 36px window-chrome bar (desktop.dc.html): flame + "Forge"
// leading, a centered ⌘K search-or-command field, native window controls trailing
// (non-macOS; macOS keeps its overlay traffic lights, so only the drag region + content
// render there). The whole bar is a Tauri drag region except the interactive islands.
import { Flame, Minus, Search, Square, X } from "lucide-react-native";
import React, { useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View, type ViewProps } from "react-native";

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
        <Flame size={14} color={tokens.accent} strokeWidth={1.75} />
        <Text style={[styles.brand, { color: tokens.ink2 }]}>Forge</Text>
      </View>
      <View style={styles.spacer} />
      <WebView dataSet={{ tauriDragRegion: "false" }}>
        <Pressable
          onPress={() => palette.open()}
          accessibilityRole="button"
          accessibilityLabel="Search or command"
          style={[styles.search, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}
        >
          <Search size={12} color={tokens.ink4} strokeWidth={2} />
          <Text style={[styles.searchHint, { color: tokens.ink4 }]}>search or command</Text>
          <Text style={[styles.kbd, { color: tokens.ink4, borderColor: tokens.border }]}>⌘K</Text>
        </Pressable>
      </WebView>
      <View style={styles.spacer} />
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
      ) : (
        <View style={styles.macosBalance} />
      )}
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
  brandGroup: { flexDirection: "row", alignItems: "center", gap: space.space8, width: 166 },
  brand: { fontSize: 12, fontWeight: "700", letterSpacing: -0.2 },
  spacer: { flex: 1 },
  search: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    height: 26,
    paddingHorizontal: space.space12,
    borderRadius: radii.radius8,
    borderWidth: 1,
    width: 340,
  },
  searchHint: { flex: 1, fontSize: 11 },
  kbd: {
    fontFamily: monoFamily.regular,
    fontSize: 10,
    borderWidth: 1,
    borderRadius: radii.radius4,
    paddingHorizontal: 5,
    paddingVertical: 1,
  },
  controls: { flexDirection: "row" },
  control: { width: 40, height: DESKTOP_WINDOW_CHROME_HEIGHT, alignItems: "center", justifyContent: "center" },
  macosBalance: { width: 166 },
});
