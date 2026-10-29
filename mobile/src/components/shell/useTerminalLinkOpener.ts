import { router } from "expo-router";
import { useCallback } from "react";
import { Linking } from "react-native";

import { isTauri, isWeb } from "../../lib/platform";
import { previewLabel, setPreviewPreferences } from "../../lib/browserPreview";
import { baseName, workspaceRelative } from "../../lib/pathRefs";
import { useBreakpoint } from "../../theme/useBreakpoint";
import { createWorkbenchSurface } from "../workbench/model";
import { useOptionalWorkbench } from "../workbench/WorkbenchProvider";
import { useSessionRow } from "./activeSession";
import { hasAnySelection } from "./terminalClipboard";
import { type TerminalLinkTarget } from "./terminalLinks";

function urlKey(url: string): string {
  let hash = 0;
  for (let index = 0; index < url.length; index += 1) hash = (Math.imul(hash, 31) + url.charCodeAt(index)) >>> 0;
  return `link-${hash.toString(16)}`;
}

/**
 * What clicking a link in the terminal does: a web page opens in the Preview dock (the desktop
 * app's embedded browser) or the system browser elsewhere, and a `path:line` opens the Files dock
 * at that line, or the Files screen on a phone. Dragging across text to select it never counts as
 * a click.
 */
export function useTerminalLinkOpener(sessionId: string | null): (target: TerminalLinkTarget) => void {
  const workbench = useOptionalWorkbench();
  const { isExpanded } = useBreakpoint();
  const row = useSessionRow(sessionId);
  const worktree = row?.worktree ?? null;
  const cwd = row?.cwd ?? null;

  return useCallback(
    (target) => {
      if (hasAnySelection()) return;
      if (target.kind === "url") {
        if (isTauri && workbench && sessionId) {
          const surface = createWorkbenchSurface({
            kind: "preview",
            sessionId,
            resourceId: urlKey(target.url),
            title: "Preview",
          });
          setPreviewPreferences(previewLabel(surface.id), { url: target.url });
          workbench.openSurface({ kind: "preview", sessionId, resourceId: urlKey(target.url), title: "Preview" });
        } else if (isWeb) {
          window.open(target.url, "_blank", "noopener,noreferrer");
        } else {
          void Linking.openURL(target.url);
        }
        return;
      }
      if (!sessionId) return;
      const path = workspaceRelative(target.ref.path, [worktree, cwd]);
      const line = target.ref.line;
      if (workbench && isExpanded) {
        workbench.openSurface({ kind: "files", sessionId, resourceId: path, title: baseName(path), line });
        return;
      }
      router.push({
        pathname: `/session/${sessionId}/files`,
        params: line == null ? { path } : { path, line: String(line) },
      } as never);
    },
    [cwd, isExpanded, sessionId, workbench, worktree],
  );
}
