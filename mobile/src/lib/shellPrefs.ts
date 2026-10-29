// Per-viewer shell presentation that should survive a reload: which sidebar groups are folded,
// the thread sort, the sidebar width, the workspaces and which one is active, and the last
// release the What's New dot was cleared for. One JSON blob, hydrated lazily, every write guarded
// so a blocked or full store leaves the in-memory value in place instead of throwing.
import AsyncStorage from "@react-native-async-storage/async-storage";
import { useSyncExternalStore } from "react";

const STORAGE_KEY = "forge.shell.v1";

export const SIDEBAR_MIN_WIDTH = 200;
export const SIDEBAR_MAX_WIDTH = 360;
export const SIDEBAR_DEFAULT_WIDTH = 232;

export type SidebarSort = "updated" | "created";

export interface Workspace {
  id: string;
  name: string;
  /** Project names (the sidebar group labels, lowercase). */
  projects: string[];
}

export interface StoredShellPrefs {
  collapsedGroups: string[];
  sort: SidebarSort;
  sidebarWidth: number;
  workspaces: Workspace[];
  activeWorkspaceId: string | null;
  whatsNewSeen: string | null;
}

export interface ShellPrefs extends StoredShellPrefs {
  loaded: boolean;
}

const DEFAULTS: StoredShellPrefs = {
  collapsedGroups: [],
  sort: "updated",
  sidebarWidth: SIDEBAR_DEFAULT_WIDTH,
  workspaces: [],
  activeWorkspaceId: null,
  whatsNewSeen: null,
};

export const clampSidebarWidth = (width: number) =>
  Math.round(Math.max(SIDEBAR_MIN_WIDTH, Math.min(SIDEBAR_MAX_WIDTH, width)));

const isString = (value: unknown): value is string => typeof value === "string";

export function parseShellPrefs(raw: string | null): StoredShellPrefs {
  if (!raw) return { ...DEFAULTS, workspaces: [], collapsedGroups: [] };
  try {
    const parsed = JSON.parse(raw) as Partial<Record<keyof StoredShellPrefs, unknown>>;
    const workspaces = Array.isArray(parsed.workspaces)
      ? parsed.workspaces.flatMap((entry): Workspace[] => {
          const candidate = entry as Partial<Workspace> | null;
          if (!candidate || !isString(candidate.id) || !isString(candidate.name)) return [];
          const projects = Array.isArray(candidate.projects) ? candidate.projects.filter(isString) : [];
          return [{ id: candidate.id, name: candidate.name, projects }];
        })
      : [];
    const active = isString(parsed.activeWorkspaceId) ? parsed.activeWorkspaceId : null;
    return {
      collapsedGroups: Array.isArray(parsed.collapsedGroups) ? parsed.collapsedGroups.filter(isString) : [],
      sort: parsed.sort === "created" ? "created" : "updated",
      sidebarWidth: typeof parsed.sidebarWidth === "number" ? clampSidebarWidth(parsed.sidebarWidth) : SIDEBAR_DEFAULT_WIDTH,
      workspaces,
      activeWorkspaceId: active && workspaces.some((workspace) => workspace.id === active) ? active : null,
      whatsNewSeen: isString(parsed.whatsNewSeen) ? parsed.whatsNewSeen : null,
    };
  } catch {
    return { ...DEFAULTS, workspaces: [], collapsedGroups: [] };
  }
}

let state: ShellPrefs = { loaded: false, ...DEFAULTS };
let hydration: Promise<void> | null = null;
const listeners = new Set<() => void>();

function replaceState(next: ShellPrefs): void {
  state = next;
  for (const listener of listeners) listener();
}

function ensureLoaded(): Promise<void> {
  if (state.loaded) return Promise.resolve();
  if (hydration) return hydration;
  hydration = AsyncStorage.getItem(STORAGE_KEY)
    .then((raw) => {
      // A write made before hydration finished wins over what was on disk.
      if (!state.loaded) replaceState({ ...parseShellPrefs(raw), loaded: true });
    })
    .catch(() => {
      if (!state.loaded) replaceState({ ...state, loaded: true });
    });
  return hydration;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  void ensureLoaded();
  return () => {
    listeners.delete(listener);
  };
}

const snapshot = () => state;

export function useShellPrefs(): ShellPrefs {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

export function readShellPrefs(): ShellPrefs {
  return state;
}

function persist(): void {
  const { loaded: _loaded, ...stored } = state;
  void AsyncStorage.setItem(STORAGE_KEY, JSON.stringify(stored)).catch(() => undefined);
}

function patch(change: Partial<StoredShellPrefs>, save = true): void {
  replaceState({ ...state, ...change, loaded: true });
  if (save) persist();
}

export function toggleGroupCollapsed(key: string): void {
  const collapsed = state.collapsedGroups.includes(key)
    ? state.collapsedGroups.filter((entry) => entry !== key)
    : [...state.collapsedGroups, key];
  patch({ collapsedGroups: collapsed });
}

export const setSidebarSort = (sort: SidebarSort) => patch({ sort });
/** `save: false` follows a drag live without writing on every pointer move; the drag end saves. */
export const setSidebarWidth = (width: number, save = true) => patch({ sidebarWidth: clampSidebarWidth(width) }, save);
export const saveShellPrefs = () => persist();
export const setWhatsNewSeen = (version: string) => patch({ whatsNewSeen: version });
export const setActiveWorkspace = (id: string | null) =>
  patch({ activeWorkspaceId: id && state.workspaces.some((workspace) => workspace.id === id) ? id : null });

export function createWorkspace(name: string): string | null {
  const trimmed = name.trim();
  if (!trimmed) return null;
  const id = `ws-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
  patch({ workspaces: [...state.workspaces, { id, name: trimmed, projects: [] }], activeWorkspaceId: id });
  return id;
}

export function deleteWorkspace(id: string): void {
  patch({
    workspaces: state.workspaces.filter((workspace) => workspace.id !== id),
    activeWorkspaceId: state.activeWorkspaceId === id ? null : state.activeWorkspaceId,
  });
}

export function toggleProjectInWorkspace(workspaceId: string, project: string): void {
  patch({
    workspaces: state.workspaces.map((workspace) => {
      if (workspace.id !== workspaceId) return workspace;
      const projects = workspace.projects.includes(project)
        ? workspace.projects.filter((entry) => entry !== project)
        : [...workspace.projects, project];
      return { ...workspace, projects };
    }),
  });
}

/** The project names the active workspace shows, or null when no workspace narrows the view. */
export function activeWorkspaceProjects(prefs: Pick<StoredShellPrefs, "workspaces" | "activeWorkspaceId">): Set<string> | null {
  const active = prefs.workspaces.find((workspace) => workspace.id === prefs.activeWorkspaceId);
  return active ? new Set(active.projects) : null;
}

export function resetShellPrefsForTest(): void {
  state = { loaded: false, ...DEFAULTS, collapsedGroups: [], workspaces: [] };
  hydration = null;
  listeners.clear();
}
