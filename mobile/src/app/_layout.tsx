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
import { UpdateNotice } from "../components/UpdateNotice";
import { Screen } from "../components/ds/Screen";
import { MasterDetail } from "../components/ds/MasterDetail";
import { ToastHost } from "../components/ds/ToastHost";
import { useActiveSessionId } from "../components/shell/activeSession";
import { DockHost } from "../components/shell/DockHost";
import { IconRail } from "../components/shell/IconRail";
import { QuickComposer } from "../components/shell/QuickComposer";
import { Sidebar } from "../components/shell/Sidebar";
import { SplitPanes, useSplitPanes } from "../components/shell/SplitPanes";
import { WorkbenchProvider, useWorkbench } from "../components/workbench/WorkbenchProvider";
import { activeWorkbenchSurface } from "../components/workbench/model";
import { PaletteHost } from "../components/overlay/CommandPalette";
import { WebTopBar } from "../components/WebTopBar";
import { AnywhereProvider as RealAnywhereProvider } from "../lib/AnywhereProvider";
import { AnywhereProvider as LegacyAnywhereProvider } from "../lib/anywhere/store";
import { AuthProvider, useAuth } from "../lib/auth";
import { initHaptics } from "../lib/haptics";
import { isTauri, isWeb } from "../lib/platform";
import { checkForDesktopUpdate } from "../lib/updater";
import { useOtaUpdates } from "../lib/useOtaUpdates";
import { useDesktopMenuAction } from "../lib/desktopMenu";
import {
  useAppShortcut,
  useGlobalShortcuts,
  useQuickComposerHotkey,
  useSidebarCollapseHotkey,
  useUsageDockHotkey,
} from "../lib/shortcuts";
import { useTabBadgeTitle } from "../lib/tabBadge";
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
const RAILLESS_ROUTES = /^\/(settings|appearance|keybindings|configuration|skills|hooks|providers|models|plans|mcp|usage|session-tree|gallery|connect|anywhere|shares)(\/|$)/;

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

  // Browser tab badge — mounted here (inside the providers, since it reads the fleet) so the
  // waiting count keeps tracking after the user navigates away from Settings. No-op off web.
  useTabBadgeTitle();

  // Machined wave 2 shell chrome — sidebar collapse (persisted), the usage dock, and
  // the quick composer all live here (not in Sidebar/IconRail/DockHost themselves)
  // because MasterDetail needs `collapsed` to size its rail BEFORE it renders either
  // rail component (see ds/MasterDetail.tsx's `railWidth` prop). Registering the
  // hotkeys unconditionally is harmless — the chrome they toggle only ever renders
  // below, gated on `isPaired && isExpanded`.
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [quickComposerOpen, setQuickComposerOpen] = useState(false);
  const workbench = useWorkbench();
  const rightSurface = activeWorkbenchSurface(workbench.state, "right");
  const bottomSurface = activeWorkbenchSurface(workbench.state, "bottom");
  const dockOpen = rightSurface != null;

  const activeSessionId = useActiveSessionId();
  // Desktop-only: on compact/medium the split reports inactive and nothing extra renders.
  const split = useSplitPanes(isPaired && isExpanded && !railless);

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
  const toggleUsageDock = () => workbench.toggleSurface({ kind: "usage" });
  useSidebarCollapseHotkey(toggleSidebarCollapsed);
  useUsageDockHotkey(toggleUsageDock);
  useQuickComposerHotkey(() => setQuickComposerOpen(true));

  // ⌘D split · ⌘J terminal · ⌘G git review. Each claims the matching Tauri menu item as well
  // (`useShellHotkeys.ts` explains why: a macOS menu key equivalent never reaches the webview).
  // Guarded on `isExpanded` so the accelerators are inert on a compact window, where none of
  // this chrome renders.
  const toggleSplit = () => {
    if (isExpanded) split.toggle();
  };
  const toggleTerminal = () => {
    if (isExpanded) workbench.toggleSurface({ kind: "terminal" });
  };
  const openGitReview = () => {
    if (isExpanded) workbench.toggleSurface({ kind: "git" });
  };
  const toggleBrowserPreview = () => {
    if (isExpanded && activeSessionId) {
      workbench.toggleSurface({ kind: "preview", sessionId: activeSessionId });
    }
  };
  useAppShortcut("workbench.split", toggleSplit);
  useDesktopMenuAction("view:split-pane", toggleSplit);
  useAppShortcut("workbench.terminal", toggleTerminal);
  useDesktopMenuAction("view:terminal", toggleTerminal);
  useAppShortcut("workbench.gitReview", openGitReview);
  useDesktopMenuAction("view:git-review", openGitReview);
  useDesktopMenuAction("view:browser-preview", toggleBrowserPreview);

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
        <Stack.Screen name="providers" />
        <Stack.Screen name="appearance" />
        <Stack.Screen name="keybindings" />
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
    <IconRail onExpand={toggleSidebarCollapsed} onToggleDock={toggleUsageDock} />
  ) : (
    <Sidebar onCollapse={toggleSidebarCollapsed} onToggleDock={toggleUsageDock} />
  );

  // The split wraps the live route stack rather than replacing it (see SplitPanes.tsx), and the
  // terminal dock sits under the content column — beside the rail, per D Rail + Terminal, not
  // spanning the whole window.
  const detail = (
    <View style={styles.detailColumn}>
      {split.active && split.panes[0] ? (
        <SplitPanes
          primaryId={split.panes[0]}
          secondaryId={split.panes[1]}
          primary={appStack}
          onSwap={split.swap}
          onClosePane={split.closePane}
        />
      ) : (
        appStack
      )}
      <DockHost
        open={bottomSurface != null && !railless}
        dock="terminal"
        surface={bottomSurface}
        tabs={workbench.state.bottom.tabs}
        sessionId={activeSessionId}
        onActivateSurface={(id) => workbench.activateSurface("bottom", id)}
        onClose={() => workbench.hidePlacement("bottom")}
      />
    </View>
  );

  return (
    <>
      {isPaired && isExpanded ? (
        <>
          {isWeb && !isTauri ? (
            <WebTopBar
              onToggleDock={toggleUsageDock}
              dockOpen={rightSurface?.kind === "usage"}
            />
          ) : null}
          <View style={styles.shellRow}>
            <MasterDetail master={rail} detail={detail} railWidth={sidebarCollapsed ? ICON_RAIL_WIDTH : SIDEBAR_WIDTH} />
            <DockHost
              open={dockOpen && !railless}
              surface={rightSurface}
              tabs={workbench.state.right.tabs}
              sessionId={activeSessionId}
              onActivateSurface={(id) => workbench.activateSurface("right", id)}
              onClose={() => workbench.hidePlacement("right")}
            />
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
  detailColumn: { flex: 1, minHeight: 0 },
});

export default function RootLayout() {
  const persistOptions = useMemo(() => ({ persister: asyncStoragePersister }), []);
  useGlobalShortcuts(); // Persisted desktop/web app bindings; hardware-keyboard no-op on native.
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
                    {/* Inside the query provider because it reads the daemon's changelog, and above
                        the navigator so it is not tied to whichever tab happens to be open. */}
                    <UpdateNotice />
                    {/* T4.2: global <CommandPalette /> host — ⌘K/Ctrl+K on web/desktop, a
                        `usePalette().open()` affordance (e.g. a header IconButton) on native. */}
                    <View style={{ flex: 1, paddingTop: isTauri ? DESKTOP_WINDOW_CHROME_HEIGHT : 0 }}>
                      <PaletteHost>
                        <WorkbenchProvider>
                          <AppLock>
                            <RootNavigator />
                          </AppLock>
                          {/* Inside PaletteHost: the Hearth chrome bar's ⌘K field calls usePalette(). */}
                          <DesktopWindowChrome />
                        </WorkbenchProvider>
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
