import {
  ArrowLeft,
  ChevronsDownUp,
  FileCode2,
  FilePlus,
  Folder,
  FolderPlus,
  FolderTree,
  RefreshCw,
  Save,
  SearchX,
  TextSearch,
} from "lucide-react-native";
import React, { useCallback, useEffect, useRef, useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";

import {
  type WorkspaceEntry,
  type WorkspaceFileResponse,
  type WorkspaceSearchMode,
} from "../../lib/api";
import { useAuth } from "../../lib/auth";
import {
  useCreateWorkspaceEntry,
  useDeleteWorkspaceEntry,
  useRenameWorkspaceEntry,
} from "../../lib/gitActions";
import {
  useWorkspaceFile,
  useWorkspaceSearch,
  useWriteWorkspaceFile,
} from "../../lib/queries";
import { supportsDirectDaemonEndpoints } from "../../lib/transport";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";
import { Button } from "../ds/Button";
import { ConfirmDialog } from "../ds/ConfirmDialog";
import { EmptyState } from "../ds/EmptyState";
import { IconButton } from "../ds/IconButton";
import { SearchField } from "../ds/SearchField";
import { useToast } from "../ds/ToastHost";
import { findController, findFocusProps } from "../../lib/find/findController";
import { matchOffsets, steppedIndex } from "../../lib/find/matching";
import { FindBar } from "../find/FindBar";
import { useWorkbench } from "../workbench/WorkbenchProvider";
import { CodeView } from "./CodeView";
import { TreeLevel, type FileTreeProps } from "./FileTree";
import { CAN_RENDER_SVG, MarkdownPreview, PdfFrame, SvgPreview } from "./FilePreview";
import { WorkspaceNameSheet } from "./WorkspaceNameSheet";
import {
  defaultViewMode,
  fileViewKind,
  hasRenderedPreview,
  highlightLanguage,
  type FileViewMode,
} from "./fileKind";
import { collapseWithin, creationDirectory, expandTo, loadExpanded, saveExpanded, toggleExpanded } from "./treeState";
import { joinWorkspacePath, parentWorkspacePath, workspaceBasename } from "./workspaceModel";
import { setWorkspaceDirty } from "./workspaceDirty";
import { markRevealHandled, requestReveal, useRevealRequest } from "./workspaceReveal";

interface WorkspaceDockProps {
  sessionId: string | null;
  resourceId: string | null;
  /** Line to land on when the file opens (a path chip in chat). */
  line?: number | null;
  /** A file to show first when nothing is pinned (the phone's Files screen, from a chip). */
  initialPath?: string | null;
}

type NamePrompt = { kind: "file" | "directory" } | { kind: "rename"; entry: WorkspaceEntry };

function WorkspaceBrowser({
  sessionId,
  onOpenFile,
}: {
  sessionId: string;
  onOpenFile: (path: string, pinned: boolean) => void;
}) {
  const tokens = useTokens();
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(() => loadExpanded(sessionId));
  useEffect(() => saveExpanded(sessionId, expanded), [sessionId, expanded]);
  const [selected, setSelected] = useState<{ path: string; isDirectory: boolean } | null>(null);
  const [highlightPath, setHighlightPath] = useState<string | null>(null);
  const [searchText, setSearchText] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const [mode, setMode] = useState<WorkspaceSearchMode>("files");
  const search = useWorkspaceSearch(sessionId, searchQuery, mode);
  const searching = searchQuery.trim().length > 0;
  const { baseUrl } = useAuth();
  const toast = useToast();
  const canEdit = baseUrl != null && supportsDirectDaemonEndpoints(baseUrl);
  const create = useCreateWorkspaceEntry();
  const rename = useRenameWorkspaceEntry();
  const remove = useDeleteWorkspaceEntry();
  const [prompt, setPrompt] = useState<NamePrompt | null>(null);
  const [deleting, setDeleting] = useState<WorkspaceEntry | null>(null);
  const promptBusy = create.isPending || rename.isPending;
  const promptError = (create.error ?? rename.error)?.message ?? null;
  const createDir = creationDirectory(selected);
  const reveal = useRevealRequest(sessionId);

  useEffect(() => {
    if (!reveal) return;
    markRevealHandled(reveal);
    setSearchText("");
    setSearchQuery("");
    setExpanded((current) => expandTo(current, reveal.path));
    setHighlightPath(reveal.path);
    setSelected({ path: reveal.path, isDirectory: false });
  }, [reveal]);

  const closePrompt = () => {
    setPrompt(null);
    create.reset();
    rename.reset();
  };
  const submitPrompt = async (value: string) => {
    if (!prompt) return;
    try {
      if (prompt.kind === "rename") {
        const target = joinWorkspacePath(parentWorkspacePath(prompt.entry.path), value);
        const result = await rename.mutateAsync({ session: sessionId, from: prompt.entry.path, to: target });
        toast.show(`Renamed to ${workspaceBasename(result.path)}`, { tone: "success" });
      } else {
        const target = joinWorkspacePath(createDir, value);
        const result = await create.mutateAsync({ session: sessionId, path: target, kind: prompt.kind });
        toast.show(`Created ${workspaceBasename(result.path)}`, { tone: "success" });
        setExpanded((current) => expandTo(current, result.path));
        if (prompt.kind === "file") onOpenFile(result.path, true);
      }
      closePrompt();
    } catch {
      // The daemon's message renders under the field.
    }
  };
  const confirmDelete = async () => {
    const entry = deleting;
    setDeleting(null);
    if (!entry) return;
    try {
      await remove.mutateAsync({ session: sessionId, path: entry.path });
      toast.show(`Deleted ${entry.name}`, { tone: "neutral" });
    } catch (error) {
      toast.show(error instanceof Error ? error.message : "Could not delete.", { tone: "danger" });
    }
  };

  const tree: FileTreeProps = {
    sessionId,
    expanded,
    selectedPath: selected?.path ?? null,
    highlightPath,
    onToggle: (dir) =>
      setExpanded((current) => (current.has(dir) ? collapseWithin(current, dir) : toggleExpanded(current, dir))),
    onSelect: (entry) => {
      setHighlightPath(null);
      setSelected({ path: entry.path, isDirectory: entry.kind === "directory" });
    },
    onOpenFile: (entry, pinned) => onOpenFile(entry.path, pinned),
    onRename: canEdit ? (target) => setPrompt({ kind: "rename", entry: target }) : null,
    onDelete: canEdit ? setDeleting : null,
  };

  const promptCopy =
    prompt?.kind === "rename"
      ? {
          title: `Rename ${prompt.entry.name}`,
          label: "New name",
          initial: prompt.entry.name,
          confirm: "Rename",
          description: undefined,
        }
      : {
          title: prompt?.kind === "directory" ? "New folder" : "New file",
          label: prompt?.kind === "directory" ? "Folder name" : "File name",
          initial: "",
          confirm: "Create",
          description: createDir ? `Created inside ${createDir}. Slashes create nested folders.` : "Created at the workspace root. Slashes create nested folders.",
        };

  return (
    <View style={styles.root}>
      <WorkspaceNameSheet
        visible={prompt != null}
        title={promptCopy.title}
        description={promptCopy.description}
        label={promptCopy.label}
        initialValue={promptCopy.initial}
        confirmLabel={promptCopy.confirm}
        busy={promptBusy}
        error={promptError}
        onSubmit={(value) => void submitPrompt(value)}
        onClose={closePrompt}
      />
      <ConfirmDialog
        visible={deleting != null}
        title={`Delete ${deleting?.name ?? ""}?`}
        message={
          deleting?.kind === "directory"
            ? "The folder and everything in it is removed from disk. This cannot be undone."
            : "The file is removed from disk. This cannot be undone."
        }
        confirmLabel="Delete"
        destructive
        loading={remove.isPending}
        onConfirm={() => void confirmDelete()}
        onCancel={() => setDeleting(null)}
      />
      <View style={[styles.browserHeader, { borderBottomColor: tokens.border }]}>
        <View style={styles.pathRow}>
          <Text style={[typeScale.monoMeta, styles.path, { color: tokens.ink2 }]} numberOfLines={1}>
            {selected ? createDir || "workspace" : "workspace"}
          </Text>
          <IconButton
            icon={<ChevronsDownUp size={16} color={expanded.size > 0 ? tokens.ink2 : tokens.ink4} />}
            onPress={() => setExpanded(new Set())}
            disabled={expanded.size === 0}
            accessibilityLabel="Collapse all folders"
            style={styles.compactButton}
          />
          {canEdit ? (
            <>
              <IconButton
                icon={<FilePlus size={16} color={tokens.ink2} />}
                onPress={() => setPrompt({ kind: "file" })}
                accessibilityLabel="New file"
                style={styles.compactButton}
              />
              <IconButton
                icon={<FolderPlus size={16} color={tokens.ink2} />}
                onPress={() => setPrompt({ kind: "directory" })}
                accessibilityLabel="New folder"
                style={styles.compactButton}
              />
            </>
          ) : null}
        </View>
        <SearchField
          value={searchText}
          onChangeText={setSearchText}
          onDebouncedChange={setSearchQuery}
          onCancel={() => setSearchQuery("")}
          placeholder={mode === "files" ? "Find files" : "Search workspace text"}
          showCancel={false}
          debounceMs={180}
        />
        <View style={styles.modeRow}>
          {(["files", "content"] as const).map((candidate) => {
            const active = candidate === mode;
            return (
              <Pressable
                key={candidate}
                onPress={() => setMode(candidate)}
                accessibilityRole="tab"
                accessibilityState={{ selected: active }}
                style={[
                  styles.mode,
                  {
                    backgroundColor: active ? tokens.selection : "transparent",
                    borderColor: active ? tokens.accent : tokens.border,
                  },
                ]}
              >
                <Text style={[typeScale.meta, { color: active ? tokens.accent : tokens.ink3 }]}>
                  {candidate === "files" ? "Files" : "Text"}
                </Text>
              </Pressable>
            );
          })}
        </View>
      </View>

      {searching ? (
        search.isLoading ? (
          <View style={styles.center}>
            <ActivityIndicator color={tokens.accent} />
          </View>
        ) : search.error ? (
          <View style={styles.error}>
            <Text style={[typeScale.sub, { color: tokens.danger }]}>{search.error.message}</Text>
            <Button label="Retry" variant="secondary" onPress={() => void search.refetch()} />
          </View>
        ) : (
          <ScrollView contentContainerStyle={styles.list}>
            {(search.data?.results ?? []).map((result, index) => (
              <Pressable
                key={`${result.path}:${result.line ?? 0}:${index}`}
                onPress={() => onOpenFile(result.path, false)}
                accessibilityRole="button"
                accessibilityLabel={`Open ${result.path}${result.line ? ` line ${result.line}` : ""}`}
                style={({ pressed }) => [
                  styles.searchResult,
                  { backgroundColor: pressed ? tokens.bg3 : "transparent" },
                ]}
              >
                {result.kind === "match" ? (
                  <TextSearch size={16} color={tokens.accent} />
                ) : (
                  <FileCode2 size={16} color={tokens.ink3} />
                )}
                <View style={styles.searchResultText}>
                  <Text style={[typeScale.body, { color: tokens.ink }]} numberOfLines={1}>
                    {result.path}
                    {result.line ? `:${result.line}` : ""}
                  </Text>
                  {result.preview ? (
                    <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]} numberOfLines={2}>
                      {result.preview}
                    </Text>
                  ) : null}
                </View>
              </Pressable>
            ))}
            {search.data?.results.length === 0 ? (
              <EmptyState icon={SearchX} message={`No ${mode === "files" ? "files" : "text"} match “${searchQuery}”.`} />
            ) : null}
            {search.data?.truncated ? (
              <Text style={[typeScale.meta, styles.truncated, { color: tokens.ink3 }]}>
                showing the first results from {search.data.scanned_files.toLocaleString()} files
              </Text>
            ) : null}
          </ScrollView>
        )
      ) : (
        <ScrollView contentContainerStyle={styles.list} accessibilityLabel="File tree">
          <TreeLevel dir="" depth={0} props={tree} />
        </ScrollView>
      )}
    </View>
  );
}

