// The editor's tab strip as a reducer. The one rule worth testing is the preview tab:
// a single click opens a file in an italic "preview" tab that the next single click replaces;
// a double click, or any edit, pins it so it stays.

export type EditorTabKind = "diff" | "file";

export interface EditorTab {
  /** `kind:sessionId:path` — opening the same thing twice finds the same tab. */
  id: string;
  kind: EditorTabKind;
  sessionId: string;
  path: string;
  preview: boolean;
}

export interface EditorTabsState {
  tabs: readonly EditorTab[];
  activeId: string | null;
}

export const EMPTY_TABS: EditorTabsState = { tabs: [], activeId: null };

export function tabId(kind: EditorTabKind, sessionId: string, path: string): string {
  return `${kind}:${sessionId}:${path}`;
}

export type EditorTabsAction =
  | { type: "open"; kind: EditorTabKind; sessionId: string; path: string; pinned: boolean }
  | { type: "pin"; id: string }
  | { type: "activate"; id: string }
  | { type: "close"; id: string }
  | { type: "closeOthers"; id: string }
  /** The path stopped being a changed file (reverted, committed): its diff tab has nothing to show. */
  | { type: "drop"; ids: readonly string[] };

export function editorTabsReducer(state: EditorTabsState, action: EditorTabsAction): EditorTabsState {
  switch (action.type) {
    case "open": {
      const id = tabId(action.kind, action.sessionId, action.path);
      const existing = state.tabs.find((tab) => tab.id === id);
      if (existing) {
        const tabs = action.pinned && existing.preview ? state.tabs.map((tab) => (tab.id === id ? { ...tab, preview: false } : tab)) : state.tabs;
        return { tabs, activeId: id };
      }
      const created: EditorTab = {
        id,
        kind: action.kind,
        sessionId: action.sessionId,
        path: action.path,
        preview: !action.pinned,
      };
      if (action.pinned) return { tabs: [...state.tabs, created], activeId: id };
      // The preview slot is reused in place, so previewing a run of files does not shuffle the strip.
      const slot = state.tabs.findIndex((tab) => tab.preview);
      if (slot === -1) return { tabs: [...state.tabs, created], activeId: id };
      const tabs = [...state.tabs];
      tabs[slot] = created;
      return { tabs, activeId: id };
    }
    case "pin": {
      if (!state.tabs.some((tab) => tab.id === action.id && tab.preview)) return state;
      return { ...state, tabs: state.tabs.map((tab) => (tab.id === action.id ? { ...tab, preview: false } : tab)) };
    }
    case "activate":
      return state.activeId === action.id || !state.tabs.some((tab) => tab.id === action.id)
        ? state
        : { ...state, activeId: action.id };
    case "close":
      return removeTabs(state, new Set([action.id]));
    case "drop":
      return action.ids.length === 0 ? state : removeTabs(state, new Set(action.ids));
    case "closeOthers": {
      const keep = state.tabs.find((tab) => tab.id === action.id);
      return keep ? { tabs: [{ ...keep, preview: false }], activeId: keep.id } : state;
    }
  }
}

function removeTabs(state: EditorTabsState, gone: ReadonlySet<string>): EditorTabsState {
  if (!state.tabs.some((tab) => gone.has(tab.id))) return state;
  const tabs = state.tabs.filter((tab) => !gone.has(tab.id));
  if (state.activeId == null || !gone.has(state.activeId)) return { tabs, activeId: state.activeId };
  // Closing the active tab lands on its right neighbour, else the left one (what a tab strip does everywhere).
  const index = state.tabs.findIndex((tab) => tab.id === state.activeId);
  const next = tabs.find((tab) => state.tabs.indexOf(tab) > index) ?? tabs[tabs.length - 1] ?? null;
  return { tabs, activeId: next?.id ?? null };
}

export function activeTab(state: EditorTabsState): EditorTab | null {
  return state.tabs.find((tab) => tab.id === state.activeId) ?? null;
}
