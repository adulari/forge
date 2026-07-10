// Root providers (T2.1): ThemeProvider (persisted light/dark/system) -> AuthProvider
// (multi-server pairing) -> PersistQueryClientProvider (warm-start react-query cache) ->
// ToastHost (global toast surface) -> AppLock (biometric gate) -> the top-level Stack.
//
// GestureHandlerRootView + SafeAreaProvider are hoisted to the very outside despite being
// listed last in the task spec: react-native-gesture-handler requires its root view to wrap
// the entire tree, and SafeAreaProvider must wrap every safe-area consumer below it (ToastHost,
// Screen, AppLock's lock view all use SafeAreaView/useSafeAreaInsets).
import AsyncStorage from "@react-native-async-storage/async-storage";
import { createAsyncStoragePersister } from "@tanstack/query-async-storage-persister";
import { QueryClient } from "@tanstack/react-query";
import { PersistQueryClientProvider } from "@tanstack/react-query-persist-client";
import { useFonts } from "expo-font";
import { Redirect, Stack } from "expo-router";
import * as SplashScreen from "expo-splash-screen";
import React, { useEffect, useMemo } from "react";
import { ActivityIndicator, StyleSheet, View, type ViewProps } from "react-native";
import { GestureHandlerRootView } from "react-native-gesture-handler";
import { SafeAreaProvider } from "react-native-safe-area-context";

import { AppLock } from "../components/AppLock";
import { ErrorBoundary } from "../components/ErrorBoundary";
import { Screen } from "../components/ds/Screen";
import { ToastHost } from "../components/ds/ToastHost";
import { PaletteHost } from "../components/overlay/CommandPalette";
import { AuthProvider, useAuth } from "../lib/auth";
import { isMacOS, isTauri } from "../lib/platform";
import { useGlobalShortcuts } from "../lib/shortcuts";
import { ThemeProvider, useTokens } from "../theme/ThemeProvider";
import { monoFamily } from "../theme/typography";

// macOS Tauri window uses `titleBarStyle: "Overlay"` (src-tauri/tauri.conf.json) — the
// webview draws under the traffic-light buttons with no OS-drawn title bar left to drag
// by, so without an explicit drag region the window is stuck wherever it opens. A
// `data-tauri-drag-region` element makes Tauri treat mousedown on it as a window-drag
// (react-native-web's `dataSet` prop -> `data-*` attributes: `dataSet={{ tauriDragRegion:
// "" }}` -> `data-tauri-drag-region=""`). RN's ViewProps type doesn't know about `dataSet`
// (it's a react-native-web-only extension), hence the narrow cast below instead of `any`.
type WebViewProps = ViewProps & { dataSet?: Record<string, string> };
const WebView = View as unknown as React.ComponentType<WebViewProps>;

const DRAG_REGION_HEIGHT = 28;

function DesktopDragRegion() {
  if (!isTauri || !isMacOS) return null;
  return (
    <WebView
      dataSet={{ tauriDragRegion: "" }}
      style={styles.dragRegion}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
    />
  );
}

// Keep the native splash up until pairing state resolves (avoids a flash of the
// "unpaired" redirect before AuthProvider finishes its one AsyncStorage/secure-store read).
SplashScreen.preventAutoHideAsync().catch(() => {
  // best-effort — a failure here just means the splash behaves like default autohide
});

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 30_000,
      gcTime: 24 * 60 * 60 * 1000,
      retry: 1,
    },
  },
});

const asyncStoragePersister = createAsyncStoragePersister({
  storage: AsyncStorage,
  key: "forge.queryCache",
});

function RootNavigator() {
  const { isLoading, isPaired } = useAuth();
  const tokens = useTokens();

  useEffect(() => {
    if (!isLoading) {
      SplashScreen.hideAsync().catch(() => {
        // best-effort — nothing sensible to do if the splash is already gone
      });
    }
  }, [isLoading]);

  if (isLoading) {
    return (
      <Screen>
        <ActivityIndicator color={tokens.ink3} />
      </Screen>
    );
  }

  return (
    <>
      <Stack
        screenOptions={{
          headerShown: false,
          contentStyle: { backgroundColor: tokens.bg1 },
        }}
      >
        <Stack.Screen name="connect" />
        <Stack.Screen name="(tabs)" />
        <Stack.Screen name="session/[id]" />
        <Stack.Screen
          name="new-session"
          // headerShown: false — new-session.tsx owns its own themed header (matches
          // every other screen instead of expo-router's default unthemed white bar).
          options={{ headerShown: false, presentation: "modal" }}
        />
      </Stack>
      {/* Declarative redirect (rather than Stack.Protected) per T2.1 spec: whatever route
          expo-router resolved on cold start/deep-link, bounce to /connect once we know
          there's no active server. */}
      {!isPaired ? <Redirect href="/connect" /> : null}
    </>
  );
}

export default function RootLayout() {
  const persistOptions = useMemo(() => ({ persister: asyncStoragePersister }), []);
  useGlobalShortcuts(); // HANDOFF(T5.1): ⌘1..4 tabs / ⌘N new session — web/desktop only, no-op native

  // Native gets JetBrains Mono from the expo-font config plugin's build-time embed;
  // that plugin has no effect on the web export, so web needs this runtime load too
  // (it registers a @font-face under the same family names — resolves near-instantly
  // since the ttf is bundled, and is a no-op check on native where it's already embedded).
  const [monoFontsLoaded, monoFontsError] = useFonts({
    [monoFamily.regular]: require("../../assets/JetBrainsMono-Regular.ttf"),
    [monoFamily.bold]: require("../../assets/JetBrainsMono-Bold.ttf"),
  });

  // Only block on the still-loading case — on error (e.g. the web runtime load failing)
  // proceed anyway so the app boots with system-font fallback instead of hanging forever
  // (AuthProvider never mounts, splash never hides).
  if (!monoFontsLoaded && !monoFontsError) return null;

  return (
    <GestureHandlerRootView style={{ flex: 1 }}>
      <SafeAreaProvider>
        <ErrorBoundary>
          <ThemeProvider>
            <AuthProvider>
              <PersistQueryClientProvider client={queryClient} persistOptions={persistOptions}>
                <ToastHost>
                  {/* T4.2: global <CommandPalette /> host — ⌘K/Ctrl+K on web/desktop, a
                      `usePalette().open()` affordance (e.g. a header IconButton) on native. */}
                  <PaletteHost>
                    <AppLock>
                      <RootNavigator />
                    </AppLock>
                  </PaletteHost>
                </ToastHost>
                <DesktopDragRegion />
              </PersistQueryClientProvider>
            </AuthProvider>
          </ThemeProvider>
        </ErrorBoundary>
      </SafeAreaProvider>
    </GestureHandlerRootView>
  );
}

const styles = StyleSheet.create({
  // Thin, non-interactive-except-for-dragging strip pinned to the very top of the
  // window. Matches the macOS overlay title bar's height so it sits above normal
  // screen content (headers/tab bars start below it) instead of eating into it.
  dragRegion: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    height: DRAG_REGION_HEIGHT,
    zIndex: 1000,
  },
});
