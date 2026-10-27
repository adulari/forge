// A stack of its own for sessions, so the dynamic segment is a route literally named `[id]`.
//
// Without it the root stack held one route named "session/[id]", and expo-router only compares
// dynamic params when a route's whole name is a bracketed segment. Going from one session to
// another (the command palette, a fork, a decision card opened from inside a session) therefore
// looked like "the same screen", navigated inside the current session's nested routes, and kept
// showing the session you were already on. Here `/session/A` → `/session/B` diverges on `id` and
// pushes a real screen, with Back returning to A.
//
// Wide web/desktop windows use a plain Slot instead: the `[id]` layout stays mounted across
// sessions and keeps the last few of them mounted itself (see `[id]/_layout.tsx`), so going back
// to a recent session is a visibility flip. There is no Back button there to need the stack.
import { Stack } from "expo-router";
import React from "react";

import { useKeepSessionsMounted } from "../../lib/sessionPrewarm";
import { HostedSessions } from "./[id]/_layout";
import { useTokens } from "../../theme/ThemeProvider";

export default function SessionStackLayout() {
  const tokens = useTokens();
  const keepMounted = useKeepSessionsMounted();
  if (keepMounted) return <HostedSessions />;
  return <Stack screenOptions={{ headerShown: false, contentStyle: { backgroundColor: tokens.bg1 } }} />;
}
