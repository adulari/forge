export const WORKBENCH_PLACEMENTS = ["right", "bottom"] as const;

export type WorkbenchPlacement = (typeof WORKBENCH_PLACEMENTS)[number];
export type WorkbenchSurfaceKind =
  | "usage"
  | "git"
  | "files"
  | "preview"
  | "terminal"
  | "tasks"
  | "agents"
  | "plan"
  | "gallery";

export interface WorkbenchSurfaceDefinition {
  kind: WorkbenchSurfaceKind;
  title: string;
  placement: WorkbenchPlacement;
  defaultSize: number;
  sessionScoped: boolean;
}

export const WORKBENCH_SURFACE_DEFINITIONS: Record<
  WorkbenchSurfaceKind,
  WorkbenchSurfaceDefinition
> = {
  usage: {
    kind: "usage",
    title: "Usage",
    placement: "right",
    defaultSize: 288,
    sessionScoped: false,
  },
  git: {
    kind: "git",
    title: "Git review",
    placement: "right",
    defaultSize: 420,
    sessionScoped: true,
  },
  files: {
    kind: "files",
    title: "Files",
    placement: "right",
    defaultSize: 560,
    sessionScoped: true,
  },
  preview: {
    kind: "preview",
    title: "Browser preview",
    placement: "right",
    defaultSize: 680,
    sessionScoped: true,
  },
  terminal: {
    kind: "terminal",
    title: "Terminal",
    placement: "bottom",
    defaultSize: 190,
    sessionScoped: true,
  },
  tasks: {
    kind: "tasks",
    title: "Tasks",
    placement: "right",
    defaultSize: 320,
    sessionScoped: true,
  },
  agents: {
    kind: "agents",
    title: "Agents",
    placement: "right",
    defaultSize: 340,
    sessionScoped: true,
  },
  plan: {
    kind: "plan",
    title: "Plan",
    placement: "right",
    defaultSize: 340,
    sessionScoped: true,
  },
  gallery: {
    kind: "gallery",
    title: "Gallery",
    placement: "right",
    defaultSize: 360,
    sessionScoped: true,
  },
};

/**
 * A surface is a concrete workbench tab. `resourceId` lets later panels keep several
 * resources of one kind open (files, previews, terminals) without changing the lane model.
 * Omitting `sessionId` means "follow the shell's active session".
 */
export interface WorkbenchSurface {
  id: string;
  kind: WorkbenchSurfaceKind;
  placement: WorkbenchPlacement;
  sessionId: string | null;
  resourceId: string | null;
  title: string;
  /** Where to land inside the resource (a file's line). Not part of the tab's identity, so opening
   * the same file at another line moves the open tab instead of adding one. */
  line: number | null;
}

export interface WorkbenchSurfaceInput {
  kind: WorkbenchSurfaceKind;
  sessionId?: string | null;
  resourceId?: string | null;
  title?: string;
  line?: number | null;
}

export interface WorkbenchLaneState {
  tabs: WorkbenchSurface[];
  activeId: string | null;
  visible: boolean;
}

export interface WorkbenchState {
  right: WorkbenchLaneState;
  bottom: WorkbenchLaneState;
}

export const INITIAL_WORKBENCH_STATE: WorkbenchState = {
  right: { tabs: [], activeId: null, visible: false },
  bottom: { tabs: [], activeId: null, visible: false },
};

export type WorkbenchAction =
  | { type: "open"; surface: WorkbenchSurface }
  | { type: "toggle"; surface: WorkbenchSurface }
  | { type: "activate"; placement: WorkbenchPlacement; id: string }
  | { type: "close"; placement: WorkbenchPlacement; id: string }
  | { type: "hide"; placement: WorkbenchPlacement }
  | { type: "reorder"; placement: WorkbenchPlacement; from: number; to: number }
  | { type: "replace"; state: WorkbenchState };

function idPart(value: string | null | undefined): string {
  return value == null || value === "" ? "_" : encodeURIComponent(value);
}

export function createWorkbenchSurface(input: WorkbenchSurfaceInput): WorkbenchSurface {
  const definition = WORKBENCH_SURFACE_DEFINITIONS[input.kind];
  const sessionId = input.sessionId ?? null;
  const resourceId = input.resourceId ?? null;
  return {
    id: `${input.kind}:${idPart(sessionId)}:${idPart(resourceId)}`,
    kind: input.kind,
    placement: definition.placement,
    sessionId,
    resourceId,
    title: input.title?.trim() || definition.title,
    line: input.line ?? null,
  };
}

export function activeWorkbenchSurface(
  state: WorkbenchState,
  placement: WorkbenchPlacement,
): WorkbenchSurface | null {
  const lane = state[placement];
  if (!lane.visible || lane.activeId == null) return null;
  return lane.tabs.find((surface) => surface.id === lane.activeId) ?? null;
}

