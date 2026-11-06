import React, { useCallback } from "react";

import { clearHostReclaimRequest, useHostReclaimRequested } from "../lib/hostReclaimRequest";
import { reclaimConfirmMessage, reclaimResultMessage } from "../lib/hostPressure";
import { useHostPressure, useReclaimHostSpace, useReclaimPreview } from "../lib/queries";
import { ConfirmDialog } from "./ds/ConfirmDialog";
import { useToast } from "./ds/ToastHost";

/**
 * The one confirmation in front of Reclaim for the banner and the palette: it deletes worktrees
 * and build directories, so it says what goes and how much it frees before anything runs.
 */
export function HostReclaimDialog() {
  const requested = useHostReclaimRequested();
  const pressure = useHostPressure(requested);
  const preview = useReclaimPreview(requested);
  const reclaim = useReclaimHostSpace();
  const toast = useToast();

  const confirm = useCallback(async () => {
    clearHostReclaimRequest();
    try {
      toast.show(reclaimResultMessage(await reclaim.mutateAsync(undefined)), { tone: "neutral" });
    } catch {
      toast.show("Couldn't reclaim space. Try `forge worktree reclaim` on the host.", { tone: "danger" });
    }
  }, [reclaim, toast]);

  return (
    <ConfirmDialog
      visible={requested}
      destructive
      title="Reclaim worktree space?"
      message={reclaimConfirmMessage(pressure.data?.summary, preview.isLoading ? undefined : preview.data)}
      confirmLabel="Reclaim"
      onConfirm={() => void confirm()}
      onCancel={clearHostReclaimRequest}
    />
  );
}
