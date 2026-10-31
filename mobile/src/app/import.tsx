// Settings > Import: bring Claude Code and Codex conversations from the host into Forge.
// Search, agent and folder filters, select-all, a plural-aware result toast and a project to
// open conversations in when their own folder is gone (crates/forge-cli/src/serve_import.rs).
// The originals are only read; an imported session continues where the conversation stopped.
import { Check, ChevronDown, Download, RefreshCw } from "lucide-react-native";
import React, { useEffect, useMemo, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { Badge } from "../components/ds/Badge";
import { Button } from "../components/ds/Button";
import { EmptyState } from "../components/ds/EmptyState";
import { ListRow } from "../components/ds/ListRow";
import { PageHeader } from "../components/ds/PageHeader";
import { Screen } from "../components/ds/Screen";
import { SearchField } from "../components/ds/SearchField";
import { SectionHeader } from "../components/ds/SectionHeader";
import { Segmented } from "../components/ds/Segmented";
import { Sheet } from "../components/ds/Sheet";
import { Skeleton } from "../components/ds/Skeleton";
import { useToast } from "../components/ds/ToastHost";
import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { ProjectPicker } from "../components/session/ProjectPicker";
import { useImportScan, useImportSessions } from "../lib/importQueries";
import {
  type ImportAgentFilter,
  type ImportCandidate,
  agentLabel,
  allSelected,
  conversationAge,
  filterCandidates,
  folderOptions,
  importButtonLabel,
  importToast,
  itemKey,
  pruneSelection,
  selectable,
  shortFolder,
  toggleAll,
} from "../lib/importSessions";
import { useTokens } from "../theme/ThemeProvider";
import { radii, space } from "../theme/tokens";
import { type } from "../theme/typography";
import { SettingsShell } from "./(tabs)/settings";

const AGENT_OPTIONS = [
  { value: "all" as const, label: "All" },
  { value: "claude" as const, label: "Claude" },
  { value: "codex" as const, label: "Codex" },
];

function CheckBox({ on, disabled }: { on: boolean; disabled: boolean }) {
  const tokens = useTokens();
  return (
    <View
      style={[
        styles.box,
        {
          borderColor: on ? tokens.accent : tokens.border,
          backgroundColor: on ? tokens.accent : "transparent",
          opacity: disabled ? 0.4 : 1,
        },
      ]}
    >
      {on ? <Check size={14} strokeWidth={3} color={tokens.bg0} /> : null}
    </View>
  );
}

function ConversationRow({
  item,
  selected,
  onToggle,
  last,
  nowMs,
}: {
  item: ImportCandidate;
  selected: boolean;
  onToggle: () => void;
  last: boolean;
  nowMs: number;
}) {
  const imported = item.imported_session_id !== null;
  const where = item.cwd_exists ? shortFolder(item.cwd) : `${shortFolder(item.cwd)} (missing)`;
  return (
    <ListRow
      title={item.title}
      subtitle={`${agentLabel(item.agent)} · ${where} · ${conversationAge(item.modified, nowMs)}`}
      leading={<CheckBox on={selected} disabled={imported} />}
      trailing={imported ? <Badge label="Imported" tone="neutral" lowercase /> : undefined}
      onPress={imported ? undefined : onToggle}
      disabled={imported}
      showSeparator={!last}
      accessibilityRole="checkbox"
      accessibilityLabel={`${item.title}, ${agentLabel(item.agent)}${imported ? ", already imported" : ""}${
        selected ? ", selected" : ""
      }`}
    />
  );
}

export default function ImportScreen() {
  const tokens = useTokens();
  const toast = useToast();
  const scan = useImportScan();
  const run = useImportSessions();
  const [agent, setAgent] = useState<ImportAgentFilter>("all");
  const [folder, setFolder] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [folderSheet, setFolderSheet] = useState(false);
  const [fallback, setFallback] = useState("");
  const [nowMs] = useState(() => Date.now());

  const items = useMemo(() => scan.data?.items ?? [], [scan.data]);
  const visible = useMemo(() => filterCandidates(items, { agent, folder, query }), [items, agent, folder, query]);
  const folders = useMemo(() => folderOptions(items), [items]);
  const importable = selectable(items).length;
  const everySelected = allSelected(visible, selected);
  const needsFallback = items.some((c) => selected.has(itemKey(c)) && !c.cwd_exists);

  useEffect(() => {
    if (scan.data && !fallback) setFallback(scan.data.fallback_cwd);
  }, [scan.data, fallback]);
  useEffect(() => {
    setSelected((s) => pruneSelection(items, s));
  }, [items]);

  const doImport = () => {
    const chosen = items.filter((c) => selected.has(itemKey(c)));
    if (chosen.length === 0 || run.isPending) return;
    run.mutate(
      { items: chosen.map((c) => ({ agent: c.agent, id: c.id })), fallbackCwd: needsFallback ? fallback : undefined },
      {
        onSuccess: (reply) => {
          const t = importToast(reply);
          toast.show(t.message, { tone: t.tone });
          setSelected(new Set());
        },
        onError: (error) => toast.show(error.message || "Import failed", { tone: "danger" }),
      },
    );
  };

  const body = (() => {
    if (scan.isLoading) {
      return (
        <View style={styles.skeletons}>
          <Skeleton height={56} width="100%" />
          <Skeleton height={56} width="100%" />
          <Skeleton height={56} width="100%" />
        </View>
      );
    }
    if (scan.isError) {
      return (
        <EmptyState
          icon={Download}
          message={`Could not look for conversations on this host. ${scan.error?.message ?? ""}`.trim()}
          action={<Button label="Try again" variant="secondary" onPress={() => void scan.refetch()} />}
        />
      );
    }
    if (items.length === 0) {
      return <EmptyState icon={Download} message="No importable sessions found on this computer." />;
    }
    if (visible.length === 0) {
      return <Text style={[type.body, styles.none, { color: tokens.ink3 }]}>Nothing matches these filters.</Text>;
    }
    return (
      <View>
        {visible.map((item, i) => (
          <ConversationRow
            key={itemKey(item)}
            item={item}
            selected={selected.has(itemKey(item))}
            onToggle={() =>
              setSelected((s) => {
                const next = new Set(s);
                if (!next.delete(itemKey(item))) next.add(itemKey(item));
                return next;
              })
            }
            last={i === visible.length - 1}
            nowMs={nowMs}
          />
        ))}
      </View>
    );
  })();

  return (
    <DesktopDrillDown>
      <SettingsShell active="import">
        <Screen scroll contentContainerStyle={styles.content}>
          <PageHeader
            title="Import conversations"
            subtitle="Bring Claude Code and Codex sessions from this computer into Forge. The originals are only read, never changed, and an imported session continues where it stopped."
            trailing={
              <Button
                label="Rescan"
                variant="ghost"
                icon={<RefreshCw size={15} strokeWidth={1.75} color={tokens.ink2} />}
                onPress={() => void scan.refetch()}
                loading={scan.isFetching && !scan.isLoading}
              />
            }
          />

          <SearchField
            value={query}
            onChangeText={setQuery}
            placeholder="Search conversations"
            accessibilityLabel="Search conversations"
          />
          <View style={styles.filters}>
            <View style={styles.agentFilter}>
              <Segmented options={AGENT_OPTIONS} value={agent} onChange={setAgent} testID="import-agent-filter" />
            </View>
            <Button
              label={folder === null ? "All folders" : shortFolder(folder)}
              variant="secondary"
              icon={<ChevronDown size={15} strokeWidth={1.75} color={tokens.ink2} />}
              onPress={() => setFolderSheet(true)}
              accessibilityLabel={`Folder filter: ${folder === null ? "all folders" : folder}`}
            />
          </View>

          <View style={[styles.bar, { borderColor: tokens.hairline }]}>
            <ListRow
              title={everySelected ? "Clear selection" : `Select all (${selectable(visible).length})`}
              subtitle={selected.size > 0 ? `${selected.size} selected` : `${importable} not yet in Forge`}
              leading={<CheckBox on={everySelected} disabled={selectable(visible).length === 0} />}
              onPress={selectable(visible).length === 0 ? undefined : () => setSelected((s) => toggleAll(visible, s))}
              disabled={selectable(visible).length === 0}
              showSeparator={false}
              accessibilityRole="checkbox"
              accessibilityLabel="Select all visible conversations"
              trailing={
                <Button
                  label={importButtonLabel(selected.size)}
                  onPress={doImport}
                  disabled={selected.size === 0}
                  loading={run.isPending}
                  testID="import-run"
                />
              }
              hasInteractiveTrailing
            />
          </View>

          {needsFallback ? (
            <View>
              <SectionHeader>Open in</SectionHeader>
              <Text style={[type.sub, styles.hint, { color: tokens.ink3 }]}>
                Some of the selected conversations were in folders that are not on this host. They open in this project.
              </Text>
              <ProjectPicker value={fallback} onChange={setFallback} />
            </View>
          ) : null}

          {body}
        </Screen>

        <Sheet visible={folderSheet} onClose={() => setFolderSheet(false)} accessibilityLabel="Filter by folder" snapPoints={[0.7]}>
          <View style={styles.sheet}>
            <Text style={[type.headingBold, { color: tokens.ink }]}>Folders</Text>
            <ListRow
              title="All folders"
              subtitle={`${items.length} conversations`}
              trailing={folder === null ? <Check size={16} strokeWidth={2} color={tokens.success} /> : undefined}
              onPress={() => {
                setFolder(null);
                setFolderSheet(false);
              }}
            />
            {folders.map((f, i) => (
              <ListRow
                key={f.folder}
                title={shortFolder(f.folder)}
                subtitle={`${f.count} ${f.count === 1 ? "conversation" : "conversations"}`}
                trailing={folder === f.folder ? <Check size={16} strokeWidth={2} color={tokens.success} /> : undefined}
                showSeparator={i < folders.length - 1}
                onPress={() => {
                  setFolder(f.folder);
                  setFolderSheet(false);
                }}
              />
            ))}
          </View>
        </Sheet>
      </SettingsShell>
    </DesktopDrillDown>
  );
}

const styles = StyleSheet.create({
  content: { gap: space.space12, paddingBottom: space.space32 },
  filters: { flexDirection: "row", flexWrap: "wrap", alignItems: "center", gap: space.space8 },
  agentFilter: { minWidth: 260 },
  bar: { borderTopWidth: StyleSheet.hairlineWidth, borderBottomWidth: StyleSheet.hairlineWidth },
  box: {
    width: 20,
    height: 20,
    borderWidth: 1.5,
    borderRadius: radii.radius4,
    alignItems: "center",
    justifyContent: "center",
  },
  skeletons: { gap: space.space8 },
  none: { padding: space.space24, textAlign: "center" },
  hint: { paddingBottom: space.space8 },
  sheet: { paddingHorizontal: space.space20, paddingBottom: space.space24, gap: space.space8 },
});
