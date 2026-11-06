// The attention-ordered change queue: groups per agent, each file with a reviewed check that is
// tied to the content hash, so an agent editing the file again clears it.
import { Check, CheckCheck, FileDiff } from "lucide-react-native";
import React, { useCallback, useMemo } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, sansFamilyFor } from "../../theme/typography";
import { BoundedList } from "../ds/BoundedList";
import { EmptyState } from "../ds/EmptyState";
import { Skeleton } from "../ds/Skeleton";
import { FileTypeIcon } from "../git/FileTypeIcon";
import { riskLine, riskTotals, type AttentionTier, type QueueGroup, type RiskTotals } from "./changeQueueModel";
import { useIde } from "./IdeContext";
import { AgentDot, IdeTextButton, LineDelta, ROW_HEIGHT } from "./ideUi";
import { rowBackground } from "./rowStyle";
import { useAcceptRoutine } from "./useAcceptRoutine";

const GROUP_HEIGHT = 46;

const SECTION_HEIGHT = 24;
const SUMMARY_HEIGHT = 40;

type Item =
  | { type: "summary"; key: string; totals: RiskTotals }
  | { type: "section"; key: string; label: string }
  | { type: "group"; key: string; group: QueueGroup }
  | {
      type: "file";
      key: string;
      sessionId: string;
      path: string;
      hash: string;
      adds: number;
      dels: number;
      tier: AttentionTier;
      risk?: "routine" | "look";
      reason?: string;
    };

/**
 * Per agent: its files, and when the daemon ranked them, a "Look here" and a "Routine" label over
 * the unreviewed ones (the rows are already sorted that way). One summary row leads the list.
 */
function toItems(groups: readonly QueueGroup[]): Item[] {
  const out: Item[] = [];
  const totals = riskTotals(groups.flatMap((group) => group.rows));
  if (totals.ranked && totals.lookHunks + totals.routineHunks > 0) out.push({ type: "summary", key: "summary", totals });
  for (const group of groups) {
    out.push({ type: "group", key: `g:${group.sessionId}`, group });
    const ranked = group.rows.some((row) => row.file.risk !== undefined);
    const counts = { look: 0, routine: 0 };
    for (const row of group.rows) if (row.tier !== "reviewed") counts[row.file.risk === "routine" ? "routine" : "look"] += 1;
    let section: "look" | "routine" | null = null;
    for (const row of group.rows) {
      if (ranked && row.tier !== "reviewed") {
        const next = row.file.risk === "routine" ? "routine" : "look";
        if (next !== section) {
          section = next;
          const noun = counts[next] === 1 ? "file" : "files";
          out.push({ type: "section", key: `s:${group.sessionId}:${next}`, label: `${next === "look" ? "Look here" : "Routine"} · ${counts[next]} ${noun}` });
        }
      }
      out.push({
        type: "file",
        key: row.key,
        sessionId: row.sessionId,
        path: row.path,
        hash: row.file.hash,
        adds: row.file.adds,
        dels: row.file.dels,
        tier: row.tier,
        risk: row.file.risk,
        reason: row.file.riskReason,
      });
    }
  }
  return out;
}

const SummaryRow = React.memo(function SummaryRow({ totals }: { totals: RiskTotals }) {
  const tokens = useTokens();
  const { accept, busy, count, hasWork } = useAcceptRoutine();
  return (
    <View style={[styles.summary, { borderBottomColor: tokens.hairline }]}>
      <Text numberOfLines={1} style={[styles.summaryText, { color: tokens.ink2 }]}>
        {riskLine(totals)}
      </Text>
      {hasWork ? (
        <IdeTextButton
          label="Accept routine"
          icon={<CheckCheck size={12} color={tokens.ink2} />}
          onPress={() => void accept()}
          loading={busy}
          disabled={count === 0}
        />
      ) : null}
    </View>
  );
});

