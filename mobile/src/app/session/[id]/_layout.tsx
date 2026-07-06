// Session detail stack — NAV-SHELL PLACEHOLDER for B0. Each segment below renders its own
// placeholder body for now. Batch 2 (W4, BUILD_PLAN §7) replaces this with the real
// session shell: ONE `useSessionSocket(id)` instance shared via context across segments,
// a custom header + sticky status strip, and the `Segmented` primitive (Chat/Tasks/Agents/
// Review) swapping content WITHOUT remounting the socket — likely via expo-router's
// `withLayoutContext` (a custom tab-like navigator), not plain `Stack.Screen` navigation
// like this placeholder. Do not assume this file's structure is final.
import { Stack } from "expo-router";
import React from "react";

import { colors } from "../../../lib/theme";

export default function SessionLayout() {
  return (
    <Stack
      screenOptions={{
        headerStyle: { backgroundColor: colors.panel },
        headerTintColor: colors.ink,
        headerTitleStyle: { color: colors.ink, fontWeight: "700" },
        contentStyle: { backgroundColor: colors.bg },
      }}
    >
      <Stack.Screen name="index" options={{ title: "Chat" }} />
      <Stack.Screen name="tasks" options={{ title: "Tasks" }} />
      <Stack.Screen name="agents" options={{ title: "Agents" }} />
      <Stack.Screen name="review" options={{ title: "Review" }} />
      <Stack.Screen
        name="overlay"
        options={{ presentation: "modal", title: "Overlay" }}
      />
    </Stack>
  );
}
