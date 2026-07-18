// Forge Anywhere — hand a live session's workspace off to another host, encrypted
// capsule in transit. Design comp "AW Handoff Sheet" (mobile.dc.html lines 620-675) +
// "AW Handoff Progress" (lines 677-736). Two phases in one Sheet: pick a destination and
// review preflight, then watch the capsule move. `client.handoffPreflight` already returns
// checkpoint/baseCommit/fileCount/capsuleBytes/blockedFiles — there's no separate
// "checkpoint status" call, the top status line reads straight off that plan.
import { Check } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { formatBytes } from "../../lib/anywhere/format";
import { useAnywhere, useAnywhereHosts } from "../../lib/anywhere/store";
import type { AnywhereHost, HandoffPlan, HandoffStage } from "../../lib/anywhere/types";
import { useEmberdot } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space, tapTarget } from "../../theme/tokens";
import { tabularNums, type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { HeatEdge } from "../ds/HeatEdge";
import { SectionHeader } from "../ds/SectionHeader";
import { Sheet } from "../ds/Sheet";
import { useToast } from "../ds/ToastHost";
import { HostDot } from "./HostDot";

export interface HandoffSheetProps {
  visible: boolean;
  onClose: () => void;
  sessionId: string;
  sessionTitle: string;
  /** Name of the host this session is currently running on — marks that row as the
   * disabled "source" entry in the destination list. */
  sourceHostName: string;
}

// Progress-view checklist slots, in order. "scanning" (preflight) has no row of its own —
// it's already done by the time this sheet's destination phase is showing preflight data.
const PROGRESS_SLOTS: HandoffStage[] = ["packaging", "uploading", "waiting-for-destination", "applying", "awaiting-ack"];
const FAILURE_STAGES: HandoffStage[] = ["rolled-back", "expired"];

export function HandoffSheet({ visible, onClose, sessionId, sessionTitle, sourceHostName }: HandoffSheetProps) {
  const tokens = useTokens();
  const toast = useToast();
  const { client } = useAnywhere();
  const { hosts } = useAnywhereHosts();

  const [plan, setPlan] = useState<HandoffPlan | null>(null);
  const [selectedHostId, setSelectedHostId] = useState<string | null>(null);
  const [phase, setPhase] = useState<"destination" | "progress">("destination");
  const [stage, setStage] = useState<HandoffStage>("scanning");
  const [starting, setStarting] = useState(false);

  useEffect(() => {
    if (!visible) return;
    setPhase("destination");
    setSelectedHostId(null);
    setStage("scanning");
    setPlan(null);
    void client.handoffPreflight(sessionId).then(setPlan);
  }, [visible, sessionId, client]);

  const destinations = useMemo(
    () => hosts.filter((h) => h.state.kind !== "revoked" && h.state.kind !== "disabled"),
    [hosts],
  );
  const selectedHost = useMemo(
    () => destinations.find((h) => h.id === selectedHostId) ?? null,
    [destinations, selectedHostId],
  );

  const runHandoff = useCallback(
    (destHostId: string) => {
      setStarting(true);
      setPhase("progress");
      setStage("scanning");
      void client
        .handoffStart(sessionId, destHostId, (update) => setStage(update.stage))
        .finally(() => setStarting(false));
    },
    [client, sessionId],
  );

  const onCreateCapsule = useCallback(() => {
    if (!selectedHost) return;
    runHandoff(selectedHost.id);
  }, [selectedHost, runHandoff]);

  const onRetry = useCallback(() => {
    if (!selectedHost) return;
    runHandoff(selectedHost.id);
  }, [selectedHost, runHandoff]);

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Hand off workspace" snapPoints={[0.9]}>
      {phase === "destination" ? (
        <View style={styles.content}>
          <View style={styles.headerRow}>
            <Text style={[typeScale.headingBold, styles.flex1, { color: tokens.ink }]}>Hand off workspace</Text>
            <Pressable onPress={onClose} accessibilityRole="button" accessibilityLabel="Cancel" hitSlop={8}>
              <Text style={[typeScale.meta, { color: tokens.ink3 }]}>Cancel</Text>
            </Pressable>
          </View>

          {plan ? (
            <View style={styles.checkpointRow}>
              <View style={[styles.smallCheckWrap, { backgroundColor: tokens.successBg }]}>
                <Check size={9} color={tokens.success} strokeWidth={3} />
              </View>
              <Text style={[typeScale.sub, styles.flex1, { color: tokens.ink2 }]}>
                {"Idle at checkpoint "}
                <Text style={[typeScale.monoMeta, { color: tokens.ink }]}>{plan.checkpoint}</Text>
                {" · base "}
                <Text style={[typeScale.monoMeta, { color: tokens.ink }]}>{plan.baseCommit}</Text>
              </Text>
            </View>
          ) : null}
          <Text style={[typeScale.meta, styles.checkpointNote, { color: tokens.ink4 }]}>
            If a tool call is running, Forge waits — or you interrupt it explicitly first.
          </Text>

          <SectionHeader>destination</SectionHeader>
          {destinations.map((host) => (
            <DestinationRow
              key={host.id}
              host={host}
              isSource={host.name === sourceHostName}
              selected={selectedHostId === host.id}
              onSelect={() => setSelectedHostId(host.id)}
            />
          ))}

          {plan ? (
            <>
              <SectionHeader>preflight</SectionHeader>
              <View style={styles.preflightRow}>
                <Text style={[typeScale.sub, styles.flex1, { color: tokens.ink2 }]}>
                  {`${plan.fileCount} files · capsule ≈ `}
                  <Text style={[typeScale.monoMeta, { color: tokens.ink }]}>{formatBytes(plan.capsuleBytes)}</Text>
                  {" encrypted"}
                </Text>
                <Text style={[typeScale.monoMeta, { color: tokens.success }]}>scan clean-ish</Text>
              </View>

              {plan.blockedFiles.length > 0 ? (
                <View style={styles.blockedWrap}>
                  <HeatEdge state="waiting" />
                  <View style={styles.blockedInner}>
                    <Text style={[typeScale.bodyBold, { color: tokens.danger }]}>
                      {`${plan.blockedFiles.length} files won't travel — nothing is dropped silently`}
                    </Text>
                    {plan.blockedFiles.map((file) => (
                      <View key={file.path} style={styles.blockedFileRow}>
                        <Text style={[typeScale.monoMeta, styles.flex1, { color: tokens.ink }]} numberOfLines={1}>
                          {file.path}
                        </Text>
                        <Text style={[typeScale.meta, { color: tokens.ink3 }]} numberOfLines={1}>
                          {file.reason}
                        </Text>
                        <Pressable
                          onPress={() => toast.show("file review isn't wired up yet")}
                          accessibilityRole="button"
                          accessibilityLabel={`Review ${file.path}`}
                          hitSlop={8}
                        >
                          <Text style={[typeScale.meta, styles.reviewLink, { color: tokens.accent }]}>Review</Text>
                        </Pressable>
                      </View>
                    ))}
                  </View>
                </View>
              ) : null}
              <Text style={[typeScale.meta, styles.excludedNote, { color: tokens.ink4 }]}>
                {"Always excluded: .git, symlinks, device files, absolute or traversal paths, detected secrets, caches, files > 25 MB, capsules > 100 MB."}
              </Text>
            </>
          ) : null}

          <Button
            label="Create encrypted capsule"
            onPress={onCreateCapsule}
            disabled={!selectedHost || starting}
            fullWidth
            style={styles.primaryButton}
          />
          {selectedHost ? (
            <Text style={[typeScale.meta, styles.footerNote, { color: tokens.ink4 }]}>
              {`${sourceHostName} stays authoritative until ${selectedHost.name} acknowledges.`}
            </Text>
          ) : null}
        </View>
      ) : (
        <HandoffProgressView
          sessionTitle={sessionTitle}
          sourceHostName={sourceHostName}
          destHostName={selectedHost?.name ?? "destination"}
          plan={plan}
          stage={stage}
          onRetry={onRetry}
          onKeepOnSource={onClose}
        />
      )}
    </Sheet>
  );
}

