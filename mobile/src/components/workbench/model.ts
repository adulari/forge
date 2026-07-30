export const WORKBENCH_PLACEMENTS = ["right", "bottom"] as const;

export type WorkbenchPlacement = (typeof WORKBENCH_PLACEMENTS)[number];
export type WorkbenchSurfaceKind = "usage" | "git" | "files" | "terminal";

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
  terminal: {
    kind: "terminal",
    title: "Terminal",
    placement: "bottom",
    defaultSize: 190,
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
}

export interface WorkbenchSurfaceInput {
  kind: WorkbenchSurfaceKind;
  sessionId?: string | null;
  resourceId?: string | null;
  title?: string;
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
  | { type: "hide"; placement: WorkbenchPlacement };

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
  }
}