const SectionLabelRow = React.memo(function SectionLabelRow({ label }: { label: string }) {
  const tokens = useTokens();
  return (
    <View style={styles.section}>
      <Text numberOfLines={1} style={[styles.sectionText, { color: tokens.ink3 }]}>
        {label}
      </Text>
    </View>
  );
});

const GroupHeader = React.memo(function GroupHeader({ group }: { group: QueueGroup }) {
  const tokens = useTokens();
  const state = group.waiting ? "waiting" : group.busy ? "busy" : "idle";
  return (
    <View style={[styles.group, { borderTopColor: tokens.hairline }]}>
      <View style={styles.groupTop}>
        <AgentDot state={state} />
        <Text numberOfLines={1} style={[styles.groupName, { color: tokens.ink }]}>
          {group.name}
        </Text>
      </View>
      <Text numberOfLines={1} style={[styles.groupSummary, { color: group.waiting ? tokens.accent : tokens.ink3 }]}>
        {group.summary}
      </Text>
    </View>
  );
});

const FileRow = React.memo(function FileRow({
  sessionId,
  path,
  hash,
  adds,
  dels,
  tier,
  risk,
  reason,
  selected,
  onOpen,
  onToggle,
}: {
  sessionId: string;
  path: string;
  hash: string;
  adds: number;
  dels: number;
  tier: AttentionTier;
  risk?: "routine" | "look";
  reason?: string;
  selected: boolean;
  onOpen: (sessionId: string, path: string, pinned: boolean) => void;
  onToggle: (sessionId: string, path: string, hash: string) => void;
}) {
  const tokens = useTokens();
  const slash = path.lastIndexOf("/");
  const base = path.slice(slash + 1);
  const dir = slash === -1 ? "" : path.slice(0, slash);
  const reviewed = tier === "reviewed";
  const open = useCallback(() => onOpen(sessionId, path, false), [onOpen, sessionId, path]);
  const pin = useMemo(() => ({ onDoubleClick: () => onOpen(sessionId, path, true) }) as object, [onOpen, sessionId, path]);
  const toggle = useCallback(() => onToggle(sessionId, path, hash), [onToggle, sessionId, path, hash]);
  return (
    <Pressable
      onPress={open}
      {...pin}
      accessibilityRole="button"
      accessibilityState={{ selected }}
      accessibilityLabel={`${path}${reason ? `, ${reason}` : ""}${reviewed ? ", reviewed" : ""}`}
      style={({ hovered, pressed }: { hovered?: boolean; pressed: boolean }) => [
        styles.file,
        { backgroundColor: rowBackground(tokens, { hovered, pressed, selected }) },
      ]}
    >
      <Pressable
        onPress={toggle}
        hitSlop={4}
        accessibilityRole="checkbox"
        accessibilityState={{ checked: reviewed }}
        accessibilityLabel={reviewed ? `Mark ${base} not reviewed` : `Mark ${base} reviewed`}
        style={[styles.check, { borderColor: reviewed ? tokens.ink3 : tokens.borderStrong, backgroundColor: reviewed ? tokens.ink3 : "transparent" }]}
      >
        {reviewed ? <Check size={10} strokeWidth={2.5} color={tokens.bg1} /> : null}
      </Pressable>
      <View style={{ opacity: reviewed ? 0.45 : 1 }}>
        <FileTypeIcon path={path} />
      </View>
      <Text numberOfLines={1} style={[styles.name, { color: reviewed ? tokens.ink3 : tokens.ink }]}>
        {base}
        {dir ? <Text style={[styles.dir, { color: tokens.ink4 }]}>{`  ${dir}`}</Text> : null}
      </Text>
      {reason && !reviewed ? (
        <Text numberOfLines={1} style={[styles.reason, { color: risk === "routine" ? tokens.ink4 : tokens.ink2 }]}>
          {reason}
        </Text>
      ) : null}
      <LineDelta adds={adds} dels={dels} dim={reviewed} />
    </Pressable>
  );
});

