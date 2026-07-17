// FEATURES.md §1.2 — `overlay` auto-presents on non-null, auto-dismisses on
// null. No props: reads `useSessionCtx().snapshot.overlay` directly, mounted
// once by the session shell (T3.1) alongside the rest of the socket-driven UI.
//
// The last non-null overlay is retained in local state so OverlayPanel keeps
// real content to animate away from when the server nulls it out — passing
// `visible={overlay != null}` down lets the Sheet/CenteredModal Anvil exit
// transition play instead of the panel vanishing mid-close.
import React, { useEffect, useState } from "react";

import { useSessionCtx } from "../../lib/sessionContext";
import type { Overlay } from "../../lib/ws";
import { OverlayPanel } from "./OverlayPanel";

export function OverlayHost() {
  const { snapshot, send } = useSessionCtx();
  // `overlay:workflow` is the daemon's projection of its own full-screen workflow view. The app
  // now has a dedicated workflow run screen (`session/[id]/workflow.tsx`) reached from the live
  // workflow pill in the session header, so popping this modal too would stack two competing
  // workflow UIs (seen as a modal-over-screen glitch). Ignore it here; it is harmless projection
  // state server-side and auto-clears when the run backgrounds.
  const raw = snapshot?.overlay ?? null;
  const overlay = raw?.kind === "overlay:workflow" ? null : raw;

  const [lastOverlay, setLastOverlay] = useState<Overlay | null>(null);

  useEffect(() => {
    if (overlay != null) setLastOverlay(overlay);
  }, [overlay]);

  if (!lastOverlay) return null;

  return (
    <OverlayPanel
      key={lastOverlay.kind}
      overlay={lastOverlay}
      visible={overlay != null}
      send={send}
      onClose={() => send({ kind: "overlay_cancel" })}
    />
  );
}