function DestinationRow({
  host,
  isSource,
  selected,
  onSelect,
}: {
  host: AnywhereHost;
  isSource: boolean;
  selected: boolean;
  onSelect: () => void;
}) {
  const tokens = useTokens();
  const isStale = host.state.kind === "stale";

  return (
    <Pressable
      disabled={isSource}
      onPress={onSelect}
      accessibilityRole="radio"
      accessibilityState={{ selected, disabled: isSource }}
      accessibilityLabel={isSource ? `${host.name}, current host, source` : host.name}
      style={styles.hostRow}
    >
      <HostDot state={host.state} />
      <Text
        style={[typeScale.body, styles.flex1, { color: isSource ? tokens.ink3 : tokens.ink, fontWeight: selected ? "600" : "400" }]}
        numberOfLines={1}
      >
        {host.name}
      </Text>
      {isSource ? (
        <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>current host — source</Text>
      ) : isStale ? (
        <Text style={[typeScale.monoMeta, { color: tokens.warn }]}>stale — may miss the capsule</Text>
      ) : selected ? (
        <View style={[styles.checkBadge, { backgroundColor: tokens.accent }]}>
          <Check size={9} color={tokens.onAccent} strokeWidth={3.5} />
        </View>
      ) : null}
    </Pressable>
  );
}