export function ChangeQueue() {
  const tokens = useTokens();
  const { groups, active, openChange, toggleReviewed, changesLoading, sessions, scope } = useIde();
  const items = useMemo(() => toItems(groups), [groups]);
  const selectedKey = active?.kind === "diff" ? `${active.sessionId}\0${active.path}` : null;

  const onOpen = useCallback((sessionId: string, path: string, pinned: boolean) => openChange(sessionId, path, pinned), [openChange]);

  const renderItem = useCallback(
    ({ item }: { item: Item }) =>
      item.type === "summary" ? (
        <SummaryRow totals={item.totals} />
      ) : item.type === "section" ? (
        <SectionLabelRow label={item.label} />
      ) : item.type === "group" ? (
        <GroupHeader group={item.group} />
      ) : (
        <FileRow
          sessionId={item.sessionId}
          path={item.path}
          hash={item.hash}
          adds={item.adds}
          dels={item.dels}
          tier={item.tier}
          risk={item.risk}
          reason={item.reason}
          selected={selectedKey === item.key}
          onOpen={onOpen}
          onToggle={toggleReviewed}
        />
      ),
    [selectedKey, onOpen, toggleReviewed],
  );

  const scoped = scope != null ? sessions.find((row) => row.id === scope) : null;
  const empty = changesLoading ? (
    <View style={styles.skeleton}>
      {[72, 56, 64, 48].map((w, i) => (
        <Skeleton key={i} width={`${w}%`} height={10} />
      ))}
    </View>
  ) : (
    <EmptyState
      icon={FileDiff}
      message={
        sessions.length === 0
          ? "Nothing to review yet."
          : scoped
            ? `${scoped.title?.trim() || "This agent"} has not changed any files yet.`
            : "No changes yet. Ask an agent to start."
      }
    />
  );

  return (
    <BoundedList<Item>
      data={items}
      keyExtractor={(item) => item.key}
      renderItem={renderItem}
      extraData={selectedKey}
      getItemLayout={undefined}
      initialNumToRender={30}
      windowSize={9}
      ListEmptyComponent={empty}
      contentContainerStyle={{ paddingBottom: space.space12, backgroundColor: tokens.bg1 }}
    />
  );
}

const styles = StyleSheet.create({
  group: { height: GROUP_HEIGHT, justifyContent: "center", gap: 2, paddingHorizontal: space.space12, borderTopWidth: StyleSheet.hairlineWidth },
  groupTop: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  groupName: { flex: 1, minWidth: 0, fontFamily: sansFamilyFor(600), fontSize: 12.5 },
  groupSummary: { marginLeft: 15, fontFamily: sansFamilyFor(400), fontSize: 11 },
  file: { height: ROW_HEIGHT, flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space12 },
  check: { width: 14, height: 14, borderRadius: 7, borderWidth: 1, alignItems: "center", justifyContent: "center" },
  name: { flex: 1, minWidth: 0, fontFamily: sansFamilyFor(500), fontSize: 12.5 },
  dir: { fontFamily: monoFamily.regular, fontSize: 10.5 },
  summary: { minHeight: SUMMARY_HEIGHT, flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: space.space8, paddingHorizontal: space.space12, borderBottomWidth: StyleSheet.hairlineWidth },
  summaryText: { flex: 1, minWidth: 0, fontFamily: sansFamilyFor(600), fontSize: 12.5 },
  section: { height: SECTION_HEIGHT, justifyContent: "flex-end", paddingHorizontal: space.space12, paddingBottom: 2 },
  sectionText: { fontFamily: monoFamily.regular, fontSize: 10, letterSpacing: 0.6, textTransform: "uppercase" },
  reason: { maxWidth: "42%", fontFamily: sansFamilyFor(400), fontSize: 10.5 },
  skeleton: { padding: space.space12, gap: space.space12 },
});
