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
import { Redirect, Stack, usePathname } from "expo-router";
import * as SplashScreen from "expo-splash-screen";
import React, { useEffect, useMemo, useState } from "react";
import { ActivityIndicator, StyleSheet, View } from "react-native";
import { GestureHandlerRootView } from "react-native-gesture-handler";
import { SafeAreaProvider } from "react-native-safe-area-context";

import { FleetWatcher } from "../components/fleet/FleetWatcher";
import { AppLock } from "../components/AppLock";
import { AnonymousTelemetry } from "../components/AnonymousTelemetry";
import { DesktopWindowChrome, DESKTOP_WINDOW_CHROME_HEIGHT } from "../components/DesktopWindowChrome";
import { ErrorBoundary } from "../components/ErrorBoundary";
import { Screen } from "../components/ds/Screen";
import { MasterDetail } from "../components/ds/MasterDetail";
import { ToastHost } from "../components/ds/ToastHost";
import { DockHost } from "../components/shell/DockHost";
import { IconRail } from "../components/shell/IconRail";
import { QuickComposer } from "../components/shell/QuickComposer";
import { Sidebar } from "../components/shell/Sidebar";
import { PaletteHost } from "../components/overlay/CommandPalette";
import { WebTopBar } from "../components/WebTopBar";
import { AnywhereProvider as RealAnywhereProvider } from "../lib/AnywhereProvider";
import { AnywhereProvider as LegacyAnywhereProvider } from "../lib/anywhere/store";
import { AuthProvider, useAuth } from "../lib/auth";
import { initHaptics } from "../lib/haptics";
import { isTauri, isWeb } from "../lib/platform";
import { checkForDesktopUpdate } from "../lib/updater";
import { useOtaUpdates } from "../lib/useOtaUpdates";
import {
  useGlobalShortcuts,
  useQuickComposerHotkey,
  useSidebarCollapseHotkey,
  useUsageDockHotkey,
} from "../lib/shortcuts";
import { ThemeProvider, useTokens } from "../theme/ThemeProvider";
import { monoFamily } from "../theme/typography";
import { useBreakpoint } from "../theme/useBreakpoint";

const SIDEBAR_COLLAPSED_KEY = "forge.sidebarCollapsed";
const SIDEBAR_WIDTH = 232;
const ICON_RAIL_WIDTH = 48;

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

// Hearth: settings-family routes bring their own 240px nav rail (SettingsShell), so the
// persistent Fleet rail collapses there — one rail on screen at a time. Connect is a
// full-bleed pairing screen on every surface.
const RAILLESS_ROUTES = /^\/(settings|configuration|skills|hooks|models|plans|mcp|usage|session-tree|gallery|connect|anywhere|shares)(\/|$)/;

// Reachable without a paired daemon: /shares/[id] is a public read-only replay link
// (no sign-in, no server), and /anywhere/* is the relay onboarding Connect itself
// deep-links into before any Direct server exists.
const UNPAIRED_ROUTES = /^\/(shares|anywhere)(\/|$)/;

