// "Continue a Claude Code session…": a quick picker over the host's Claude Code conversations,
// newest first. Enter (or a click) imports it into Forge, or syncs the new turns when it already
// came over, and opens the Forge session. Mounted lazily by ClaudePickerHost.
import { Terminal } from "lucide-react-native";
import React, { useMemo, useState } from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";

import { claudePickerRows } from "../../lib/claudeHandoff";
import { useContinueFromClaudeCode } from "../../lib/claudeHandoffQueries";
import { useImportScan } from "../../lib/importQueries";
import { conversationAge, shortFolder } from "../../lib/importSessions";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { Button } from "../ds/Button";
import { EmptyState } from "../ds/EmptyState";
import { ListRow } from "../ds/ListRow";
import { SearchField } from "../ds/SearchField";
import { Sheet } from "../ds/Sheet";
import { Skeleton } from "../ds/Skeleton";

const MAX_ROWS = 60;

export default function ClaudeSessionPicker({ visible, onClose }: { visible: boolean; onClose: () => void }) {
  const tokens = useTokens();
  const scan = useImportScan();
  const continueFrom = useContinueFromClaudeCode();
  const [query, setQuery] = useState("");
  const [busyId, setBusyId] = useState<string | null>(null);
  const [nowMs] = useState(() => Date.now());
  const rows = useMemo(() => claudePickerRows(scan.data?.items ?? [], query).slice(0, MAX_ROWS), [scan.data, query]);

  const choose = (id: string) => {
    if (busyId) return;
    setBusyId(id);
    continueFrom(
      id,
      () => {
        setBusyId(null);
        onClose();
      },
      // A failure leaves the picker open for another try; the toast says why.
      () => setBusyId(null),
    );
  };

  const body = (() => {
    if (scan.isLoading) {
      return (
        <View style={styles.skeletons}>
          <Skeleton height={52} width="100%" />
          <Skeleton height={52} width="100%" />
          <Skeleton height={52} width="100%" />
        </View>
      );
    }
    if (scan.isError) {
      return (
        <EmptyState
          icon={Terminal}
          message={`Could not look for Claude Code sessions on this host. ${scan.error?.message ?? ""}`.trim()}
          action={<Button label="Try again" variant="secondary" onPress={() => void scan.refetch()} />}
        />
      );
    }
    if (rows.length === 0) {
      return (
        <Text style={[type.body, styles.none, { color: tokens.ink3 }]}>
          {query.trim() ? "No Claude Code session matches." : "No Claude Code sessions found on this computer."}
        </Text>
      );
    }
    return (
      <ScrollView keyboardShouldPersistTaps="handled" style={styles.list}>
        {rows.map((row, i) => (
          <ListRow
            key={row.key}
            title={row.title}
            subtitle={`${row.cwdExists ? shortFolder(row.cwd) : `${shortFolder(row.cwd)} (missing)`} · ${conversationAge(row.modified, nowMs)}`}
            trailing={
              busyId === row.id ? (
                <Badge label="Opening" tone="neutral" lowercase />
              ) : row.linkedSessionId ? (
                <Badge label="In Forge" tone="neutral" lowercase />
              ) : undefined
            }
            onPress={() => choose(row.id)}
            showSeparator={i < rows.length - 1}
            accessibilityLabel={`${row.title}${row.linkedSessionId ? ", already in Forge, syncs new turns" : ""}`}
          />
        ))}
      </ScrollView>
    );
  })();

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Continue a Claude Code session" snapPoints={[0.75]}>
      <View style={styles.sheet}>
        <Text style={[type.headingBold, { color: tokens.ink }]}>Continue a Claude Code session</Text>
        <Text style={[type.sub, { color: tokens.ink3 }]}>
          Brings the conversation into Forge and opens it. Sessions already in Forge pick up their new turns.
        </Text>
        <SearchField
          value={query}
          onChangeText={setQuery}
          placeholder="Search by title or folder"
          accessibilityLabel="Search Claude Code sessions"
          autoFocus
          onSubmitEditing={() => rows[0] && choose(rows[0].id)}
        />
        {body}
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  sheet: { paddingHorizontal: space.space20, paddingBottom: space.space24, gap: space.space8 },
  list: { maxHeight: 420 },
  skeletons: { gap: space.space8 },
  none: { padding: space.space24, textAlign: "center" },
});
