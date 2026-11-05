// What IDE mode reads: the project's agents and, for each, the files it changed. Everything is
// derived from the fleet list the app already keeps live plus one query per working tree; no
// socket of its own.
import { useQueries, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useRef } from "react";

import { getGitStatus, getSessionDiff, type SessionRow } from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { useProjectChanges as useProjectChangesRoute, useSessions } from "../../lib/queries";
import { projectChips } from "../../lib/fleetProjects";
import { projectName } from "../../lib/sidebarModel";
import {
  filesFromGitStatus,
  filesFromSessionDiff,
  planChangeQueries,
  projectSessions,
  type SessionChanges,
} from "./changesAdapter";
import { changesCwd, routeMissing, sessionChangesFromRoute } from "./changesFromRoute";
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
 * One call to `GET /api/projects/changes?cwd=` (real content hashes, attributed per agent). A
 * daemon that predates the route answers 404, and only then does the client fall back to one
 * query per working tree.
 */
export function useProjectChanges(sessions: readonly SessionRow[]): ProjectChanges {
  const cwd = useMemo(() => changesCwd(sessions), [sessions]);
  const route = useProjectChangesRoute(cwd, sessions.length > 0);
  const fallback = route.isError && routeMissing(route.error);
  // The fleet already knows when a session's line counts move: refresh right then, not on the poll.
  const fleetSignature = sessions.map((row) => `${row.id}:${row.diff_added ?? "-"}:${row.diff_removed ?? "-"}:${row.busy ? 1 : 0}`).join("|");
  const lastSignature = useRef(fleetSignature);
  const { refetch } = route;
  useEffect(() => {
    if (lastSignature.current === fleetSignature) return;
    lastSignature.current = fleetSignature;
    void refetch();
  }, [fleetSignature, refetch]);
  const legacy = useLegacyChanges(fallback ? sessions : []);
  const bySession = useMemo(
    () => (route.data && !fallback ? sessionChangesFromRoute(route.data, sessions) : legacy.bySession),
    [route.data, fallback, sessions, legacy.bySession],
  );
  const loading = fallback ? legacy.loading : route.isPending && sessions.length > 0;
  // An open diff or file must follow an agent's next edit: when a file's content hash moves, the
  // per-file queries for that session are stale.
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  const seenHashes = useRef(new Map<string, string>());
  useEffect(() => {
    const touched = new Set<string>();
    const seen = seenHashes.current;
    for (const [id, changes] of bySession) {
      for (const file of changes.files) {
        const key = `${id}\u0000${file.path}`;
        const before = seen.get(key);
        if (before !== undefined && before !== file.hash) touched.add(id);
        seen.set(key, file.hash);
      }
    }
    for (const id of touched) {
      void queryClient.invalidateQueries({ queryKey: sessionDiffKey(baseUrl, id) });
      void queryClient.invalidateQueries({ queryKey: ["git", "diff", baseUrl, id] });
      void queryClient.invalidateQueries({ queryKey: ["git", "status", baseUrl, id] });
      void queryClient.invalidateQueries({ queryKey: ["workspace", "file", baseUrl, id] });
    }
  }, [bySession, baseUrl, queryClient]);
  const value = useMemo<ProjectChanges>(() => ({ bySession, loading }), [bySession, loading]);

  // Marks for files that changed again or left the set go away, but only for trees that answered.
  useEffect(() => {
    if (fallback || !route.data) return;
    const live = new Map<string, Map<string, string>>();
    for (const [id, changes] of bySession) {
      if (route.data.sessions.some((group) => group.session_id === id)) {
        live.set(id, new Map(changes.files.map((file) => [file.path, file.hash])));
      }
    }
    if (live.size > 0) pruneReviewedMarks(live);
  }, [bySession, fallback, route.data]);
  return value;
}

/** One query per working tree, for daemons without the project-changes route. */
function useLegacyChanges(sessions: readonly SessionRow[]): ProjectChanges {
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