type StageRowState = "done" | "active" | "pending";

function slotState(index: number, currentIndex: number): StageRowState {
  if (currentIndex > index) return "done";
  if (currentIndex === index) return "active";
  return "pending";
}

function StageRow({ state, label, meta, sub }: { state: StageRowState; label: string; meta?: string; sub?: string }) {
  const tokens = useTokens();
  const { dotStyle } = useEmberdot(state === "active" ? "busy" : "idle");

  return (
    <View>
      <View style={styles.stageRow}>
        {state === "done" ? (
          <View style={[styles.stageIconWrap, { backgroundColor: tokens.successBg }]}>
            <Check size={11} color={tokens.success} strokeWidth={3} />
          </View>
        ) : state === "active" ? (
          <View style={[styles.stageIconWrap, { backgroundColor: tokens.selection }]}>
            <Animated.View style={[styles.stageActiveDot, { backgroundColor: tokens.accent }, dotStyle]} />
          </View>
        ) : (
          <View style={[styles.stageIconWrap, styles.stageHollow, { borderColor: tokens.borderStrong }]} />
        )}
        <Text style={[typeScale.bodyBold, styles.flex1, { color: state === "pending" ? tokens.ink3 : tokens.ink }]} numberOfLines={1}>
          {label}
        </Text>
        {meta ? (
          <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink4 }]} numberOfLines={1}>
            {meta}
          </Text>
        ) : null}
      </View>
      {sub ? <Text style={[typeScale.meta, styles.stageSub, { color: tokens.ink3 }]}>{sub}</Text> : null}
    </View>
  );
}

