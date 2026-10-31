// "Start a Home session": a session whose cwd is the daemon's Home scope (`~/.forge/home`) with
// no worktree, for chats and scratch work that belong to no repository. Reachable from the
// command palette, the desktop tray and the project picker.
import { useCallback } from "react";

import { useToast } from "../components/ds/ToastHost";
import { useDesktopMenuAction } from "./desktopMenu";
import { ApiError } from "./api";
import { getHomeProject } from "./apiProjects";
import { useAuth } from "./auth";
import { openSession } from "./nav";
import { useCreateSession } from "./queries";

export function useStartHomeSession(): () => Promise<void> {
  const { baseUrl } = useAuth();
  const create = useCreateSession();
  const toast = useToast();
  return useCallback(async () => {
    if (!baseUrl) return;
    try {
      const home = await getHomeProject(baseUrl);
      const session = await create.mutateAsync({ cwd: home.path, worktree: false });
      openSession(session.id);
    } catch (error) {
      toast.show(error instanceof ApiError ? error.message : "could not start a Home session", { tone: "danger" });
    }
  }, [baseUrl, create, toast]);
}

/** Claims the tray's "New Home session" item (and the palette command that reuses it). */
export function useHomeSessionMenuAction(): void {
  const start = useStartHomeSession();
  useDesktopMenuAction("tray:home-session", () => void start());
}
