import {
  ArrowLeft,
  File,
  FileCode2,
  Folder,
  Link,
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
  useWorkspaceEntries,
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
import { EmptyState } from "../ds/EmptyState";
import { IconButton } from "../ds/IconButton";
import { SearchField } from "../ds/SearchField";
import { useToast } from "../ds/ToastHost";
import { useWorkbench } from "../workbench/WorkbenchProvider";
import { parentWorkspacePath, workspaceBasename } from "./workspaceModel";

interface WorkspaceDockProps {
  sessionId: string | null;
  resourceId: string | null;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function WorkspaceEntryRow({
  entry,
  onOpen,
}: {
  entry: WorkspaceEntry;
  onOpen: (entry: WorkspaceEntry) => void;
}) {
  const tokens = useTokens();
  const Icon = entry.kind === "directory" ? Folder : entry.kind === "symlink" ? Link : File;
  return (
    <Pressable
      onPress={() => onOpen(entry)}
      disabled={entry.kind === "symlink"}
      accessibilityRole="button"
      accessibilityLabel={`${entry.kind} ${entry.path}`}
      style={({ pressed }) => [
        styles.entry,
        { backgroundColor: pressed ? tokens.bg3 : "transparent" },
        entry.kind === "symlink" && styles.disabled,
      ]}
    >
      <Icon size={16} strokeWidth={1.75} color={entry.kind === "directory" ? tokens.accent : tokens.ink3} />
      <Text style={[typeScale.body, styles.entryName, { color: tokens.ink }]} numberOfLines={1}>
        {entry.name}
      </Text>
      {entry.kind === "file" ? (
        <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>
          {formatBytes(entry.size)}
        </Text>
      ) : null}
    </Pressable>
  );
}

function WorkspaceBrowser({
  sessionId,
  onOpenFile,
}: {
  sessionId: string;
  onOpenFile: (path: string) => void;
}) {
  const tokens = useTokens();
  const [path, setPath] = useState("");
  const [searchText, setSearchText] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const [mode, setMode] = useState<WorkspaceSearchMode>("files");
  const entries = useWorkspaceEntries(sessionId, path);
  const search = useWorkspaceSearch(sessionId, searchQuery, mode);
  const searching = searchQuery.trim().length > 0;

  const openEntry = (entry: WorkspaceEntry) => {
    if (entry.kind === "directory") setPath(entry.path);
    else if (entry.kind === "file") onOpenFile(entry.path);
  };

  const error = searching ? search.error : entries.error;
  const loading = searching ? search.isLoading : entries.isLoading;

  return (
    <View style={styles.root}>
      <View style={[styles.browserHeader, { borderBottomColor: tokens.border }]}>
        <View style={styles.pathRow}>
          <IconButton
            icon={<ArrowLeft size={17} color={path ? tokens.ink2 : tokens.ink4} />}
            onPress={() => setPath(parentWorkspacePath(path))}
            disabled={!path}
            accessibilityLabel="Open parent directory"
            style={styles.compactButton}
          />
          <Text style={[typeScale.monoMeta, styles.path, { color: tokens.ink2 }]} numberOfLines={1}>
            {path || "workspace"}
          </Text>
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

      {loading ? (
        <View style={styles.center}>
          <ActivityIndicator color={tokens.accent} />
        </View>
      ) : error ? (
        <View style={styles.error}>
          <Text style={[typeScale.sub, { color: tokens.danger }]}>{error.message}</Text>
          <Button
            label="Retry"
            variant="secondary"
            onPress={() => void (searching ? search.refetch() : entries.refetch())}
          />
        </View>
      ) : searching ? (
        <ScrollView contentContainerStyle={styles.list}>
          {(search.data?.results ?? []).map((result, index) => (
            <Pressable
              key={`${result.path}:${result.line ?? 0}:${index}`}
              onPress={() => onOpenFile(result.path)}
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
      ) : (
        <ScrollView contentContainerStyle={styles.list}>
          {(entries.data?.entries ?? []).map((entry) => (
            <WorkspaceEntryRow key={entry.path} entry={entry} onOpen={openEntry} />
          ))}
          {entries.data?.entries.length === 0 ? (
            <EmptyState icon={Folder} message="This directory is empty." />
          ) : null}
          {entries.data && entries.data.truncated > 0 ? (
            <Text style={[typeScale.meta, styles.truncated, { color: tokens.ink3 }]}>
              More entries were omitted from this very large directory
            </Text>
          ) : null}
        </ScrollView>
      )}
    </View>
  );
}

function WorkspaceEditor({
  sessionId,
  path,
  onBack,
}: {
  sessionId: string;
  path: string;
  onBack?: () => void;
}) {
  const tokens = useTokens();
  const toast = useToast();
  const file = useWorkspaceFile(sessionId, path);
  const write = useWriteWorkspaceFile();
  const [draft, setDraft] = useState("");
  const [loadedHash, setLoadedHash] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);
  const loadedHashRef = useRef<string | null>(null);

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
    if (!loadedHash || !dirty) return;
    try {
      const saved = await write.mutateAsync({
        session: sessionId,
        path,
        content: draft,
        expected_hash: loadedHash,
      });
      loadResponse(saved);
      toast.show(`Saved ${workspaceBasename(path)}`, { tone: "success" });
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
          <Text style={[typeScale.meta, { color: tokens.ink4 }]}>
            {lineCount.toLocaleString()} lines · {draft.length.toLocaleString()} chars
            {dirty ? " · modified" : ""}
          </Text>
        </View>
        <IconButton
          icon={<RefreshCw size={16} color={tokens.ink2} />}
          onPress={() => void reload()}
          disabled={file.isFetching || write.isPending}
          accessibilityLabel="Reload file from disk"
          style={styles.compactButton}
        />
        <IconButton
          icon={<Save size={17} color={dirty ? tokens.accent : tokens.ink4} />}
          onPress={() => void save()}
          disabled={!dirty || write.isPending}
          accessibilityLabel="Save file"
          style={styles.compactButton}
        />
      </View>
      {write.error ? (
        <View style={[styles.saveError, { backgroundColor: tokens.dangerBg }]}>
          <Text style={[typeScale.meta, { color: tokens.danger }]}>{write.error.message}</Text>
        </View>
      ) : null}
      <TextInput
        value={draft}
        onChangeText={(text) => {
          setDraft(text);
          setDirty(text !== file.data?.content);
          write.reset();
        }}
        multiline
        autoCapitalize="none"
        autoCorrect={false}
        spellCheck={false}
        editable={!write.isPending}
        textAlignVertical="top"
        selectionColor={tokens.accent}
        accessibilityLabel={`Edit ${path}`}
        style={[
          styles.editor,
          {
            color: tokens.ink,
            backgroundColor: tokens.bg0,
          },
        ]}
      />
    </View>
  );
}

export function WorkspaceDock({ sessionId, resourceId }: WorkspaceDockProps) {
  const { baseUrl } = useAuth();
  const { isExpanded } = useBreakpoint();
  const workbench = useWorkbench();
  const [localResource, setLocalResource] = useState<string | null>(null);

  if (!sessionId) {
    return <EmptyState icon={Folder} message="Open a session to browse its workspace." />;
  }
  if (baseUrl && !supportsDirectDaemonEndpoints(baseUrl)) {
    return (
      <EmptyState
        icon={Folder}
        message="Workspace files need a direct connection to this host. Forge Anywhere carries sessions only — connect over your network or a tunnel to browse and edit files."
      />
    );
  }
  const activeResource = resourceId ?? localResource;
  const openFile = (path: string) => {
    if (!isExpanded) {
      setLocalResource(path);
      return;
    }
    workbench.openSurface({
      kind: "files",
      sessionId,
      resourceId: path,
      title: workspaceBasename(path),
    });
  };

  return activeResource ? (
    <WorkspaceEditor
      sessionId={sessionId}
      path={activeResource}
      onBack={!resourceId ? () => setLocalResource(null) : undefined}
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
  entry: {
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
  editor: {
    flex: 1,
    padding: space.space12,
    fontFamily: monoFamily.regular,
    fontSize: 12,
    lineHeight: 18,
  },
});
