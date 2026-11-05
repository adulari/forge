// IDE mode's one source of truth: which project and agent are in view, the change queue built
// from their trees, the open editor tabs, and the actions that move between them. Every pane
// reads this; none of them fetch the fleet themselves.
import { usePathname } from "expo-router";
import React, { createContext, useCallback, useContext, useEffect, useMemo, useReducer, useRef, useState } from "react";

import type { SessionRow } from "../../lib/api";
import { projectName } from "../../lib/sidebarModel";
import { routedSessionId } from "../shell/activeSession";
import {
  buildQueue,
  flattenQueue,
  isReviewed,
  nextToReview,
  stepQueue,
  type QueueAgent,
  type QueueGroup,
  type QueueRow,
  type ReviewedMarks,
} from "./changeQueueModel";
import { defaultProject, type SessionChanges } from "./changesAdapter";
import { EMPTY_TABS, activeTab, editorTabsReducer, tabId, type EditorTab, type EditorTabsAction } from "./editorTabsModel";
import { onIdeClaudeTerminal, onIdeTerminalToggle } from "./ideEvents";
import { failingPathsOf, setBottomTab, setIdeTerminalId, useFailingChecks } from "./ideSignals";
import { CLAUDE_TERMINAL_ID } from "../../lib/claudeHandoff";
import { markFileReviewed, unmarkFileReviewed, useReviewedMarks } from "./reviewedStore";
import { useIdeProjects, useProjectChanges, useProjectSessions } from "./useIdeData";

export type RailTab = "changes" | "files" | "git";

export interface IdeContextValue {
  project: string | null;
  projects: ReturnType<typeof useIdeProjects>["projects"];
  setProject: (project: string) => void;
  sessions: readonly SessionRow[];
  /** Everything the fleet knows, for the project switcher. */
  allSessions: readonly SessionRow[];
  fleetLoading: boolean;
  /** `null` is "All agents". */
  scope: string | null;
  setScope: (sessionId: string | null) => void;
  /** The agent the right-hand panel shows. */
  panelSessionId: string | null;
  panelSession: SessionRow | null;
  railTab: RailTab;
  setRailTab: (tab: RailTab) => void;
  changes: ReadonlyMap<string, SessionChanges>;
  changesLoading: boolean;
  groups: QueueGroup[];
  rows: QueueRow[];
  marks: ReviewedMarks;
  tabs: readonly EditorTab[];
  active: EditorTab | null;
  activeRow: QueueRow | null;
  dispatchTabs: (action: EditorTabsAction) => void;
  openChange: (sessionId: string, path: string, pinned?: boolean) => void;
  openFile: (sessionId: string, path: string, pinned?: boolean) => void;
  stepFile: (step: 1 | -1) => void;
  toggleReviewed: (sessionId: string, path: string, hash: string) => void;
  markActiveReviewed: () => void;
  bottomOpen: boolean;
  toggleBottom: () => void;
  composerSignal: number;
  focusComposer: () => void;
}

const IdeCtx = createContext<IdeContextValue | null>(null);

export function useIde(): IdeContextValue {
  const ctx = useContext(IdeCtx);
  if (!ctx) throw new Error("useIde must be used inside IdeProvider");
  return ctx;
}

