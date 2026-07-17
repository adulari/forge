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
import React, { useEffect, useMemo } from "react";
import { ActivityIndicator, View } from "react-native";
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
import { ExpandedFleetRail } from "../components/fleet/DesktopDrillDown";
import { PaletteHost } from "../components/overlay/CommandPalette";
import { WebTopBar } from "../components/WebTopBar";
import { AuthProvider, useAuth } from "../lib/auth";
import { initHaptics } from "../lib/haptics";
import { isTauri, isWeb } from "../lib/platform";
import { checkForDesktopUpdate } from "../lib/updater";
import { useOtaUpdates } from "../lib/useOtaUpdates";
import { useGlobalShortcuts } from "../lib/shortcuts";
import { ThemeProvider, useTokens } from "../theme/ThemeProvider";
import { monoFamily } from "../theme/typography";
import { useBreakpoint } from "../theme/useBreakpoint";

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
const RAILLESS_ROUTES = /^\/(settings|configuration|skills|hooks|models|plans|mcp|usage|session-tree|gallery|connect)(\/|$)/;

function RootNavigator() {
  const { isLoading, isPaired } = useAuth();
  const tokens = useTokens();
  const { isExpanded } = useBreakpoint();
  const pathname = usePathname();
  const railless = RAILLESS_ROUTES.test(pathname);

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
        <Stack.Screen name="plans" />

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

  return (
    <>
      {isPaired && isExpanded ? (
        <>
          {isWeb && !isTauri ? <WebTopBar /> : null}
          <MasterDetail master={railless ? null : <ExpandedFleetRail />} detail={appStack} />
        </>
      ) : (
        appStack
      )}
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
  useOtaUpdates(); // EAS Update OTA check on launch + foreground (no-op in dev / when disabled)

  useEffect(() => {
    void initHaptics();
    if (isTauri) void checkForDesktopUpdate().catch(() => undefined);
  }, []);

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
            </AuthProvider>
          </ThemeProvider>
        </ErrorBoundary>
      </SafeAreaProvider>
    </GestureHandlerRootView>
  );
}
