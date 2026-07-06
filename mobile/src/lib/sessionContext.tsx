// Per-session context: owns the ONE `useSessionSocket` instance for a session and exposes
// it to every segment (Chat/Tasks/Agents/Review) under `session/[id]/`. The session shell
// layout (`src/app/session/[id]/_layout.tsx`) mounts `SessionProvider` exactly once; child
// route segments consume it via `useSessionCtx()` and never create their own socket
// (UI_RULES.md #3 — data via hooks only, and BUILD_PLAN §6 Session shell contract).
import React, { createContext, useContext, useMemo } from "react";

import { useAuth } from "./auth";
import { type ConnectionState, type RemoteInput, type Snapshot, useSessionSocket } from "./ws";

export interface SessionCtxValue {
  sessionId: string;
  baseUrl: string | null;
  snapshot: Snapshot | null;
  connectionState: ConnectionState;
  send: (input: RemoteInput) => void;
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

  const value = useMemo<SessionCtxValue>(
    () => ({ sessionId, baseUrl, snapshot, connectionState, send }),
    [sessionId, baseUrl, snapshot, connectionState, send],
  );

  return <SessionCtx.Provider value={value}>{children}</SessionCtx.Provider>;
}

/** B3 contract: `useSessionCtx()` — call from any segment under `session/[id]/`. */
export function useSessionCtx(): SessionCtxValue {
  const ctx = useContext(SessionCtx);
  if (!ctx) throw new Error("useSessionCtx must be used within a SessionProvider");
  return ctx;
}