function HandoffProgressView({
  sessionTitle,
  sourceHostName,
  destHostName,
  plan,
  stage,
  onRetry,
  onKeepOnSource,
}: {
  sessionTitle: string;
  sourceHostName: string;
  destHostName: string;
  plan: HandoffPlan | null;
  stage: HandoffStage;
  onRetry: () => void;
  onKeepOnSource: () => void;
}) {
  const tokens = useTokens();
  const failed = FAILURE_STAGES.includes(stage);
  const currentIndex = stage === "complete" ? PROGRESS_SLOTS.length : PROGRESS_SLOTS.indexOf(stage);
  // `plan.fileCount - blockedFiles.length` is the real packaged count, not a fabricated stat.
  const packagedFiles = plan ? plan.fileCount - plan.blockedFiles.length : null;

  return (
    <View style={styles.content}>
      <Text style={[typeScale.headingBold, { color: tokens.ink }]} numberOfLines={1}>
        {`Handing off to ${destHostName}`}
      </Text>
      <Text style={[typeScale.monoMeta, styles.progressMeta, { color: tokens.ink3 }]} numberOfLines={1}>
        {`${sessionTitle} · ${sourceHostName} → ${destHostName}${plan ? ` · ${plan.checkpoint}` : ""}`}
      </Text>

      <View style={styles.stagesWrap}>
        <StageRow
          state={slotState(0, currentIndex)}
          label="Capsule packaged"
          meta={packagedFiles != null && plan ? `${packagedFiles} files · ${formatBytes(plan.capsuleBytes)}` : undefined}
        />
        <StageRow state={slotState(1, currentIndex)} label="Uploaded to relay" meta="sealed" />
        <StageRow
          state={slotState(2, currentIndex)}
          label={`${destHostName} verifying base commit`}
          meta={plan?.baseCommit}
          sub={slotState(2, currentIndex) === "active" ? "Preparing an isolated worktree — your original checkout is untouched." : undefined}
        />
        <StageRow state={slotState(3, currentIndex)} label="Apply & import" />
        <StageRow state={slotState(4, currentIndex)} label="Acknowledgement" meta="ownership moves only after this" />
      </View>

      {!failed ? (
        <Text style={[typeScale.sub, styles.reassurance, { color: tokens.ink3 }]}>
          {`The session stays open. When ${destHostName} acknowledges, you continue right here — same chat, same tabs, new host.`}
        </Text>
      ) : (
        <View style={styles.failureWrap}>
          <HeatEdge state="waiting" />
          <View style={styles.failureInner}>
            <Text style={[typeScale.bodyBold, { color: tokens.danger }]}>{`Import failed on ${destHostName} · rolled back`}</Text>
            <Text style={[typeScale.sub, styles.failureBody, { color: tokens.ink2 }]}>
              {`The destination worktree was removed. ${sourceHostName} remains authoritative — nothing changed. Same for patch conflicts, missing commits, expiry, quota, interrupted transfer or acknowledgement timeout.`}
            </Text>
            <View style={styles.failureActions}>
              <Button label="Retry" onPress={onRetry} />
              <Button label={`Keep on ${sourceHostName}`} variant="secondary" onPress={onKeepOnSource} />
            </View>
          </View>
        </View>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space20, paddingBottom: space.space24, gap: space.space4 },
  flex1: { flex: 1, minWidth: 0 },
  headerRow: { flexDirection: "row", alignItems: "center", gap: space.space8, minHeight: tapTarget },
  checkpointRow: { flexDirection: "row", alignItems: "center", gap: space.space8, marginTop: space.space4 },
  smallCheckWrap: { width: 16, height: 16, borderRadius: 8, alignItems: "center", justifyContent: "center", flexShrink: 0 },
  checkpointNote: { marginLeft: 24, marginBottom: space.space8 },
  hostRow: { flexDirection: "row", alignItems: "center", gap: space.space12, minHeight: tapTarget, paddingVertical: space.space4 },
  checkBadge: { width: 17, height: 17, borderRadius: radii.radiusPill, alignItems: "center", justifyContent: "center" },
  preflightRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space8 },
  blockedWrap: { position: "relative", paddingLeft: space.space12, marginTop: space.space4 },
  blockedInner: { gap: space.space8 },
  blockedFileRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  reviewLink: { fontWeight: "600" },
  excludedNote: { marginTop: space.space8, lineHeight: 16 },
  primaryButton: { marginTop: space.space16 },
  footerNote: { textAlign: "center", paddingVertical: space.space8 },
  stagesWrap: { marginTop: space.space16, gap: 0 },
  stageRow: { flexDirection: "row", alignItems: "center", gap: space.space12, paddingVertical: space.space8, minHeight: tapTarget },
  stageIconWrap: { width: 20, height: 20, borderRadius: 10, alignItems: "center", justifyContent: "center", flexShrink: 0 },
  stageHollow: { borderWidth: 2 },
  stageActiveDot: { width: 8, height: 8, borderRadius: 4 },
  stageSub: { marginLeft: 32, marginBottom: space.space4, lineHeight: 16 },
  progressMeta: { marginTop: 2 },
  reassurance: { marginTop: space.space20, lineHeight: 19 },
  failureWrap: { position: "relative", paddingLeft: space.space12, marginTop: space.space20 },
  failureInner: { gap: space.space8 },
  failureBody: { lineHeight: 18 },
  failureActions: { flexDirection: "row", gap: space.space8 },
});
