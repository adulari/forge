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
import { Redirect, Stack } from "expo-router";
import * as SplashScreen from "expo-splash-screen";
import React, { useEffect, useMemo } from "react";
import { ActivityIndicator } from "react-native";
import { GestureHandlerRootView } from "react-native-gesture-handler";
import { SafeAreaProvider } from "react-native-safe-area-context";

import { AppLock } from "../components/AppLock";
import { Screen } from "../components/ds/Screen";
import { ToastHost } from "../components/ds/ToastHost";
import { PaletteHost } from "../components/overlay/CommandPalette";
import { AuthProvider, useAuth } from "../lib/auth";
import { ThemeProvider, useTokens } from "../theme/ThemeProvider";

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
          options={{ headerShown: true, presentation: "modal", title: "New session" }}
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

  return (
    <GestureHandlerRootView style={{ flex: 1 }}>
      <SafeAreaProvider>
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
            </PersistQueryClientProvider>
          </AuthProvider>
        </ThemeProvider>
      </SafeAreaProvider>
    </GestureHandlerRootView>
  );
}
