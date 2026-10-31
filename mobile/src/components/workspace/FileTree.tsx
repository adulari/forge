import { ChevronDown, ChevronRight, File, Folder, Link, Pencil, Trash2 } from "lucide-react-native";
import React from "react";
import { ActivityIndicator, Pressable, StyleSheet, Text, View } from "react-native";

import { type WorkspaceEntry } from "../../lib/api";
import { useWorkspaceEntries } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { IconButton } from "../ds/IconButton";

const INDENT = 14;

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export interface FileTreeProps {
  sessionId: string;
  expanded: ReadonlySet<string>;
  selectedPath: string | null;
  highlightPath: string | null;
  onToggle: (dir: string) => void;
  onSelect: (entry: WorkspaceEntry) => void;
  onOpenFile: (entry: WorkspaceEntry, pinned: boolean) => void;
  /** `null` when the connection cannot mutate the workspace (Forge Anywhere relay). */
  onRename: ((entry: WorkspaceEntry) => void) | null;
  onDelete: ((entry: WorkspaceEntry) => void) | null;
}

function TreeRow({
  entry,
  depth,
  open,
  selected,
  highlighted,
  props,
}: {
  entry: WorkspaceEntry;
  depth: number;
  open: boolean;
  selected: boolean;
  highlighted: boolean;
  props: FileTreeProps;
}) {
  const tokens = useTokens();
  const isDirectory = entry.kind === "directory";
  const Icon = isDirectory ? Folder : entry.kind === "symlink" ? Link : File;
  const press = () => {
    props.onSelect(entry);
    if (isDirectory) props.onToggle(entry.path);
    else if (entry.kind === "file") props.onOpenFile(entry, false);
  };
  // react-native-web forwards onDoubleClick to the DOM node; native ignores the prop.
  const doublePress = { onDoubleClick: () => entry.kind === "file" && props.onOpenFile(entry, true) } as object;
  return (
    <View style={styles.entryRow}>
      <Pressable
        onPress={press}
        {...doublePress}
        disabled={entry.kind === "symlink"}
        accessibilityRole="button"
        accessibilityState={{ expanded: isDirectory ? open : undefined, selected }}
        accessibilityLabel={`${entry.kind} ${entry.path}`}
        style={({ pressed }) => [
          styles.entry,
          { paddingLeft: space.space8 + depth * INDENT },
          {
            backgroundColor: highlighted ? tokens.selection : pressed ? tokens.bg3 : selected ? tokens.bg2 : "transparent",
          },
          entry.kind === "symlink" && styles.disabled,
        ]}
      >
        <View style={styles.chevron}>
          {isDirectory ? (
            open ? (
              <ChevronDown size={13} color={tokens.ink3} />
            ) : (
              <ChevronRight size={13} color={tokens.ink3} />
            )
          ) : null}
        </View>
        <Icon size={15} strokeWidth={1.75} color={isDirectory ? tokens.accent : tokens.ink3} />
        <Text style={[typeScale.body, styles.entryName, { color: tokens.ink }]} numberOfLines={1}>
          {entry.name}
        </Text>
        {entry.kind === "file" ? (
          <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>{formatBytes(entry.size)}</Text>
        ) : null}
      </Pressable>
      {props.onRename ? (
        <IconButton
          icon={<Pencil size={13} color={tokens.ink3} />}
          onPress={() => props.onRename?.(entry)}
          accessibilityLabel={`Rename ${entry.path}`}
          style={styles.compactButton}
        />
      ) : null}
      {props.onDelete ? (
        <IconButton
          icon={<Trash2 size={13} color={tokens.ink3} />}
          onPress={() => props.onDelete?.(entry)}
          accessibilityLabel={`Delete ${entry.path}`}
          style={styles.compactButton}
        />
      ) : null}
    </View>
  );
}

/** One directory's children. A folder fetches its listing only once it is opened. */
export function TreeLevel({ dir, depth, props }: { dir: string; depth: number; props: FileTreeProps }) {
  const tokens = useTokens();
  const entries = useWorkspaceEntries(props.sessionId, dir);
  if (entries.isLoading) {
    return (
      <View style={[styles.status, { paddingLeft: space.space12 + depth * INDENT }]}>
        <ActivityIndicator size="small" color={tokens.accent} />
      </View>
    );
  }
  if (entries.error) {
    return (
      <Pressable
        onPress={() => void entries.refetch()}
        accessibilityRole="button"
        accessibilityLabel={`Retry loading ${dir || "workspace"}`}
        style={[styles.status, { paddingLeft: space.space12 + depth * INDENT }]}
      >
        <Text style={[typeScale.meta, { color: tokens.danger }]} numberOfLines={2}>
          {entries.error.message} · tap to retry
        </Text>
      </Pressable>
    );
  }
  const list = entries.data?.entries ?? [];
  return (
    <>
      {list.map((entry) => {
        const open = entry.kind === "directory" && props.expanded.has(entry.path);
        return (
          <React.Fragment key={entry.path}>
            <TreeRow
              entry={entry}
              depth={depth}
              open={open}
              selected={props.selectedPath === entry.path}
              highlighted={props.highlightPath === entry.path}
              props={props}
            />
            {open ? <TreeLevel dir={entry.path} depth={depth + 1} props={props} /> : null}
          </React.Fragment>
        );
      })}
      {list.length === 0 ? (
        <Text
          style={[typeScale.meta, styles.status, { color: tokens.ink4, paddingLeft: space.space12 + depth * INDENT }]}
        >
          Empty
        </Text>
      ) : null}
      {entries.data && entries.data.truncated > 0 ? (
        <Text
          style={[typeScale.meta, styles.status, { color: tokens.ink3, paddingLeft: space.space12 + depth * INDENT }]}
        >
          More entries were omitted from this very large directory
        </Text>
      ) : null}
    </>
  );
}

const styles = StyleSheet.create({
  entryRow: { flexDirection: "row", alignItems: "center", paddingRight: space.space8 },
  entry: {
    flex: 1,
    minHeight: 32,
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    paddingRight: space.space8,
    paddingVertical: 4,
  },
  chevron: { width: 14, alignItems: "center", justifyContent: "center" },
  entryName: { flex: 1 },
  disabled: { opacity: 0.5 },
  compactButton: { width: 28, height: 28 },
  status: { paddingVertical: 6, paddingRight: space.space12 },
});
