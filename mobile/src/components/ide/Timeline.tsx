// What the agent did, newest first: prompts, reads, edits, commands. Clicking an edit opens the
// file it touched. Reads the history page the chat already loads, so it adds no socket.
import { FileEdit, FileText, MessageSquare, Terminal, Wrench, CornerDownRight } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { type FlatList, Pressable, StyleSheet, Text, View } from "react-native";

import { useHistory } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, sansFamilyFor } from "../../theme/typography";
import { BoundedList } from "../ds/BoundedList";
import { EmptyState } from "../ds/EmptyState";
import { Skeleton } from "../ds/Skeleton";
import { useIde } from "./IdeContext";
import { useTurnFocus } from "./ideSignals";
import { LineDelta } from "./ideUi";
import { rowBackground } from "./rowStyle";
import { buildTimeline, findTurnItem, resolveChangedPath, type TimelineItem, type TimelineKind } from "./timelineModel";

const ROW = 26;

const ICONS: Record<TimelineKind, React.ComponentType<{ size?: number; color?: string; strokeWidth?: number }>> = {
  prompt: MessageSquare,
  reply: CornerDownRight,
  read: FileText,
  edit: FileEdit,
  command: Terminal,
  tool: Wrench,
};

const Row = React.memo(function Row({
  item,
  onOpen,
  highlighted,
}: {
  item: TimelineItem;
  onOpen: (item: TimelineItem) => void;
  highlighted: boolean;
}) {
  const tokens = useTokens();
  const Icon = ICONS[item.kind];
  const open = useCallback(() => onOpen(item), [item, onOpen]);
  const clickable = item.kind === "edit" && item.path != null;
  const muted = item.kind === "read" || item.kind === "tool";
  const content = (
    <>
      <Icon size={12} strokeWidth={1.75} color={item.failed ? tokens.danger : item.kind === "prompt" ? tokens.ink : tokens.ink3} />
      <Text
        numberOfLines={1}
        style={[
          item.kind === "command" || item.kind === "edit" || item.kind === "read" ? styles.mono : styles.sans,
          { color: item.failed ? tokens.danger : muted ? tokens.ink3 : item.kind === "reply" ? tokens.ink2 : tokens.ink },
        ]}
      >
        {item.label}
      </Text>
      {item.kind === "edit" ? <LineDelta adds={item.adds} dels={item.dels} /> : null}
    </>
  );
  if (!clickable) return <View style={[styles.row, highlighted && { backgroundColor: tokens.bg3 }]}>{content}</View>;
  return (
    <Pressable
      onPress={open}
      accessibilityRole="button"
      accessibilityLabel={`Open ${item.label}`}
      style={({ hovered }: { hovered?: boolean; pressed: boolean }) => [styles.row, { backgroundColor: highlighted ? tokens.bg3 : rowBackground(tokens, { hovered }) }]}
    >
      {content}
    </Pressable>
  );
});

export function Timeline({ sessionId }: { sessionId: string }) {
  const { changes, openChange, openFile } = useIde();
  const history = useHistory(sessionId, { includeTools: true });
  const rows = useMemo(() => history.data?.pages.flat() ?? [], [history.data]);
  const items = useMemo(() => buildTimeline(rows), [rows]);
  const listRef = useRef<FlatList<TimelineItem>>(null);
  const [highlight, setHighlight] = useState<string | null>(null);
  const focus = useTurnFocus();
  useEffect(() => {
    if (!focus || focus.sessionId !== sessionId) return;
    const index = findTurnItem(items, focus.turn, focus.path);
    if (index < 0) return;
    setHighlight(items[index].key);
    listRef.current?.scrollToIndex({ index, animated: false, viewPosition: 0.3 });
    const timer = setTimeout(() => setHighlight(null), 2500);
    return () => clearTimeout(timer);
    // Only a new jump request moves the list; live updates to `items` must not re-scroll it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focus]);
  const changedPaths = useMemo(() => changes.get(sessionId)?.files.map((file) => file.path) ?? [], [changes, sessionId]);

  const onOpen = useCallback(
    (item: TimelineItem) => {
      if (!item.path) return;
      const changed = resolveChangedPath(item.path, changedPaths);
      if (changed) openChange(sessionId, changed);
      else openFile(sessionId, item.path.replace(/^.*?\/(?=[^/]+\/[^/]+$)/, ""));
    },
    [changedPaths, openChange, openFile, sessionId],
  );

  if (history.isPending) {
    return (
      <View style={styles.skeleton}>
        {[64, 48, 72, 40].map((w, i) => (
          <Skeleton key={i} width={`${w}%`} height={10} />
        ))}
      </View>
    );
  }
  return (
    <BoundedList<TimelineItem>
      ref={listRef}
      data={items}
      extraData={highlight}
      keyExtractor={(item) => item.key}
      renderItem={({ item }) => <Row item={item} onOpen={onOpen} highlighted={item.key === highlight} />}
      getItemLayout={(_, index) => ({ length: ROW, offset: ROW * index, index })}
      ListEmptyComponent={<EmptyState icon={MessageSquare} message="Nothing yet. Message this agent to give it work." />}
    />
  );
}

const styles = StyleSheet.create({
  row: { height: ROW, flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space16 },
  sans: { flex: 1, minWidth: 0, fontFamily: sansFamilyFor(400), fontSize: 12 },
  mono: { flex: 1, minWidth: 0, fontFamily: monoFamily.regular, fontSize: 11 },
  skeleton: { padding: space.space16, gap: space.space12 },
});
