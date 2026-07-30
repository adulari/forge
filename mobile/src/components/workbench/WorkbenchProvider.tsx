import React, { createContext, useCallback, useContext, useMemo, useReducer } from "react";

import {
  INITIAL_WORKBENCH_STATE,
  createWorkbenchSurface,
  workbenchReducer,
  type WorkbenchPlacement,
  type WorkbenchState,
  type WorkbenchSurfaceInput,
} from "./model";

export interface WorkbenchController {
  state: WorkbenchState;
  openSurface: (input: WorkbenchSurfaceInput) => void;
  toggleSurface: (input: WorkbenchSurfaceInput) => void;
  activateSurface: (placement: WorkbenchPlacement, id: string) => void;
  closeSurface: (placement: WorkbenchPlacement, id: string) => void;
  hidePlacement: (placement: WorkbenchPlacement) => void;
}

const WorkbenchContext = createContext<WorkbenchController | null>(null);

export function WorkbenchProvider({ children }: { children: React.ReactNode }) {
  const [state, dispatch] = useReducer(workbenchReducer, INITIAL_WORKBENCH_STATE);

  const openSurface = useCallback((input: WorkbenchSurfaceInput) => {
    dispatch({ type: "open", surface: createWorkbenchSurface(input) });
  }, []);
  const toggleSurface = useCallback((input: WorkbenchSurfaceInput) => {
    dispatch({ type: "toggle", surface: createWorkbenchSurface(input) });
  }, []);
  const activateSurface = useCallback((placement: WorkbenchPlacement, id: string) => {
    dispatch({ type: "activate", placement, id });
  }, []);
  const closeSurface = useCallback((placement: WorkbenchPlacement, id: string) => {
    dispatch({ type: "close", placement, id });
  }, []);
  const hidePlacement = useCallback((placement: WorkbenchPlacement) => {
    dispatch({ type: "hide", placement });
  }, []);

  const value = useMemo(
    () => ({
      state,
      openSurface,
      toggleSurface,
      activateSurface,
      closeSurface,
      hidePlacement,
    }),
    [
      activateSurface,
      closeSurface,
      hidePlacement,
      openSurface,
      state,
      toggleSurface,
    ],
  );

  return <WorkbenchContext.Provider value={value}>{children}</WorkbenchContext.Provider>;
}

export function useWorkbench(): WorkbenchController {
  const controller = useContext(WorkbenchContext);
  if (!controller) {
    throw new Error("useWorkbench must be used inside WorkbenchProvider");
  }
  return controller;
}
