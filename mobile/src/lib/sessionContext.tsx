// Per-session context: owns the ONE `useSessionSocket` instance for a session and exposes
// it to every segment (Chat/Tasks/Agents/Review) under `session/[id]/`. The session shell
// layout (`src/app/session/[id]/_layout.tsx`) mounts `SessionProvider` exactly once; child
// route segments consume it via `useSessionCtx()` and never create their own socket
// (UI_RULES.md #3 — data via hooks only, and BUILD_PLAN §6 Session shell contract).
import React, { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";

import type { Attachment } from "../components/chat/attach";
import { useAuth } from "./auth";
import { type ConnectionState, type RemoteInput, type Snapshot, useSessionSocket } from "./ws";

/** In-flight Composer draft (text + attachments), keyed by session id so it survives the
 * Chat/Tasks/Agents/Review segment switches that `router.replace` the route out from under
 * the Composer (see Composer.tsx's `commit`) — but never leaks into a wrong session if this
 * provider instance is ever handed a different `sessionId` without remounting. Deliberately
 * session-lifetime only (no AsyncStorage): a half-typed draft dying with the app is fine. */
interface Draft {
  text: string;
  attachments: Attachment[];
}

const EMPTY_DRAFT: Draft = { text: "", attachments: [] };

// A deep link into a nonexistent/foreign session id never gets a Snapshot over WS (the
// daemon has nothing to send) — without a deadline every segment would wait forever and the
// skeleton/empty states would never resolve to an honest "this session doesn't exist".
const SNAPSHOT_TIMEOUT_MS = 10_000;

export interface SessionCtxValue {
  sessionId: string;
  baseUrl: string | null;
  snapshot: Snapshot | null;
  /** True once this session has been mounted for `SNAPSHOT_TIMEOUT_MS` with no Snapshot ever
   * arriving over WS — segments use this to stop rendering "loading" skeletons/fillers and
   * show an honest error state instead. Resets on every `snapshot != null` and on session
   * switch. */
  snapshotTimedOut: boolean;
  connectionState: ConnectionState;
  send: (input: RemoteInput) => boolean;
  /** Measured height of the shell's header block (SessionHeader + banners + StatusStrip +
   * Segmented) — segments below it use this as `Screen`'s `keyboardVerticalOffset` so
   * KeyboardAvoidingView knows how much real screen-top content sits above it (RN's own docs
   * call for this; it isn't inferred automatically). 0 until the shell's first layout pass. */
  headerHeight: number;
  setHeaderHeight: (h: number) => void;
  /** Composer draft for THIS session — see `Draft` above. */
  draftText: string;
  draftAttachments: Attachment[];
  setDraftText: (text: string) => void;
  setDraftAttachments: (next: Attachment[] | ((prev: Attachment[]) => Attachment[])) => void;
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
  const [drafts, setDrafts] = useState<Record<string, Draft>>({});
  const draft = drafts[sessionId] ?? EMPTY_DRAFT;

  const [snapshotTimedOut, setSnapshotTimedOut] = useState(false);
  const hasSnapshot = snapshot != null;
  useEffect(() => {
    if (hasSnapshot) {
      setSnapshotTimedOut(false);
      return;
    }
    setSnapshotTimedOut(false);
    const t = setTimeout(() => setSnapshotTimedOut(true), SNAPSHOT_TIMEOUT_MS);
    return () => clearTimeout(t);
  }, [sessionId, hasSnapshot]);

  const setDraftText = useCallback(
    (text: string) => {
      setDrafts((prev) => ({ ...prev, [sessionId]: { text, attachments: prev[sessionId]?.attachments ?? [] } }));
    },
    [sessionId],
  );

  const setDraftAttachments = useCallback(
    (next: Attachment[] | ((prev: Attachment[]) => Attachment[])) => {
      setDrafts((prev) => {
        const current = prev[sessionId] ?? EMPTY_DRAFT;
        const attachments = typeof next === "function" ? next(current.attachments) : next;
        return { ...prev, [sessionId]: { text: current.text, attachments } };
      });
    },
    [sessionId],
  );

  const value = useMemo<SessionCtxValue>(
    () => ({
      sessionId,
      baseUrl,
      snapshot,
      snapshotTimedOut,
      connectionState,
      send,
      headerHeight,
      setHeaderHeight,
      draftText: draft.text,
      draftAttachments: draft.attachments,
      setDraftText,
      setDraftAttachments,
    }),
    [
      sessionId,
      baseUrl,
      snapshot,
      snapshotTimedOut,
      connectionState,
      send,
      headerHeight,
      draft,
      setDraftText,
      setDraftAttachments,
    ],
  );

  return <SessionCtx.Provider value={value}>{children}</SessionCtx.Provider>;
}

/** B3 contract: `useSessionCtx()` — call from any segment under `session/[id]/`. */
export function useSessionCtx(): SessionCtxValue {
  const ctx = useContext(SessionCtx);
  if (!ctx) throw new Error("useSessionCtx must be used within a SessionProvider");
  return ctx;
}
