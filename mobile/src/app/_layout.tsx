// Root providers: persisted react-query client (warm-start cache, UI_RULES.md #25),
// auth/pairing gate, and the top-level Stack. No custom fonts are loaded — the web UI
// uses the system font stack (-apple-system/system-ui), so this app matches it with
// platform defaults (BUILD_PLAN §3) rather than bundling a font asset.
// HANDOFF(T0.1): removed the NativeWind global.css import (file deleted, T0.1 dropped
// NativeWind/Tailwind per ARCHITECTURE §1). This whole file is replaced by T2.1
// (ThemeProvider, AuthProvider, PersistQueryClientProvider root shell).
import { createAsyncStoragePersister } from "@tanstack/query-async-storage-persister";
import { QueryClient } from "@tanstack/react-query";
import { PersistQueryClientProvider } from "@tanstack/react-query-persist-client";
import AsyncStorage from "@react-native-async-storage/async-storage";
import { Stack } from "expo-router";
import { StatusBar } from "expo-status-bar";
import React, { useMemo } from "react";
import { SafeAreaProvider } from "react-native-safe-area-context";

import { AppLock } from "../components/AppLock";
import { Loading, Screen } from "../components/ui";
import { AuthProvider, useAuth } from "../lib/auth";
import { colors } from "../lib/theme";

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

  if (isLoading) {
    return (
      <Screen>
        <Loading />
      </Screen>
    );
  }

  return (
    <Stack
      screenOptions={{
        headerStyle: { backgroundColor: colors.panel },
        headerTintColor: colors.ink,
        headerTitleStyle: { color: colors.ink, fontWeight: "700" },
        contentStyle: { backgroundColor: colors.bg },
      }}
    >
      <Stack.Protected guard={!isPaired}>
        <Stack.Screen name="connect" options={{ headerShown: false }} />
      </Stack.Protected>
      <Stack.Protected guard={isPaired}>
        <Stack.Screen name="(tabs)" options={{ headerShown: false }} />
        <Stack.Screen name="session/[id]" options={{ headerShown: false }} />
        <Stack.Screen
          name="new-session"
          options={{ presentation: "modal", title: "New session" }}
        />
        <Stack.Screen name="settings" options={{ title: "Settings" }} />
      </Stack.Protected>
    </Stack>
  );
}

export default function RootLayout() {
  const persistOptions = useMemo(
    () => ({ persister: asyncStoragePersister }),
    [],
  );

  return (
    <SafeAreaProvider>
      <PersistQueryClientProvider
        client={queryClient}
        persistOptions={persistOptions}
      >
        <AuthProvider>
          <StatusBar style="light" />
          <AppLock>
            <RootNavigator />
          </AppLock>
        </AuthProvider>
      </PersistQueryClientProvider>
    </SafeAreaProvider>
  );
}
