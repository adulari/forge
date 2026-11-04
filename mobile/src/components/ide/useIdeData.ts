// What IDE mode reads: the project's agents and, for each, the files it changed. Everything is
// derived from the fleet list the app already keeps live plus one query per working tree; no
// socket of its own.
import { useQueries, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useRef } from "react";

import { getGitStatus, getSessionDiff, type SessionRow } from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { useSessions } from "../../lib/queries";
import { projectChips } from "../../lib/fleetProjects";
import { projectName } from "../../lib/sidebarModel";
import {
  filesFromGitStatus,
  filesFromSessionDiff,
  planChangeQueries,
  projectSessions,
  type SessionChanges,
} from "./changesAdapter";
import { pruneReviewedMarks } from "./reviewedStore";

// The same keys lib/queries.ts uses for the Review dock, so a stage, discard or commit there
// refreshes the queue without any wiring of its own.
const gitStatusKey = (baseUrl: string | null, id: string) => ["git", "status", baseUrl, id] as const;
const sessionDiffKey = (baseUrl: string | null, id: string) => ["sessions", "diff", baseUrl, id] as const;

const POLL_MS = 20_000;

export interface IdeProjects {
  rows: SessionRow[];
  projects: ReturnType<typeof projectChips>;
  loading: boolean;
}

export function useIdeProjects(): IdeProjects {
  const { data, isPending } = useSessions();
  return useMemo(() => {
    const rows = data ?? [];
    return { rows, projects: projectChips(rows), loading: isPending && rows.length === 0 };
  }, [data, isPending]);
}

export function useProjectSessions(rows: readonly SessionRow[], project: string | null): SessionRow[] {
  return useMemo(() => projectSessions(rows, project), [rows, project]);
}

export interface ProjectChanges {
  bySession: ReadonlyMap<string, SessionChanges>;
  /** Some tree has not answered yet. */
  loading: boolean;
}

/**
 * One query per working tree. Swap this body for `GET /api/projects/changes?cwd=` when the route
 * exists; consumers only see `SessionChanges`.
 */
export function useProjectChanges(sessions: readonly SessionRow[]): ProjectChanges {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  const plan = useMemo(() => planChangeQueries(sessions), [sessions]);
  const holders = useMemo(() => plan.filter((entry) => entry.sharedWith == null), [plan]);

  const results = useQueries({
    queries: holders.map(({ row }) =>
      row.worktree
        ? {
            queryKey: sessionDiffKey(baseUrl, row.id),
            queryFn: () => getSessionDiff(baseUrl as string, row.id),
            enabled: baseUrl != null,
            refetchInterval: POLL_MS,
            staleTime: 5_000,
          }
        : {
            queryKey: gitStatusKey(baseUrl, row.id),
            queryFn: () => getGitStatus(baseUrl as string, row.id),
            enabled: baseUrl != null,
            refetchInterval: POLL_MS,
            staleTime: 5_000,
          },
    ),
  });

  // The fleet already knows when a session's line counts move: refresh that tree right then
  // instead of waiting out the poll.
  const signature = useRef(new Map<string, string>());
  useEffect(() => {
    for (const { row } of holders) {
      const next = `${row.diff_added ?? "-"}:${row.diff_removed ?? "-"}:${row.busy ? 1 : 0}`;
      const previous = signature.current.get(row.id);
      signature.current.set(row.id, next);
      if (previous != null && previous !== next) {
        void queryClient.invalidateQueries({
          queryKey: row.worktree ? sessionDiffKey(baseUrl, row.id) : gitStatusKey(baseUrl, row.id),
        });
      }
    }
  }, [baseUrl, holders, queryClient]);

  const dataKey = results.map((result) => result.dataUpdatedAt).join(",");
  const value = useMemo<ProjectChanges>(() => {
    const bySession = new Map<string, SessionChanges>();
    const holderIndex = new Map(holders.map((entry, index) => [entry.row.id, index]));
    let loading = false;
    for (const { row, sharedWith } of plan) {
      if (sharedWith != null) {
        bySession.set(row.id, { sessionId: row.id, source: "tree", files: [], sharedWith });
        continue;
      }
      const result = results[holderIndex.get(row.id) ?? -1];
      if (result?.isPending) loading = true;
      const data = result?.data;
      if (!data) {
        bySession.set(row.id, { sessionId: row.id, source: row.worktree ? "worktree" : "tree", files: [], sharedWith: null });
        continue;
      }
      bySession.set(row.id, {
        sessionId: row.id,
        source: row.worktree ? "worktree" : "tree",
        files: row.worktree
          ? filesFromSessionDiff((data as Awaited<ReturnType<typeof getSessionDiff>>).files)
          : filesFromGitStatus(data as Awaited<ReturnType<typeof getGitStatus>>),
        sharedWith: null,
      });
    }
    return { bySession, loading };
    // `results` identity changes every render; its data timestamps are the real signal.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [plan, holders, dataKey]);

  // Marks for files that changed again or left the set go away, but only for trees that answered.
  useEffect(() => {
    const live = new Map<string, Map<string, string>>();
    holders.forEach(({ row }, index) => {
      if (!results[index]?.data) return;
      const changes = value.bySession.get(row.id);
      if (changes) live.set(row.id, new Map(changes.files.map((file) => [file.path, file.hash])));
    });
    if (live.size > 0) pruneReviewedMarks(live);
  }, [value, holders, results]);

  return value;
}

export function useProjectNameOfSession(rows: readonly SessionRow[], sessionId: string | null): string | null {
  return useMemo(() => {
    const row = sessionId ? rows.find((candidate) => candidate.id === sessionId) : undefined;
    return row ? projectName(row.cwd) : null;
  }, [rows, sessionId]);
}
