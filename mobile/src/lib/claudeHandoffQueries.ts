// Calls and the one-step flows for moving a conversation between Forge and Claude Code
// (crates/forge-cli/src/serve_claude_link.rs). The pure parts are in claudeHandoff.ts.
import * as Clipboard from "expo-clipboard";
import { router } from "expo-router";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";

import { requestIdeClaudeTerminal } from "../components/ide/ideEvents";
import { useToast } from "../components/ds/ToastHost";
import { requestOpenActionOutput } from "../components/ports/actionOutputBus";
import { ApiError, request } from "./api";
import { useAuth } from "./auth";
import {
  CLAUDE_TERMINAL_ID,
  CLAUDE_TERMINAL_TITLE,
  type ClaudeContinueReply,
  type ClaudeExportReply,
  continueToast,
  copyableCommand,
  queueInitialCommand,
  takeInitialCommand,
} from "./claudeHandoff";
import { openSession } from "./nav";

const text = (error: unknown, fallback: string) => (error instanceof ApiError || error instanceof Error ? error.message : fallback);

export function useExportToClaude() {
  const { baseUrl } = useAuth();
  return useMutation<ClaudeExportReply, Error, string>({
    mutationFn: (sessionId) =>
      request(baseUrl as string, `/api/sessions/${encodeURIComponent(sessionId)}/claude`, { method: "POST", body: "{}" }, 60_000),
  });
}

export function useContinueClaudeSession() {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation<ClaudeContinueReply, Error, string>({
    mutationFn: (ccSessionId) =>
      request(
        baseUrl as string,
        `/api/claude/sessions/${encodeURIComponent(ccSessionId)}/continue`,
        { method: "POST", body: "{}" },
        120_000,
      ),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["sessions"] });
      void queryClient.invalidateQueries({ queryKey: ["import-scan"] });
    },
  });
}

/**
 * "Continue in Claude Code": bring the Claude Code transcript up to date, then open a terminal in
 * the session's folder with `claude --resume …` typed in. Where there is no terminal to open (a
 * phone) the command is copied instead, and the terminal screen opens if the session has one.
 */
export function useContinueInClaudeCode(): (sessionId: string) => void {
  const toast = useToast();
  const exportToClaude = useExportToClaude();
  const { mutate } = exportToClaude;
  return useCallback(
    (sessionId: string) => {
      mutate(sessionId, {
        onSuccess: (reply) => {
          const copy = () =>
            void Clipboard.setStringAsync(copyableCommand(reply)).then(() => toast.show("Command copied"));
          queueInitialCommand(sessionId, CLAUDE_TERMINAL_ID, reply.command);
          const opened =
            requestIdeClaudeTerminal(sessionId) ||
            requestOpenActionOutput({ sessionId, terminalId: CLAUDE_TERMINAL_ID, title: CLAUDE_TERMINAL_TITLE });
          if (opened) {
            toast.show("Opening Claude Code in the terminal", {
              tone: "success",
              action: { label: "Copy command", onPress: copy },
            });
            return;
          }
          takeInitialCommand(sessionId, CLAUDE_TERMINAL_ID);
          copy();
          router.push(`/session/${sessionId}/terminal` as Parameters<typeof router.push>[0]);
        },
        onError: (error) => toast.show(text(error, "Could not hand this session to Claude Code"), { tone: "danger" }),
      });
    },
    [mutate, toast],
  );
}

/** Continue one Claude Code conversation in Forge (import or sync) and open the Forge session. */
export function useContinueFromClaudeCode(): (
  ccSessionId: string,
  onDone?: () => void,
  onFailed?: () => void,
) => void {
  const toast = useToast();
  const { mutate } = useContinueClaudeSession();
  return useCallback(
    (ccSessionId: string, onDone?: () => void, onFailed?: () => void) => {
      mutate(ccSessionId, {
        onSuccess: (reply) => {
          toast.show(continueToast(reply), { tone: "success" });
          onDone?.();
          openSession(reply.session_id);
        },
        onError: (error) => {
          toast.show(text(error, "Could not continue this Claude Code session"), { tone: "danger" });
          onFailed?.();
        },
      });
    },
    [mutate, toast],
  );
}
