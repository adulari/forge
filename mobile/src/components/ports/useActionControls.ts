import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";

import { ApiError } from "../../lib/api";
import { runProjectAction, stopProjectAction } from "../../lib/apiProjectActions";
import { useDirectBase } from "../../lib/queriesProjectActions";
import { useToast } from "../ds/ToastHost";
import { requestOpenActionOutput } from "./actionOutputBus";

export interface ActionRef {
  id: string;
  name: string;
}

const message = (error: unknown, fallback: string) => (error instanceof ApiError && error.message ? error.message : fallback);

/** Run and stop for project actions, with toasts and cache refresh. */
export function useActionControls() {
  const baseUrl = useDirectBase();
  const toast = useToast();
  const queryClient = useQueryClient();

  const refresh = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: ["project-actions"] });
    void queryClient.invalidateQueries({ queryKey: ["project-actions-running"] });
    void queryClient.invalidateQueries({ queryKey: ["session-ports"] });
  }, [queryClient]);

  const runMutation = useMutation({
    mutationFn: ({ sessionId, action }: { sessionId: string; action: ActionRef; openOutput: boolean }) =>
      runProjectAction(baseUrl as string, sessionId, action.id),
    onSuccess: (result, { sessionId, action, openOutput }) => {
      toast.show(`Started ${action.name}`, { tone: "success" });
      if (openOutput) requestOpenActionOutput({ sessionId, terminalId: result.terminal_id, title: action.name });
      refresh();
    },
    onError: (error, { action }) => toast.show(message(error, `Could not start ${action.name}`), { tone: "danger" }),
  });

  const stopMutation = useMutation({
    mutationFn: ({ sessionId, action }: { sessionId: string; action: ActionRef }) =>
      stopProjectAction(baseUrl as string, sessionId, action.id),
    onSuccess: (result, { action }) => {
      toast.show(result.stopped ? `Stopped ${action.name}` : `${action.name} was not running`, { tone: "neutral" });
      refresh();
    },
    onError: (error, { action }) => toast.show(message(error, `Could not stop ${action.name}`), { tone: "danger" }),
  });

  const run = useCallback(
    (sessionId: string, action: ActionRef, openOutput = true) => runMutation.mutate({ sessionId, action, openOutput }),
    [runMutation],
  );
  const stop = useCallback(
    (sessionId: string, action: ActionRef) => stopMutation.mutate({ sessionId, action }),
    [stopMutation],
  );

  return {
    available: baseUrl != null,
    run,
    stop,
    pendingActionId:
      (runMutation.isPending ? runMutation.variables?.action.id : null) ??
      (stopMutation.isPending ? stopMutation.variables?.action.id : null) ??
      null,
  };
}