const VIEW_MODES: readonly { mode: FileViewMode; label: string }[] = [
  { mode: "edit", label: "Edit" },
  { mode: "code", label: "Code" },
  { mode: "preview", label: "Preview" },
];

function WorkspaceEditor({
  sessionId,
  path,
  line,
  onBack,
  onReveal,
  onFirstEdit,
}: {
  sessionId: string;
  path: string;
  line: number | null;
  onBack?: () => void;
  onReveal: (path: string) => void;
  /** The first unsaved edit: a preview tab becomes a real one. */
  onFirstEdit?: () => void;
}) {
  const tokens = useTokens();
  const toast = useToast();
  const file = useWorkspaceFile(sessionId, path);
  const write = useWriteWorkspaceFile();
  const [draft, setDraft] = useState("");
  const [loadedHash, setLoadedHash] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);
  const kind = fileViewKind(path);
  const [viewMode, setViewMode] = useState<FileViewMode>(() => defaultViewMode(kind, line, CAN_RENDER_SVG));
  const loadedHashRef = useRef<string | null>(null);
  const firstEditRef = useRef(onFirstEdit);
  useEffect(() => {
    firstEditRef.current = onFirstEdit;
  });
  const inputRef = useRef<TextInput>(null);
  const [landing, setLanding] = useState<{ start: number; end: number } | null>(null);

  const draftRef = useRef(draft);
  useEffect(() => {
    draftRef.current = draft;
  });
  useEffect(() => {
    let hits: number[] = [];
    let width = 0;
    let index = -1;
    const activate = (next: number) => {
      index = next;
      const at = hits[next];
      if (at === undefined) return { count: hits.length, index };
      const line = draftRef.current.slice(0, at).split("\n").length;
      const range = { start: at, end: at + width };
      setLanding(range);
      revealLine(inputRef.current, range, line, false);
      return { count: hits.length, index, detail: `line ${line}` };
    };
    return findController.register({
      id: "editor",
      label: "file",
      search(query, caseSensitive) {
        hits = matchOffsets(draftRef.current, query, caseSensitive);
        width = query.length;
        return activate(hits.length > 0 ? 0 : -1);
      },
      step: (direction) => activate(steppedIndex(index, hits.length, direction)),
      clear() {
        hits = [];
        index = -1;
        setLanding(null);
      },
    });
  }, []);

  const loadedContent = file.data?.content ?? null;
  // The daemon may resolve the asked-for path elsewhere (`SOLVER.md` → `notes/SOLVER.md`); saves
  // go to where the file really is.
  const actualPath = file.data?.path ?? path;
  const readOnly = file.data?.read_only === true;
  useEffect(() => {
    if (line == null || loadedContent == null) return;
    const range = lineRange(loadedContent, line);
    setLanding(range);
    revealLine(inputRef.current, range, line);
  }, [line, loadedContent, path]);

  useEffect(() => {
    setWorkspaceDirty(sessionId, path, dirty);
    if (dirty) firstEditRef.current?.();
  }, [sessionId, path, dirty]);
  useEffect(() => () => setWorkspaceDirty(sessionId, path, false), [sessionId, path]);
  useEffect(() => {
    if (line != null) setViewMode("edit");
  }, [line]);

  const loadResponse = useCallback((response: WorkspaceFileResponse) => {
    setDraft(response.content);
    setLoadedHash(response.hash);
    loadedHashRef.current = response.hash;
    setDirty(false);
  }, []);

  useEffect(() => {
    if (file.data && file.data.hash !== loadedHashRef.current) loadResponse(file.data);
  }, [file.data, loadResponse]);

  const reload = async () => {
    const result = await file.refetch();
    if (result.data) loadResponse(result.data);
  };

  const save = async () => {
    if (!loadedHash || !dirty || readOnly) return;
    try {
      const saved = await write.mutateAsync({
        session: sessionId,
        path: actualPath,
        content: draft,
        expected_hash: loadedHash,
      });
      loadResponse(saved);
      toast.show(`Saved ${workspaceBasename(actualPath)}`, { tone: "success" });
    } catch {
      // The mutation's concrete daemon message renders below the toolbar.
    }
  };

  if (file.isLoading && !file.data) {
    return (
      <View style={styles.center}>
        <ActivityIndicator color={tokens.accent} />
      </View>
    );
  }
  if (file.error && !file.data) {
    return (
      <View style={styles.error}>
        <Text style={[typeScale.sub, { color: tokens.danger }]}>{file.error.message}</Text>
        <Button label="Retry" variant="secondary" onPress={() => void reload()} />
      </View>
    );
  }

  const lineCount = draft.length === 0 ? 0 : draft.split("\n").length;
  const rendered = hasRenderedPreview(kind, CAN_RENDER_SVG);
  const modes = VIEW_MODES.filter((option) => option.mode !== "preview" || rendered);
  const saveOnShortcut = (event: { key?: string; metaKey?: boolean; ctrlKey?: boolean; preventDefault?: () => void }) => {
    if ((event.metaKey || event.ctrlKey) && event.key?.toLowerCase() === "s") {
      event.preventDefault?.();
      void save();
    }
  };
  // react-native-web hands keydown to the DOM node; native has no such event and ignores the prop.
  // The text field stops keydown from bubbling, so it takes the same handler as onKeyPress.
  const saveShortcut = { onKeyDown: saveOnShortcut } as object;
  return (
    <View style={styles.root} {...findFocusProps("editor")} {...saveShortcut}>
      <FindBar targetId="editor" />
      <View style={[styles.editorToolbar, { borderBottomColor: tokens.border }]}>
        {onBack ? (
          <IconButton
            icon={<ArrowLeft size={17} color={tokens.ink2} />}
            onPress={onBack}
            accessibilityLabel="Back to workspace files"
            style={styles.compactButton}
          />
        ) : null}
        <View style={styles.editorMeta}>
          <Text style={[typeScale.monoMeta, { color: tokens.ink2 }]} numberOfLines={1}>
            {actualPath}
          </Text>
          <Text style={[typeScale.meta, { color: tokens.ink4 }]}>
            {lineCount.toLocaleString()} lines · {draft.length.toLocaleString()} chars
            {line != null ? ` · line ${line}` : ""}
            {readOnly ? " · read only, outside this workspace" : dirty ? " · modified" : ""}
          </Text>
        </View>
        <View style={styles.modeRow} accessibilityRole="tablist">
          {modes.map((option) => {
            const active = option.mode === viewMode;
            return (
              <Pressable
                key={option.mode}
                onPress={() => setViewMode(option.mode)}
                accessibilityRole="tab"
                accessibilityState={{ selected: active }}
                accessibilityLabel={`${option.label} view`}
                style={[
                  styles.mode,
                  {
                    minWidth: 0,
                    backgroundColor: active ? tokens.selection : "transparent",
                    borderColor: active ? tokens.accent : tokens.border,
                  },
                ]}
              >
                <Text style={[typeScale.meta, { color: active ? tokens.accent : tokens.ink3 }]}>{option.label}</Text>
              </Pressable>
            );
          })}
        </View>
        <IconButton
          icon={<FolderTree size={16} color={tokens.ink2} />}
          onPress={() => onReveal(actualPath)}
          accessibilityLabel="Reveal in file tree"
          style={styles.compactButton}
        />
        <IconButton
          icon={<RefreshCw size={16} color={tokens.ink2} />}
          onPress={() => void reload()}
          disabled={file.isFetching || write.isPending}
          accessibilityLabel="Reload file from disk"
          style={styles.compactButton}
        />
        {readOnly ? null : (
          <IconButton
            icon={<Save size={17} color={dirty ? tokens.accent : tokens.ink4} />}
            onPress={() => void save()}
            disabled={!dirty || write.isPending}
            accessibilityLabel="Save file"
            style={styles.compactButton}
          />
        )}
      </View>
      {write.error ? (
        <View style={[styles.saveError, { backgroundColor: tokens.dangerBg }]}>
          <Text style={[typeScale.meta, { color: tokens.danger }]}>{write.error.message}</Text>
        </View>
      ) : null}
      {viewMode === "code" ? (
        <CodeView code={draft} language={highlightLanguage(actualPath)} />
      ) : viewMode === "preview" && kind === "markdown" ? (
        <MarkdownPreview content={draft} />
      ) : viewMode === "preview" && kind === "svg" ? (
        <SvgPreview content={draft} />
      ) : null}
      <TextInput
        ref={inputRef}
        value={draft}
        selection={landing ?? undefined}
        onSelectionChange={() => setLanding(null)}
        onKeyPress={(event) => saveOnShortcut(event as unknown as Parameters<typeof saveOnShortcut>[0])}
        onChangeText={(text) => {
          setDraft(text);
          setDirty(text !== file.data?.content);
          write.reset();
        }}
        multiline
        autoCapitalize="none"
        autoCorrect={false}
        spellCheck={false}
        editable={!write.isPending && !readOnly}
        textAlignVertical="top"
        selectionColor={tokens.accent}
        accessibilityLabel={`Edit ${path}`}
        style={[
          styles.editor,
          {
            color: tokens.ink,
            backgroundColor: tokens.bg0,
          },
          viewMode !== "edit" && styles.hiddenEditor,
        ]}
      />
    </View>
  );
}