export function IdeProvider({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();
  const routedId = routedSessionId(pathname);
  const { rows: allSessions, projects, loading: fleetLoading } = useIdeProjects();

  const [chosenProject, setChosenProject] = useState<string | null>(null);
  const fallbackProject = useMemo(() => defaultProject(allSessions, routedId), [allSessions, routedId]);
  const project =
    chosenProject && allSessions.some((row) => projectName(row.cwd) === chosenProject) ? chosenProject : fallbackProject;
  const sessions = useProjectSessions(allSessions, project);

  const [scopeState, setScopeState] = useState<string | null>(null);
  const scope = scopeState && sessions.some((row) => row.id === scopeState) ? scopeState : null;

  // Opening a session from the sidebar's world (palette, notification, deep link) lands on it here.
  const lastRouted = useRef<string | null>(null);
  useEffect(() => {
    if (!routedId || routedId === lastRouted.current) return;
    lastRouted.current = routedId;
    const row = allSessions.find((candidate) => candidate.id === routedId);
    if (!row) return;
    setChosenProject(projectName(row.cwd));
    setScopeState(routedId);
  }, [routedId, allSessions]);

  const setProject = useCallback((next: string) => {
    setChosenProject(next);
    setScopeState(null);
  }, []);

  const [railTab, setRailTab] = useState<RailTab>("changes");
  const [tabState, dispatch] = useReducer(editorTabsReducer, EMPTY_TABS);
  const [bottomOpen, setBottomOpen] = useState(false);
  const [composerSignal, setComposerSignal] = useState(0);

  const { bySession, loading: changesLoading } = useProjectChanges(sessions);
  const marks = useReviewedMarks();

  const failing = useFailingChecks();
  const agents = useMemo<QueueAgent[]>(
    () =>
      sessions.map((row) => {
        const failingPaths = failingPathsOf(failing, row.id);
        const files = bySession.get(row.id)?.files ?? [];
        return {
          sessionId: row.id,
          name: row.title?.trim() || `session ${row.id.slice(0, 8)}`,
          waiting: row.waiting,
          busy: row.busy,
          lastActivity: row.last_activity,
          files: failingPaths.size === 0 ? files : files.map((file) => (failingPaths.has(file.path) ? { ...file, failing: true } : file)),
        };
      }),
    [sessions, bySession, failing],
  );
  const groups = useMemo(() => buildQueue({ agents, marks, scope }), [agents, marks, scope]);
  const rows = useMemo(() => flattenQueue(groups), [groups]);

  const active = activeTab(tabState);
  const activeRow = useMemo(
    () => (active?.kind === "diff" ? (rows.find((row) => row.sessionId === active.sessionId && row.path === active.path) ?? null) : null),
    [active, rows],
  );

  const panelSessionId = useMemo(() => {
    if (scope) return scope;
    if (active && sessions.some((row) => row.id === active.sessionId)) return active.sessionId;
    return sessions[0]?.id ?? null;
  }, [scope, active, sessions]);
  const panelSession = useMemo(() => sessions.find((row) => row.id === panelSessionId) ?? null, [sessions, panelSessionId]);

  // A diff whose file left the change set (reverted, committed) has nothing left to show.
  useEffect(() => {
    if (changesLoading) return;
    const gone = tabState.tabs
      .filter((tab) => tab.kind === "diff")
      .filter((tab) => {
        const changed = bySession.get(tab.sessionId);
        if (!changed) return false;
        return !changed.files.some((file) => file.path === tab.path);
      })
      .map((tab) => tab.id);
    if (gone.length > 0) dispatch({ type: "drop", ids: gone });
  }, [bySession, changesLoading, tabState.tabs]);

  const openChange = useCallback((sessionId: string, path: string, pinned = false) => {
    dispatch({ type: "open", kind: "diff", sessionId, path, pinned });
  }, []);
  const openFile = useCallback((sessionId: string, path: string, pinned = false) => {
    dispatch({ type: "open", kind: "file", sessionId, path, pinned });
  }, []);

  const rowsRef = useRef(rows);
  useEffect(() => {
    rowsRef.current = rows;
  });
  const activeRowRef = useRef(activeRow);
  useEffect(() => {
    activeRowRef.current = activeRow;
  });

  const stepFile = useCallback(
    (step: 1 | -1) => {
      const next = stepQueue(rowsRef.current, activeRowRef.current?.key ?? null, step);
      if (next) openChange(next.sessionId, next.path);
    },
    [openChange],
  );

  const marksRef = useRef(marks);
  useEffect(() => {
    marksRef.current = marks;
  });
  const toggleReviewed = useCallback((sessionId: string, path: string, hash: string) => {
    if (isReviewed(marksRef.current, sessionId, path, hash)) unmarkFileReviewed(sessionId, path);
    else markFileReviewed(sessionId, path, hash);
  }, []);

  const markActiveReviewed = useCallback(() => {
    const current = activeRowRef.current;
    if (!current) return;
    if (current.tier === "reviewed") {
      unmarkFileReviewed(current.sessionId, current.path);
      return;
    }
    markFileReviewed(current.sessionId, current.path, current.file.hash);
    // Tiers are recomputed after the mark lands, so the next file is picked from the old list
    // minus this one.
    const next = nextToReview(rowsRef.current, current.key);
    if (next) openChange(next.sessionId, next.path);
  }, [openChange]);

  const toggleBottom = useCallback(() => setBottomOpen((open) => !open), []);
  const focusComposer = useCallback(() => setComposerSignal((n) => n + 1), []);
  useEffect(() => onIdeTerminalToggle(toggleBottom), [toggleBottom]);
  useEffect(
    () =>
      onIdeClaudeTerminal((sessionId) => {
        setScopeState(sessionId);
        setIdeTerminalId(CLAUDE_TERMINAL_ID);
        setBottomTab("terminal");
        setBottomOpen(true);
      }),
    [],
  );

  const value = useMemo<IdeContextValue>(
    () => ({
      project,
      projects,
      setProject,
      sessions,
      allSessions,
      fleetLoading,
      scope,
      setScope: setScopeState,
      panelSessionId,
      panelSession,
      railTab,
      setRailTab,
      changes: bySession,
      changesLoading,
      groups,
      rows,
      marks,
      tabs: tabState.tabs,
      active,
      activeRow,
      dispatchTabs: dispatch,
      openChange,
      openFile,
      stepFile,
      toggleReviewed,
      markActiveReviewed,
      bottomOpen,
      toggleBottom,
      composerSignal,
      focusComposer,
    }),
    [
      project, projects, setProject, sessions, allSessions, fleetLoading, scope, panelSessionId, panelSession, railTab,
      bySession, changesLoading, groups, rows, marks, tabState.tabs, active, activeRow, openChange, openFile, stepFile,
      toggleReviewed, markActiveReviewed, bottomOpen, toggleBottom, composerSignal, focusComposer,
    ],
  );
  return <IdeCtx.Provider value={value}>{children}</IdeCtx.Provider>;
}

export { tabId };