function RootNavigator() {
  const { isLoading, isPaired } = useAuth();
  const tokens = useTokens();
  const { isExpanded } = useBreakpoint();
  const pathname = usePathname();
  const railless = RAILLESS_ROUTES.test(pathname);

  // Machined wave 2 shell chrome — sidebar collapse (persisted), the usage dock, and
  // the quick composer all live here (not in Sidebar/IconRail/DockHost themselves)
  // because MasterDetail needs `collapsed` to size its rail BEFORE it renders either
  // rail component (see ds/MasterDetail.tsx's `railWidth` prop). Registering the
  // hotkeys unconditionally is harmless — the chrome they toggle only ever renders
  // below, gated on `isPaired && isExpanded`.
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [dockOpen, setDockOpen] = useState(false);
  const [quickComposerOpen, setQuickComposerOpen] = useState(false);

  useEffect(() => {
    void AsyncStorage.getItem(SIDEBAR_COLLAPSED_KEY).then((raw) => {
      if (raw === "true") setSidebarCollapsed(true);
    });
  }, []);

  const toggleSidebarCollapsed = () => {
    setSidebarCollapsed((collapsed) => {
      const next = !collapsed;
      void AsyncStorage.setItem(SIDEBAR_COLLAPSED_KEY, String(next)).catch(() => undefined);
      return next;
    });
  };
  useSidebarCollapseHotkey(toggleSidebarCollapsed);
  useUsageDockHotkey(() => setDockOpen((open) => !open));
  useQuickComposerHotkey(() => setQuickComposerOpen(true));

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

  const appStack = (
    <Stack
        screenOptions={{
          headerShown: false,
          contentStyle: { backgroundColor: tokens.bg1 },
        }}
      >
        <Stack.Screen name="connect" />
        <Stack.Screen name="(tabs)" />
        <Stack.Screen name="configuration" />
        <Stack.Screen name="skills" />
        <Stack.Screen name="hooks" />
        <Stack.Screen name="models" />
        <Stack.Screen name="session-tree" />

        <Stack.Screen name="mcp" />
        <Stack.Screen name="session/[id]" />
        <Stack.Screen
          name="new-session"
          // headerShown: false — new-session.tsx owns its own themed header (matches
          // every other screen instead of expo-router's default unthemed white bar).
          options={{ headerShown: false, presentation: "modal" }}
        />
    </Stack>
  );

  const rail = railless ? null : sidebarCollapsed ? (
    <IconRail onExpand={toggleSidebarCollapsed} onToggleDock={() => setDockOpen((open) => !open)} />
  ) : (
    <Sidebar onCollapse={toggleSidebarCollapsed} onToggleDock={() => setDockOpen((open) => !open)} />
  );

  return (
    <>
      {isPaired && isExpanded ? (
        <>
          {isWeb && !isTauri ? <WebTopBar onToggleDock={() => setDockOpen((open) => !open)} dockOpen={dockOpen} /> : null}
          <View style={styles.shellRow}>
            <MasterDetail master={rail} detail={appStack} railWidth={sidebarCollapsed ? ICON_RAIL_WIDTH : SIDEBAR_WIDTH} />
            <DockHost open={dockOpen && !railless} onClose={() => setDockOpen(false)} />
          </View>
          <QuickComposer visible={quickComposerOpen} onClose={() => setQuickComposerOpen(false)} />
        </>
      ) : (
        appStack
      )}
      {/* Declarative redirect (rather than Stack.Protected) per T2.1 spec: whatever route
          expo-router resolved on cold start/deep-link, bounce to /connect once we know
          there's no active server. */}
      {!isPaired && !UNPAIRED_ROUTES.test(pathname) ? <Redirect href="/connect" /> : null}
    </>
  );
}

const styles = StyleSheet.create({
  shellRow: { flex: 1, flexDirection: "row" },
});

export default function RootLayout() {
  const persistOptions = useMemo(() => ({ persister: asyncStoragePersister }), []);
  useGlobalShortcuts(); // HANDOFF(T5.1): ⌘1..4 tabs / ⌘N new session — web/desktop only, no-op native
  useOtaUpdates(); // EAS Update OTA check on launch + foreground (no-op in dev / when disabled)

  useEffect(() => {
    void initHaptics();
    if (isTauri) void checkForDesktopUpdate().catch(() => undefined);
  }, []);

  // Native gets Geist + Geist Mono from the expo-font config plugin's build-time embed;
  // that plugin has no effect on the web export, so web needs this runtime load too
  // (it registers a @font-face under the same family names — resolves near-instantly
  // since the ttfs are bundled, and is a no-op check on native where they're already
  // embedded). Machined makes sans a bundled family on every platform too (not just
  // mono), so both families load here now.
  const [monoFontsLoaded, monoFontsError] = useFonts({
    "Geist-Regular": require("../../assets/Geist-Regular.ttf"),
    "Geist-Medium": require("../../assets/Geist-Medium.ttf"),
    "Geist-SemiBold": require("../../assets/Geist-SemiBold.ttf"),
    "Geist-Bold": require("../../assets/Geist-Bold.ttf"),
    [monoFamily.regular]: require("../../assets/GeistMono-Regular.ttf"),
    [monoFamily.medium]: require("../../assets/GeistMono-Medium.ttf"),
    [monoFamily.bold]: require("../../assets/GeistMono-SemiBold.ttf"),
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
              <RealAnywhereProvider>
                <LegacyAnywhereProvider>
                  <PersistQueryClientProvider client={queryClient} persistOptions={persistOptions}>
                  <ToastHost>
                    <AnonymousTelemetry />
                    <FleetWatcher />
                    {/* T4.2: global <CommandPalette /> host — ⌘K/Ctrl+K on web/desktop, a
                        `usePalette().open()` affordance (e.g. a header IconButton) on native. */}
                    <View style={{ flex: 1, paddingTop: isTauri ? DESKTOP_WINDOW_CHROME_HEIGHT : 0 }}>
                      <PaletteHost>
                        <AppLock>
                          <RootNavigator />
                        </AppLock>
                        {/* Inside PaletteHost: the Hearth chrome bar's ⌘K field calls usePalette(). */}
                        <DesktopWindowChrome />
                      </PaletteHost>
                    </View>
                  </ToastHost>
                  </PersistQueryClientProvider>
                </LegacyAnywhereProvider>
              </RealAnywhereProvider>
            </AuthProvider>
          </ThemeProvider>
        </ErrorBoundary>
      </SafeAreaProvider>
    </GestureHandlerRootView>
  );
}
