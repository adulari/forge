// A stack of its own for sessions, so the dynamic segment is a route literally named `[id]`.
//
// Without it the root stack held one route named "session/[id]", and expo-router only compares
// dynamic params when a route's whole name is a bracketed segment. Going from one session to
// another (the command palette, a fork, a decision card opened from inside a session) therefore
// looked like "the same screen", navigated inside the current session's nested routes, and kept
// showing the session you were already on. Here `/session/A` → `/session/B` diverges on `id` and
// pushes a real screen, with Back returning to A.
import { Stack } from "expo-router";
import React from "react";

import { useTokens } from "../../theme/ThemeProvider";

export default function SessionStackLayout() {
  const tokens = useTokens();
  return <Stack screenOptions={{ headerShown: false, contentStyle: { backgroundColor: tokens.bg1 } }} />;
}