const EDITOR_LINE_HEIGHT = 18;

/** The character range of 1-based `line` in `text`, clamped to the last line. */
export function lineRange(text: string, line: number): { start: number; end: number } {
  const lines = text.split("\n");
  const index = Math.min(Math.max(line, 1), lines.length) - 1;
  let start = 0;
  for (let i = 0; i < index; i++) start += lines[i].length + 1;
  return { start, end: start + lines[index].length };
}

/** Select the line and scroll it into view. The textarea on web needs the scroll set by hand;
 * on a phone the platform scrolls a controlled selection into view itself. */
function revealLine(input: TextInput | null, range: { start: number; end: number }, line: number, focus = true) {
  const node = input as unknown as { scrollTop?: number; focus?: () => void; setSelectionRange?: (a: number, b: number) => void } | null;
  if (!node || typeof node.scrollTop !== "number") return;
  node.scrollTop = Math.max(0, (line - 4) * EDITOR_LINE_HEIGHT);
  if (focus) node.focus?.();
  node.setSelectionRange?.(range.start, range.end);
}

function WorkspacePdf({
  sessionId,
  path,
  onBack,
  onReveal,
}: {
  sessionId: string;
  path: string;
  onBack?: () => void;
  onReveal: (path: string) => void;
}) {
  const tokens = useTokens();
  return (
    <View style={styles.root}>
      <View style={[styles.editorToolbar, { borderBottomColor: tokens.border }]}>
        {onBack ? (
          <IconButton
            icon={<ArrowLeft size={17} color={tokens.ink2} />}
            onPress={onBack}
            accessibilityLabel="Back to workspace files"
            style={styles.compactButton}
          />
        ) : null}
        <View style={styles.editorMeta}>
          <Text style={[typeScale.monoMeta, { color: tokens.ink2 }]} numberOfLines={1}>
            {path}
          </Text>
          <Text style={[typeScale.meta, { color: tokens.ink4 }]}>PDF · read only</Text>
        </View>
        <IconButton
          icon={<FolderTree size={16} color={tokens.ink2} />}
          onPress={() => onReveal(path)}
          accessibilityLabel="Reveal in file tree"
          style={styles.compactButton}
        />
      </View>
      <PdfFrame sessionId={sessionId} path={path} />
    </View>
  );
}

