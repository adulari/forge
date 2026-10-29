// Search-backed palette entries: message hits on this host and on every online Anywhere host,
// and workspace file names for the session the Files dock is showing.
import { useQueries, type UseQueryResult } from "@tanstack/react-query";
import { router } from "expo-router";
import { FileText } from "lucide-react-native";
import React, { useCallback, useMemo } from "react";

import { ApiError, createSession, searchSessions, type SessionRow, type SessionSearchResult } from "../../lib/api";
import { useAnywhere, useAnywhereHosts } from "../../lib/anywhere/store";
import { useAuth } from "../../lib/auth";
import { requestMessageJump } from "../../lib/messageJump";
import { openSession } from "../../lib/nav";
import { filesEmptyState, filesHeader, type FilesEmptyState } from "../../lib/palette/sections";
import { useGitStatus, useSessionSearch, useWorkspaceSearch } from "../../lib/queries";
import { supportsDirectDaemonEndpoints } from "../../lib/transport";
import { useTokens } from "../../theme/ThemeProvider";
import { formatCwd, formatRelativeTime } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { StatusDot } from "../ds/StatusDot";
import { useToast } from "../ds/ToastHost";
import { useWorkbench } from "../workbench/WorkbenchProvider";
import { liveSessionState } from "./usePaletteSources";
import type { PaletteItem } from "./paletteTypes";

const MIN_QUERY = 2;
const REMOTE_HITS_PER_HOST = 8;
const FILE_HITS = 40;

const SOURCE_SCORE: Record<SessionSearchResult["match_source"], number> = {
  title: 700,
  id: 450,
  cwd: 400,
  message: 300,
};

export type SearchPhase = "idle" | "searching" | "indexing" | "error" | "ready";

export interface PaletteSearch {
  sessionHits: PaletteItem[];
  sessionPhase: SearchPhase;
  sessionError: string | null;
  remoteHostsSearching: number;
  fileItems: PaletteItem[];
  filesHeader: string;
  filesPhase: SearchPhase;
  filesEmpty: FilesEmptyState;
  retry: () => void;
}

function phaseFor(args: { active: boolean; waiting: boolean; error: unknown }): SearchPhase {
  if (!args.active) return "idle";
  if (args.error) return args.error instanceof ApiError && args.error.status === 503 ? "indexing" : "error";
  return args.waiting ? "searching" : "ready";
}

function combineRemote(results: UseQueryResult<SessionSearchResult[]>[]) {
  return {
    data: results.map((r) => r.data),
    fetching: results.filter((r) => r.isFetching).length,
    refetch: () => results.forEach((r) => void r.refetch()),
  };
}

function basename(path: string): string {
  const parts = path.split("/");
  return parts[parts.length - 1] || path;
}

function dirname(path: string): string {
  const index = path.lastIndexOf("/");
  return index > 0 ? path.slice(0, index) : "";
}

