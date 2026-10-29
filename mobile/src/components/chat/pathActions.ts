// What clicking a path or an image in chat does, for the session the chat belongs to. Null outside
// a session (the What's New panel, a shared replay): there a path is only text.
import { router } from "expo-router";
import { useMemo } from "react";

import { useOptionalSessionStable } from "../../lib/sessionContext";
import { workspaceImageUrl } from "../../lib/mediaApi";
import { baseName, workspaceRelative, type PathRef } from "../../lib/pathRefs";
import { useBreakpoint } from "../../theme/useBreakpoint";
import { useSessionRow } from "../shell/activeSession";
import { useOptionalWorkbench } from "../workbench/WorkbenchProvider";

export interface PathActions {
  sessionId: string;
  /** The daemon URL of a project (or evidence) image, optionally downsampled to `width`. */
  imageUrl: (path: string, width?: number) => string;
  /** Open a file in the Files dock at the reference's line (a Files screen on a phone). */
  openFile: (ref: PathRef) => void;
}

export function usePathActions(): PathActions | null {
  const session = useOptionalSessionStable();
  const workbench = useOptionalWorkbench();
  const { isExpanded } = useBreakpoint();
  const row = useSessionRow(session?.sessionId ?? null);
  const sessionId = session?.sessionId ?? null;
  const baseUrl = session?.baseUrl ?? null;
  const worktree = row?.worktree ?? null;
  const cwd = row?.cwd ?? null;

  return useMemo(() => {
    if (!sessionId || !baseUrl) return null;
    const relative = (path: string) => workspaceRelative(path, [worktree, cwd]);
    return {
      sessionId,
      imageUrl: (path, width) => workspaceImageUrl(baseUrl, sessionId, relative(path), width),
      openFile: (ref) => {
        const path = relative(ref.path);
        if (workbench && isExpanded) {
          workbench.openSurface({ kind: "files", sessionId, resourceId: path, title: baseName(path), line: ref.line });
          return;
        }
        router.push({
          pathname: `/session/${sessionId}/files`,
          params: ref.line == null ? { path } : { path, line: String(ref.line) },
        } as never);
      },
    };
  }, [baseUrl, cwd, isExpanded, sessionId, workbench, worktree]);
}
