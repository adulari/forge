import { Minus, Square, X } from "lucide-react-native";
import React, { useEffect, useState } from "react";
import { Pressable, StyleSheet, View, type ViewProps } from "react-native";

import { isMacOS, isTauri } from "../lib/platform";
import { useTokens } from "../theme/ThemeProvider";
import { tapTarget } from "../theme/tokens";

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
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
    >
      {!isMacOS ? (
        <WebView style={styles.controls} dataSet={{ tauriDragRegion: "false" }}>
          <Pressable onPress={() => void windowControls?.minimize()} style={styles.control} accessibilityLabel="Minimize window">
            <Minus size={16} color={tokens.ink2} />
          </Pressable>
          <Pressable onPress={() => void toggleMaximize()} style={styles.control} accessibilityLabel="Maximize window">
            <Square size={14} color={tokens.ink2} />
          </Pressable>
          <Pressable onPress={() => void windowControls?.close()} style={styles.control} accessibilityLabel="Close window">
            <X size={16} color={tokens.ink2} />
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
    height: 32,
    borderBottomWidth: StyleSheet.hairlineWidth,
    zIndex: 1000,
    flexDirection: "row",
    justifyContent: "flex-end",
  },
  macos: { paddingLeft: 76 },
  controls: { flexDirection: "row" },
  control: { width: tapTarget, height: 32, alignItems: "center", justifyContent: "center" },
});