function upsertLane(lane: WorkbenchLaneState, surface: WorkbenchSurface): WorkbenchLaneState {
  const index = lane.tabs.findIndex((candidate) => candidate.id === surface.id);
  const tabs =
    index === -1
      ? [...lane.tabs, surface]
      : lane.tabs.map((candidate, candidateIndex) =>
          candidateIndex === index ? surface : candidate,
        );
  return { tabs, activeId: surface.id, visible: true };
}

function closeFromLane(lane: WorkbenchLaneState, id: string): WorkbenchLaneState {
  const index = lane.tabs.findIndex((surface) => surface.id === id);
  if (index === -1) return lane;

  const tabs = lane.tabs.filter((surface) => surface.id !== id);
  if (lane.activeId !== id) return { ...lane, tabs };

  const fallback = tabs[Math.min(index, tabs.length - 1)] ?? null;
  return {
    tabs,
    activeId: fallback?.id ?? null,
    visible: lane.visible && fallback != null,
  };
}

export function workbenchReducer(
  state: WorkbenchState,
  action: WorkbenchAction,
): WorkbenchState {
  switch (action.type) {
    case "open": {
      const placement = action.surface.placement;
      return { ...state, [placement]: upsertLane(state[placement], action.surface) };
    }
    case "toggle": {
      const placement = action.surface.placement;
      const lane = state[placement];
      if (lane.visible && lane.activeId === action.surface.id) {
        return { ...state, [placement]: { ...lane, visible: false } };
      }
      return { ...state, [placement]: upsertLane(lane, action.surface) };
    }
    case "activate": {
      const lane = state[action.placement];
      if (!lane.tabs.some((surface) => surface.id === action.id)) return state;
      return {
        ...state,
        [action.placement]: { ...lane, activeId: action.id, visible: true },
      };
    }
    case "close":
      return {
        ...state,
        [action.placement]: closeFromLane(state[action.placement], action.id),
      };
    case "hide":
      return {
        ...state,
        [action.placement]: { ...state[action.placement], visible: false },
      };
    case "reorder": {
      const lane = state[action.placement];
      const { from, to } = action;
      if (from === to || from < 0 || to < 0 || from >= lane.tabs.length || to >= lane.tabs.length) return state;
      const tabs = [...lane.tabs];
      const [moved] = tabs.splice(from, 1);
      if (!moved) return state;
      tabs.splice(to, 0, moved);
      return { ...state, [action.placement]: { ...lane, tabs } };
    }
    case "replace":
      return action.state;
  }
}

/**
 * The lanes worth restoring on the next launch. Surfaces pinned to one session (a second
 * terminal, a preview) are dropped: that session may not exist any more, and a lane that
 * reopens onto a dead id is worse than one that starts closed.
 */
export function restorableWorkbenchState(state: WorkbenchState): WorkbenchState {
  const lane = (source: WorkbenchLaneState): WorkbenchLaneState => {
    const tabs = source.tabs.filter((surface) => surface.sessionId == null);
    const activeId = tabs.some((surface) => surface.id === source.activeId) ? source.activeId : (tabs[0]?.id ?? null);
    return { tabs, activeId, visible: source.visible && activeId != null };
  };
  return { right: lane(state.right), bottom: lane(state.bottom) };
}

/** Rebuild the lanes from persisted JSON, keeping only entries that name a known surface kind. */
export function parseWorkbenchState(raw: unknown): WorkbenchState {
  const parseLane = (value: unknown, placement: WorkbenchPlacement): WorkbenchLaneState => {
    if (typeof value !== "object" || value === null) return { tabs: [], activeId: null, visible: false };
    const record = value as { tabs?: unknown; activeId?: unknown; visible?: unknown };
    const tabs: WorkbenchSurface[] = [];
    if (Array.isArray(record.tabs)) {
      for (const entry of record.tabs) {
        const kind = (entry as { kind?: unknown } | null)?.kind;
        if (typeof kind !== "string" || !(kind in WORKBENCH_SURFACE_DEFINITIONS)) continue;
        const surface = createWorkbenchSurface({ kind: kind as WorkbenchSurfaceKind });
        if (surface.placement === placement && !tabs.some((tab) => tab.id === surface.id)) tabs.push(surface);
      }
    }
    const activeId = tabs.some((tab) => tab.id === record.activeId) ? (record.activeId as string) : (tabs[0]?.id ?? null);
    return { tabs, activeId, visible: record.visible === true && activeId != null };
  };
  const source = (typeof raw === "object" && raw !== null ? raw : {}) as { right?: unknown; bottom?: unknown };
  return { right: parseLane(source.right, "right"), bottom: parseLane(source.bottom, "bottom") };
}
