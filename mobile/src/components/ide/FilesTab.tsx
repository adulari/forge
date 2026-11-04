// The existing workspace tree, with the files an agent changed picked out by colour.
import { FolderOpen } from "lucide-react-native";
import React, { useCallback, useMemo, useState } from "react";
import { ScrollView } from "react-native";

import { EmptyState } from "../ds/EmptyState";
import { TreeLevel, type FileTreeProps } from "../workspace/FileTree";
import { loadExpanded, saveExpanded, toggleExpanded } from "../workspace/treeState";
import { useIde } from "./IdeContext";

export function FilesTab() {
  const { panelSessionId, changes, openFile, active } = useIde();
  if (!panelSessionId) return <EmptyState icon={FolderOpen} message="Pick an agent to browse its files." />;
  return <Tree key={panelSessionId} sessionId={panelSessionId} changes={changes} openFile={openFile} activePath={active?.kind === "file" ? active.path : null} />;
}

function Tree({
  sessionId,
  changes,
  openFile,
  activePath,
}: {
  sessionId: string;
  changes: ReturnType<typeof useIde>["changes"];
  openFile: ReturnType<typeof useIde>["openFile"];
  activePath: string | null;
}) {
  const [expanded, setExpanded] = useState(() => loadExpanded(sessionId));
  const [selected, setSelected] = useState<string | null>(null);
  const changedPaths = useMemo(() => new Set(changes.get(sessionId)?.files.map((file) => file.path) ?? []), [changes, sessionId]);
  const onToggle = useCallback(
    (dir: string) =>
      setExpanded((current) => {
        const next = toggleExpanded(current, dir);
        saveExpanded(sessionId, next);
        return next;
      }),
    [sessionId],
  );
  const props = useMemo<FileTreeProps>(
    () => ({
      sessionId,
      expanded,
      selectedPath: selected ?? activePath,
      highlightPath: null,
      onToggle,
      onSelect: (entry) => setSelected(entry.path),
      onOpenFile: (entry, pinned) => openFile(sessionId, entry.path, pinned),
      onRename: null,
      onDelete: null,
      changedPaths,
    }),
    [sessionId, expanded, selected, activePath, onToggle, openFile, changedPaths],
  );
  return (
    <ScrollView>
      <TreeLevel dir="" depth={0} props={props} />
    </ScrollView>
  );
}
