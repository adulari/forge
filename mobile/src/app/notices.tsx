// Notices — what the model sent with `push_notification`, per host, newest first. The hosted push
// that woke the phone could only say "open Forge"; this is where the actual text is.
import { Bell } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useRef } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { BackLink } from "../components/ds/BackLink";
import { BoundedList } from "../components/ds/BoundedList";
import { EmptyState } from "../components/ds/EmptyState";
import { RelativeTime } from "../components/ds/RelativeTime";
import { Screen } from "../components/ds/Screen";
import { SectionHeader } from "../components/ds/SectionHeader";
import { useOpenNotice } from "../components/notices/useOpenNotice";
import { useAuth } from "../lib/auth";
import { buildNoticeRows, sessionLabel, type NoticeListRow } from "../lib/notices/core";
import { markNoticesSeen, syncNotices, useNoticesStore } from "../lib/notices/store";
import { useSessions } from "../lib/queries";
import { usePullRefresh } from "../lib/usePullRefresh";
import { useTokens } from "../theme/ThemeProvider";
import { radii, space } from "../theme/tokens";
import { monoFamily, type as typeScale } from "../theme/typography";
import { useBreakpoint } from "../theme/useBreakpoint";

function NoticeRowView({ row, titles }: { row: Extract<NoticeListRow, { kind: "notice" }>; titles: ReadonlyMap<string, string> }) {
  const tokens = useTokens();
  const open = useOpenNotice();
  const { notice } = row;
  const session = sessionLabel(notice, titles);
  return (
    <Pressable
      onPress={() => open(row.serverId, notice)}
      accessibilityRole="button"
      accessibilityLabel={`${row.unseen ? "Unread. " : ""}${notice.title}. ${notice.message}`}
      style={[styles.row, { borderBottomColor: tokens.hairline }]}
    >
      <View style={[styles.dot, { backgroundColor: row.unseen ? tokens.accent : "transparent" }]} />
      <View style={styles.rowBody}>
        <View style={styles.rowHead}>
          <Text style={[typeScale.bodyBold, styles.rowTitle, { color: tokens.ink }]} numberOfLines={1}>
            {notice.title}
          </Text>
          <RelativeTime timestampMs={notice.at * 1000} />
        </View>
        <Text style={[typeScale.body, { color: tokens.ink2 }]} numberOfLines={4}>
          {notice.message}
        </Text>
        {session ? (
          <Text style={[typeScale.meta, { color: tokens.ink3, fontFamily: monoFamily.regular }]} numberOfLines={1}>
            {session}
          </Text>
        ) : null}
      </View>
    </Pressable>
  );
}

export default function NoticesScreen() {
  const tokens = useTokens();
  const { isExpanded } = useBreakpoint();
  const { servers } = useAuth();
  const { hosts } = useNoticesStore();
  const sessions = useSessions();
  const rows = useMemo(() => buildNoticeRows(servers, hosts), [servers, hosts]);
  const titles = useMemo(
    () => new Map((sessions.data ?? []).map((s) => [s.id, s.title])),
    [sessions.data],
  );
  const refresh = useCallback(async () => {
    await Promise.all(servers.map((s) => syncNotices({ id: s.id, baseUrl: s.baseUrl })));
  }, [servers]);
  const pull = usePullRefresh(refresh);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Unread dots stay while the list is open; leaving it is what counts as having looked.
  const hostIds = useRef<string[]>([]);
  useEffect(() => {
    hostIds.current = Object.keys(hosts);
  }, [hosts]);
  useEffect(() => () => hostIds.current.forEach((id) => markNoticesSeen(id)), []);

  const renderItem = useCallback(
    ({ item }: { item: NoticeListRow }) =>
      item.kind === "header" ? <SectionHeader>{item.label}</SectionHeader> : <NoticeRowView row={item} titles={titles} />,
    [titles],
  );

  return (
    <Screen scroll={false}>
      <View style={styles.header}>
        {/* The desktop sidebar opens Notices directly, so "back to Inbox" was only true on phones. */}
        {isExpanded ? null : <BackLink label="Inbox" />}
        <Text style={[typeScale.title, { color: tokens.ink }]}>Notices</Text>
        <Text style={[typeScale.sub, { color: tokens.ink3 }]}>Messages your sessions sent you.</Text>
      </View>
      <BoundedList
        data={rows}
        keyExtractor={(item: NoticeListRow) => item.key}
        renderItem={renderItem}
        ListEmptyComponent={<EmptyState icon={Bell} message="No notices. When a session sends one, it shows up here." />}
        refreshing={pull.refreshing}
        onRefresh={pull.onRefresh}
        contentContainerStyle={styles.listPad}
      />
    </Screen>
  );
}

const styles = StyleSheet.create({
  header: { paddingTop: space.space12, gap: space.space4 },
  listPad: { paddingTop: space.space16, paddingBottom: space.space32 },
  row: {
    flexDirection: "row",
    gap: space.space8,
    paddingHorizontal: space.space16,
    paddingVertical: space.space12,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  dot: { width: 8, height: 8, borderRadius: radii.radius8, marginTop: space.space8 },
  rowBody: { flex: 1, gap: space.space4 },
  rowHead: { flexDirection: "row", alignItems: "baseline", gap: space.space8 },
  rowTitle: { flex: 1 },
});
