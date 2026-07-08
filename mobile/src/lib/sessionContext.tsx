// Per-session context: owns the ONE `useSessionSocket` instance for a session and exposes
// it to every segment (Chat/Tasks/Agents/Review) under `session/[id]/`. The session shell
// layout (`src/app/session/[id]/_layout.tsx`) mounts `SessionProvider` exactly once; child
// route segments consume it via `useSessionCtx()` and never create their own socket
// (UI_RULES.md #3 — data via hooks only, and BUILD_PLAN §6 Session shell contract).
import React, { createContext, useContext, useMemo, useState } from "react";

import { useAuth } from "./auth";
import { type ConnectionState, type RemoteInput, type Snapshot, useSessionSocket } from "./ws";

export interface SessionCtxValue {
  sessionId: string;
  baseUrl: string | null;
  snapshot: Snapshot | null;
  connectionState: ConnectionState;
  send: (input: RemoteInput) => void;
  /** Measured height of the shell's header block (SessionHeader + banners + StatusStrip +
   * Segmented) — segments below it use this as `Screen`'s `keyboardVerticalOffset` so
   * KeyboardAvoidingView knows how much real screen-top content sits above it (RN's own docs
   * call for this; it isn't inferred automatically). 0 until the shell's first layout pass. */
  headerHeight: number;
  setHeaderHeight: (h: number) => void;
}

const SessionCtx = createContext<SessionCtxValue | null>(null);

export function SessionProvider({
  sessionId,
  children,
}: {
  sessionId: string;
  children: React.ReactNode;
}) {
  const { baseUrl } = useAuth();
  const { snapshot, connectionState, send } = useSessionSocket(baseUrl, sessionId);
  const [headerHeight, setHeaderHeight] = useState(0);

  const value = useMemo<SessionCtxValue>(
    () => ({ sessionId, baseUrl, snapshot, connectionState, send, headerHeight, setHeaderHeight }),
    [sessionId, baseUrl, snapshot, connectionState, send, headerHeight],
  );

  return <SessionCtx.Provider value={value}>{children}</SessionCtx.Provider>;
}

/** B3 contract: `useSessionCtx()` — call from any segment under `session/[id]/`. */
export function useSessionCtx(): SessionCtxValue {
  const ctx = useContext(SessionCtx);
  if (!ctx) throw new Error("useSessionCtx must be used within a SessionProvider");
  return ctx;
}