export function WorkspaceDock({ sessionId, resourceId, line = null, initialPath = null }: WorkspaceDockProps) {
  const { isExpanded } = useBreakpoint();
  const workbench = useWorkbench();
  const [localResource, setLocalResource] = useState<string | null>(initialPath);

  if (!sessionId) {
    return <EmptyState icon={Folder} message="Open a session to browse its workspace." />;
  }
  const activeResource = resourceId ?? localResource;
  const openFile = (path: string, pinned: boolean) => {
    if (!isExpanded) {
      setLocalResource(path);
      return;
    }
    workbench.openSurface({
      kind: "files",
      sessionId,
      resourceId: path,
      title: workspaceBasename(path),
      preview: !pinned,
    });
  };
  const reveal = (path: string) => {
    requestReveal(sessionId, path);
    if (isExpanded) workbench.openSurface({ kind: "files", sessionId });
    else setLocalResource(null);
  };
  const pinThisTab = () => {
    const tab = workbench.state.right.tabs.find(
      (candidate) => candidate.kind === "files" && candidate.sessionId === sessionId && candidate.resourceId === resourceId,
    );
    if (tab?.preview) workbench.pinSurface("right", tab.id);
  };

  if (activeResource && fileViewKind(activeResource) === "pdf") {
    return (
      <WorkspacePdf
        sessionId={sessionId}
        path={activeResource}
        onBack={!resourceId ? () => setLocalResource(null) : undefined}
        onReveal={reveal}
      />
    );
  }

  return activeResource ? (
    <WorkspaceEditor
      key={`${sessionId}:${activeResource}`}
      sessionId={sessionId}
      path={activeResource}
      line={line}
      onBack={!resourceId ? () => setLocalResource(null) : undefined}
      onReveal={reveal}
      onFirstEdit={resourceId ? pinThisTab : undefined}
    />
  ) : (
    <WorkspaceBrowser sessionId={sessionId} onOpenFile={openFile} />
  );
}