export function usePaletteSearch(options: {
  query: string;
  visible: boolean;
  searchSessionsEnabled: boolean;
  searchFilesEnabled: boolean;
  isCompact: boolean;
  activeSessionId: string | null;
  sessions: SessionRow[] | undefined;
  close: () => void;
}): PaletteSearch {
  const { query, visible, searchSessionsEnabled, searchFilesEnabled, isCompact, activeSessionId, sessions, close } = options;
  const tokens = useTokens();
  const toast = useToast();
  const workbench = useWorkbench();
  const { baseUrl, setActive } = useAuth();
  const { signedIn } = useAnywhere();
  const { hosts } = useAnywhereHosts();
  const q = query.trim();
  const longEnough = q.length >= MIN_QUERY;

  const local = useSessionSearch(searchSessionsEnabled ? q : "", 40);

  const remoteHosts = useMemo(
    () =>
      signedIn && searchSessionsEnabled && visible && longEnough
        ? hosts.filter((host) => host.state.kind === "online" && `fany://${host.id}` !== baseUrl)
        : [],
    [signedIn, searchSessionsEnabled, visible, longEnough, hosts, baseUrl],
  );
  const remote = useQueries({
    queries: remoteHosts.map((host) => ({
      queryKey: ["palette", "host-search", host.id, q],
      queryFn: () => searchSessions(`fany://${host.id}`, q, REMOTE_HITS_PER_HOST),
      staleTime: 15_000,
      retry: false,
    })),
    combine: combineRemote,
  });

  const liveById = useMemo(() => new Map((sessions ?? []).map((row) => [row.id, row])), [sessions]);

  const hitItem = useCallback(
    (match: SessionSearchResult, host: { id: string; name: string } | null): PaletteItem => {
      const live = host ? undefined : liveById.get(match.id);
      const title = match.title || `session ${match.id.slice(0, 8)}`;
      const age = formatRelativeTime(match.last_activity * 1000);
      const excerpt = match.match_excerpt?.replace(/\s+/g, " ").trim();
      const base = `${formatCwd(match.cwd)} · ${age}`;
      const jumpSeq = match.match_source === "message" ? match.match_seq : null;
      const open = (id: string) => {
        if (jumpSeq != null) requestMessageJump(id, jumpSeq);
        openSession(id);
      };
      return {
        id: host ? `hit:${host.id}:${match.id}` : `hit:${match.id}`,
        section: "sessions",
        sessionId: host ? undefined : match.id,
        title,
        subtitle: excerpt ? `${base} — ${excerpt}` : base,
        keywords: `${match.id} ${match.cwd}`,
        highlight: true,
        fixedScore: SOURCE_SCORE[match.match_source],
        priority: live?.waiting,
        recency: match.last_activity,
        leading: <StatusDot state={live ? liveSessionState(live) : match.running ? "idle" : "done"} />,
        trailing: host ? <Badge label={host.name.toUpperCase()} tone="neutral" /> : undefined,
        onSelect: () => {
          close();
          if (host) setActive(`anywhere:${host.id}`);
          if (match.running) {
            open(match.id);
            return;
          }
          createSession(host ? `fany://${host.id}` : (baseUrl as string), { resume: match.id })
            .then((res) => open(res.id))
            .catch((err: unknown) => toast.show(err instanceof ApiError ? err.message : "couldn't open session", { tone: "danger" }));
        },
      };
    },
    [liveById, close, setActive, baseUrl, toast],
  );

  const sessionHits = useMemo<PaletteItem[]>(() => {
    const items = (local.data ?? []).map((match) => hitItem(match, null));
    remoteHosts.forEach((host, index) => {
      for (const match of remote.data[index] ?? []) items.push(hitItem(match, host));
    });
    return items;
  }, [local.data, hitItem, remoteHosts, remote.data]);

  const sessionPhase = phaseFor({
    active: searchSessionsEnabled && longEnough,
    waiting: local.isDebouncing || local.isFetching,
    error: local.isError ? local.error : null,
  });

  const filesSessionId = activeSessionId;
  const filesRow = filesSessionId ? liveById.get(filesSessionId) : undefined;
  const gitSessionId =
    searchFilesEnabled && filesSessionId && baseUrl && supportsDirectDaemonEndpoints(baseUrl) ? filesSessionId : null;
  const git = useGitStatus(gitSessionId);
  const fileSearch = useWorkspaceSearch(searchFilesEnabled && longEnough ? filesSessionId : null, q, "files", FILE_HITS);

  const openFile = useCallback(
    (path: string) => {
      close();
      if (!filesSessionId) return;
      if (isCompact) {
        router.push(`/session/${filesSessionId}/files`);
        return;
      }
      workbench.openSurface({ kind: "files", sessionId: filesSessionId, resourceId: path, title: basename(path) });
    },
    [close, filesSessionId, isCompact, workbench],
  );

  const fileItems = useMemo<PaletteItem[]>(
    () =>
      (fileSearch.data?.results ?? [])
        .filter((result) => result.kind === "file")
        .map((result) => ({
          id: `file:${result.path}`,
          section: "files" as const,
          title: basename(result.path),
          subtitle: dirname(result.path) || undefined,
          highlight: true,
          fixedScore: 500,
          leading: <FileText size={18} strokeWidth={1.75} color={tokens.ink2} />,
          onSelect: () => openFile(result.path),
        })),
    [fileSearch.data, tokens, openFile],
  );

  const filesActive = searchFilesEnabled && !!filesSessionId && longEnough;
  const filesPhase = phaseFor({
    active: filesActive,
    waiting: fileSearch.isFetching,
    error: fileSearch.isError ? fileSearch.error : null,
  });

  const retry = useCallback(() => {
    void local.refetch();
    void fileSearch.refetch();
    remote.refetch();
  }, [local, fileSearch, remote]);

  return {
    sessionHits,
    sessionPhase,
    sessionError: local.isError ? (local.error instanceof ApiError ? local.error.message : "Couldn't search session history") : null,
    remoteHostsSearching: remote.fetching,
    fileItems,
    filesHeader: filesHeader(filesRow ? formatCwd(filesRow.cwd) : null, git.data?.branch ?? null),
    filesPhase,
    filesEmpty: filesEmptyState(!!filesSessionId, q),
    retry,
  };
}
