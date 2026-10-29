// "Resolve with Forge": starts a turn on the session that fixes the conflicted files. The dock is
// not always rendered inside a session's socket (the workbench dock follows the focused pane), so
// the prompt goes out through a short-lived SessionProvider mounted only while a resolve is in
// flight, the same way the secondary docks borrow a session.
import React, { useEffect, useRef } from "react";

import { conflictResolutionPrompt } from "./gitReviewSupport";
import { SessionProvider, useSessionCtx } from "../../lib/sessionContext";

const CONNECT_TIMEOUT_MS = 8_000;

export interface ConflictResolveRequest {
  paths: string[];
  operation: string | null;
}

function SendOnce({
  request,
  onSent,
  onFailed,
}: {
  request: ConflictResolveRequest;
  onSent: () => void;
  onFailed: (message: string) => void;
}) {
  const { send, connectionState } = useSessionCtx();
  const done = useRef(false);

  useEffect(() => {
    if (done.current || connectionState !== "open") return;
    done.current = true;
    const text = conflictResolutionPrompt(request.paths, request.operation);
    if (send({ kind: "prompt", text })) onSent();
    else onFailed("Forge could not take the request. Try again in a moment.");
  }, [connectionState, onFailed, onSent, request, send]);

  useEffect(() => {
    const timer = setTimeout(() => {
      if (done.current) return;
      done.current = true;
      onFailed("Could not reach the session to hand it the conflicts.");
    }, CONNECT_TIMEOUT_MS);
    return () => clearTimeout(timer);
  }, [onFailed]);

  return null;
}

export function ConflictResolver({
  sessionId,
  request,
  onSent,
  onFailed,
}: {
  sessionId: string;
  request: ConflictResolveRequest | null;
  onSent: () => void;
  onFailed: (message: string) => void;
}) {
  if (!request) return null;
  return (
    <SessionProvider key={`${sessionId}:${request.paths.join("\0")}`} sessionId={sessionId}>
      <SendOnce request={request} onSent={onSent} onFailed={onFailed} />
    </SessionProvider>
  );
}
