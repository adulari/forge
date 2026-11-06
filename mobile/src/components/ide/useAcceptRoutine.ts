// Runs "Accept routine" for the queue in view and hands back an undo. The planning lives in
// acceptRoutine.ts; this is the network and the toast.
import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useRef, useState } from "react";

import { applyGitHunk, getGitDiff, type GitHunkRequest } from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { useToast } from "../ds/ToastHost";
import { isGitReviewReadOnly } from "../git/gitReviewSupport";
import { acceptMessage, planAcceptRoutine, routineStageRequests, undoStageRequests, type AcceptPlan } from "./acceptRoutine";
import { useIde } from "./IdeContext";
import { markFileReviewed, unmarkFileReviewed } from "./reviewedStore";

const UNDO_TOAST_MS = 9000;

export function useAcceptRoutine() {
  const { rows, sessions } = useIde();
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  const running = useRef(false);

  const readOnly = isGitReviewReadOnly(baseUrl);
  const worktrees = new Set(sessions.filter((row) => row.worktree).map((row) => row.id));
  const plan: AcceptPlan = planAcceptRoutine(rows, worktrees, readOnly);
  const count = plan.stage.reduce((n, row) => n + (row.file.routineHunks ?? 0), 0) + plan.review.length;

  const accept = useCallback(async () => {
    if (running.current || baseUrl == null) return;
    running.current = true;
    setBusy(true);
    const staged: GitHunkRequest[] = [];
    const touched = new Set<string>();
    const reviewed = plan.review.map((row) => ({ sessionId: row.sessionId, path: row.path, hash: row.file.hash }));
    let failed = 0;
    try {
      for (const row of plan.stage) {
        try {
          const diff = await getGitDiff(baseUrl, { session: row.sessionId, path: row.path, staged: false, context: 3 });
          const file = diff.files.find((candidate) => candidate.path === row.path) ?? diff.files[0];
          if (!file) continue;
          for (const request of routineStageRequests(row.sessionId, file)) {
            try {
              await applyGitHunk(baseUrl, request);
              staged.push(request);
              touched.add(row.sessionId);
            } catch {
              failed += 1;
            }
          }
        } catch {
          failed += 1;
        }
      }
      for (const mark of reviewed) markFileReviewed(mark.sessionId, mark.path, mark.hash);
    } finally {
      running.current = false;
      setBusy(false);
      for (const session of touched) {
        for (const key of [
          ["git", "status", baseUrl, session],
          ["git", "diff", baseUrl, session],
          ["sessions", "diff", baseUrl, session],
          ["workspace", "file", baseUrl, session],
        ]) {
          void queryClient.invalidateQueries({ queryKey: key });
        }
      }
      void queryClient.invalidateQueries({ queryKey: ["ide", "changes", baseUrl] });
    }
    const summary = { hunks: staged.length, files: new Set(staged.map((request) => `${request.session}\0${request.path}`)).size, failed };
    const undo = async () => {
      try {
        for (const request of undoStageRequests(staged)) await applyGitHunk(baseUrl, request);
        for (const mark of reviewed) unmarkFileReviewed(mark.sessionId, mark.path);
        toast.show("Restored", { tone: "success" });
      } catch {
        toast.show("Could not undo: the files changed since.", { tone: "danger" });
      } finally {
        for (const session of touched) void queryClient.invalidateQueries({ queryKey: ["git", "status", baseUrl, session] });
        void queryClient.invalidateQueries({ queryKey: ["ide", "changes", baseUrl] });
      }
    };
    const message = acceptMessage(summary, reviewed.length);
    if (staged.length + reviewed.length === 0) {
      toast.show(message, { tone: failed > 0 ? "danger" : "neutral" });
      return;
    }
    toast.show(message, { duration: UNDO_TOAST_MS, action: { label: "Undo", onPress: () => void undo() } });
  }, [baseUrl, plan, queryClient, toast]);

  return { accept, busy, count, hasWork: plan.stage.length + plan.review.length > 0 };
}