const styles = StyleSheet.create({
  root: { flex: 1, minHeight: 0 },
  center: { flex: 1, alignItems: "center", justifyContent: "center" },
  browserHeader: {
    flexShrink: 0,
    gap: space.space8,
    padding: space.space12,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  pathRow: { flexDirection: "row", alignItems: "center", gap: space.space4 },
  path: { flex: 1 },
  compactButton: { width: 32, height: 32 },
  modeRow: { flexDirection: "row", gap: space.space8 },
  mode: {
    minWidth: 64,
    alignItems: "center",
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius4,
    paddingHorizontal: space.space8,
    paddingVertical: 4,
  },
  list: { paddingVertical: space.space8 },
  entryRow: { flexDirection: "row", alignItems: "center", paddingRight: space.space8 },
  entry: {
    flex: 1,
    minHeight: 36,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space12,
    paddingVertical: 6,
  },
  entryName: { flex: 1 },
  disabled: { opacity: 0.5 },
  searchResult: {
    minHeight: 44,
    flexDirection: "row",
    alignItems: "flex-start",
    gap: space.space8,
    paddingHorizontal: space.space12,
    paddingVertical: space.space8,
  },
  searchResultText: { flex: 1, gap: 2 },
  truncated: { paddingHorizontal: space.space12, paddingVertical: space.space8 },
  error: { flex: 1, justifyContent: "center", padding: space.space16, gap: space.space12 },
  editorToolbar: {
    minHeight: 48,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space4,
    paddingHorizontal: space.space12,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  editorMeta: { flex: 1, minWidth: 0 },
  saveError: { paddingHorizontal: space.space12, paddingVertical: space.space8 },
  hiddenEditor: { display: "none" },
  editor: {
    flex: 1,
    padding: space.space12,
    fontFamily: monoFamily.regular,
    fontSize: 12,
    lineHeight: 18,
  },
});
