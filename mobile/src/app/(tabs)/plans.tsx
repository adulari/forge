// Native Features pack — "NF Plans" / "NF Desktop Plans". Cross-session plans awaiting
// review. The featured (first) plan renders as the Hearth decision card (pattern 7):
// dash-marked "Plan" header, per-step medallions, Approve / Revise / Cancel with an
// inline revise box. Remaining plans are de-boxed rows that expand their steps inline.
//
// LIVE DATA:
// - `usePlans()` (REST) lists every session's awaiting-review plan: {session_id,
//   session_title, title, steps[], notes}. It carries NO per-step state and NO approval
//   channel (no prompt_seq / send).
// - To make the featured card actually approvable, it attaches a short-lived session
//   socket to that plan's session (same pattern as DecisionCard / DecisionPeek). That
//   snapshot supplies the live `plan`, the plan-approval `question` + `prompt_seq`, and
//   `tasks`. Per-step medallion state is DERIVED by matching each plan step's title to a
//   `snapshot.tasks` entry (case-insensitive substring); unmatched steps fall back to
//   "pending" (heuristic — the wire has no step→status mapping). When the socket has no
//   live approval question (plan already executing, or not connected), the card degrades
//   to read-only steps + an "Open session" action rather than faking inline approval.
import { router } from "expo-router";
import { Check, ChevronDown, ClipboardList, Send } from "lucide-react-native";
import React, { useEffect, useMemo, useState } from "react";
import { Pressable, RefreshControl, StyleSheet, Text, View, type ViewStyle } from "react-native";

import { BackLink } from "../../components/ds/BackLink";
import { Button } from "../../components/ds/Button";
import { Card } from "../../components/ds/Card";
import { EmptyState } from "../../components/ds/EmptyState";
import { IconButton } from "../../components/ds/IconButton";
import { Input } from "../../components/ds/Input";
import { Screen } from "../../components/ds/Screen";
import { SectionHeader } from "../../components/ds/SectionHeader";
import { StatusDot } from "../../components/ds/StatusDot";
import { useToast } from "../../components/ds/ToastHost";
import { DesktopDrillDown } from "../../components/fleet/DesktopDrillDown";
import { useAuth } from "../../lib/auth";
import { haptics } from "../../lib/haptics";
import { type PlanRow } from "../../lib/api";
import { usePlans } from "../../lib/queries";
import { type QuestionOption, type SnapshotTask, useSessionSocket } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space, tapTarget } from "../../theme/tokens";
import { tabularNums, type } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";
import { SettingsShell } from "./settings";

type StepState = "done" | "running" | "pending";

function normalize(value: string): string {
  return value.trim().toLowerCase();
}

/** Heuristic: match a plan step to a live task by title, then map its status. */
function stepStateFor(title: string, tasks: SnapshotTask[]): StepState {
  const target = normalize(title);
  const match = tasks.find((task) => {
    const candidate = normalize(task.title);
    return candidate === target || candidate.includes(target) || target.includes(candidate);
  });
  if (!match) return "pending";
  return match.status === "done" ? "done" : match.status === "in_progress" ? "running" : "pending";
}

function findOptionNumber(options: QuestionOption[], pattern: RegExp, fallback: string): string {
  const idx = options.findIndex((option) => pattern.test(option.label));
  return idx >= 0 ? String(idx + 1) : fallback;
}

function StepMedallion({ state }: { state: StepState }) {
  const tokens = useTokens();
  if (state === "done") {
    return (
      <View style={[styles.medallion, { backgroundColor: tokens.successBg }]}>
        <Check size={10} strokeWidth={3.2} color={tokens.success} />
      </View>
    );
  }
  if (state === "running") {
    return (
      <View style={[styles.medallion, { backgroundColor: tokens.selection }]}>
        <StatusDot state="busy" size={6} accessibilityLabel="step running" />
      </View>
    );
  }
  return <View style={[styles.medallionPending, { borderColor: tokens.borderStrong }]} />;
}

function FeaturedPlanCard({ plan }: { plan: PlanRow }) {
  const tokens = useTokens();
  const toast = useToast();
  const { baseUrl } = useAuth();
  const { snapshot, send } = useSessionSocket(baseUrl, plan.session_id);

  const [revising, setRevising] = useState(false);
  const [reviseText, setReviseText] = useState("");
  const [lockedSeq, setLockedSeq] = useState<number | null>(null);

  const livePlan = snapshot?.plan ?? null;
  const steps = livePlan?.steps ?? plan.steps;
  const tasks = snapshot?.tasks ?? [];
  const question = snapshot?.question ?? null;
  const options = snapshot?.question_options ?? [];
  const promptSeq = snapshot?.prompt_seq ?? 0;
  const canApprove = question != null && livePlan != null;

  useEffect(() => {
    setLockedSeq(null);
    setRevising(false);
    setReviseText("");
  }, [promptSeq]);

  const locked = lockedSeq != null && lockedSeq === promptSeq;

  const answer = (text: string, haptic: () => void) => {
    if (locked || text.trim().length === 0) return;
    setLockedSeq(promptSeq);
    haptic();
    if (!send({ kind: "answer", text, seq: promptSeq })) {
      setLockedSeq(null);
      toast.show("not sent — reconnect and try again", { tone: "danger" });
    }
  };

  const approveNumber = findOptionNumber(options, /build/i, "1");
  const cancelNumber = findOptionNumber(options, /cancel/i, "2");

  return (
    <Card heatEdge="waiting" style={styles.featured}>
      <View style={styles.featuredHeader}>
        <View style={[styles.dash, { backgroundColor: tokens.accent }]} />
        <Text style={[type.section, { color: tokens.accent }]}>Plan</Text>
        <View style={styles.spacer} />
        <Text style={[type.monoMeta, tabularNums, { color: tokens.ink4 }]} numberOfLines={1}>
          {plan.session_title || plan.session_id}
        </Text>
      </View>

      <Text style={[styles.featuredTitle, { color: tokens.ink }]}>{plan.title}</Text>

      <View style={styles.steps}>
        {steps.map((step, index) => {
          const state = stepStateFor(step.title, tasks);
          return (
            <View key={`${step.title}-${index}`} style={styles.step}>
              <StepMedallion state={state} />
              <View style={styles.stepBody}>
                <Text
                  style={[
                    type.body,
                    state === "done"
                      ? { color: tokens.ink3, textDecorationLine: "line-through" }
                      : state === "running"
                        ? [type.bodyBold, { color: tokens.ink }]
                        : { color: tokens.ink },
                  ]}
                >
                  {step.title}
                </Text>
                {step.detail ? <Text style={[type.monoMeta, styles.stepDetail, { color: tokens.ink4 }]}>{step.detail}</Text> : null}
              </View>
            </View>
          );
        })}
      </View>

      {canApprove ? (
        <>
          <View style={styles.actions}>
            <Button
              label="Approve"
              variant="allow"
              onPress={() => answer(approveNumber, haptics.allow)}
              disabled={locked}
              style={styles.approve}
            />
            <Button label="Revise" variant="ghost" onPress={() => setRevising((value) => !value)} disabled={locked} />
            <Button label="Cancel" variant="ghost" onPress={() => answer(cancelNumber, haptics.deny)} disabled={locked} />
          </View>
          {revising ? (
            <View style={styles.reviseRow}>
              <Input
                value={reviseText}
                onChangeText={setReviseText}
                placeholder="what should change?"
                editable={!locked}
                onSubmitEditing={() => answer(reviseText, haptics.select)}
                returnKeyType="send"
                containerStyle={styles.reviseInput}
                accessibilityLabel="plan revision"
              />
              <IconButton
                icon={<Send size={20} strokeWidth={1.75} color={tokens.ink} />}
                onPress={() => answer(reviseText, haptics.select)}
                disabled={locked || reviseText.trim().length === 0}
                accessibilityLabel="send revision"
              />
            </View>
          ) : null}
        </>
      ) : (
        <View style={styles.actions}>
          <Button
            label="Open session to review"
            variant="secondary"
            onPress={() => router.push(`/session/${plan.session_id}`)}
            style={styles.approve}
          />
        </View>
      )}
    </Card>
  );
}

function OpenPlanRow({ plan, expanded, onToggle, showSeparator }: { plan: PlanRow; expanded: boolean; onToggle: () => void; showSeparator: boolean }) {
  const tokens = useTokens();
  const separator: ViewStyle | undefined = showSeparator ? { borderBottomColor: tokens.hairline, borderBottomWidth: StyleSheet.hairlineWidth } : undefined;
  return (
    <View style={[styles.openRow, separator]}>
      <Pressable onPress={onToggle} accessibilityRole="button" accessibilityState={{ expanded }} accessibilityLabel={plan.title} style={styles.openRowHead}>
        <View style={styles.openRowText}>
          <Text style={[type.bodyBold, { color: tokens.ink }]} numberOfLines={expanded ? undefined : 1}>
            {plan.title}
          </Text>
          <Text style={[type.monoMeta, tabularNums, styles.openRowMeta, { color: tokens.ink4 }]} numberOfLines={1}>
            {plan.session_title || plan.session_id}
          </Text>
        </View>
        <Text style={[type.monoMeta, tabularNums, { color: tokens.ink4 }]}>{`${plan.steps.length} step${plan.steps.length === 1 ? "" : "s"}`}</Text>
        <ChevronDown size={16} strokeWidth={1.75} color={tokens.ink3} style={expanded ? styles.chevronOpen : undefined} />
      </Pressable>
      {expanded ? (
        <View style={styles.openRowSteps}>
          {plan.steps.map((step, index) => (
            <View key={`${step.title}-${index}`} style={styles.openStep}>
              <Text style={[type.monoMeta, tabularNums, styles.openStepIndex, { color: tokens.ink4 }]}>{index + 1}</Text>
              <View style={styles.stepBody}>
                <Text style={[type.body, { color: tokens.ink }]}>{step.title}</Text>
                {step.detail ? <Text style={[type.monoMeta, styles.stepDetail, { color: tokens.ink4 }]}>{step.detail}</Text> : null}
              </View>
            </View>
          ))}
          {plan.notes ? <Text style={[type.sub, styles.openNotes, { color: tokens.ink3, borderLeftColor: tokens.border }]}>{plan.notes}</Text> : null}
          <Pressable onPress={() => router.push(`/session/${plan.session_id}`)} accessibilityRole="button" style={styles.openLink}>
            <Text style={[type.meta, { color: tokens.accent }]}>Open session</Text>
          </Pressable>
        </View>
      ) : null}
    </View>
  );
}

function PlansScreenBody() {
  const tokens = useTokens();
  const { isCompact } = useBreakpoint();
  const query = usePlans();
  const plans = query.data ?? [];
  const featured = plans[0] ?? null;
  const rest = plans.slice(1);
  const [expanded, setExpanded] = useState<string | null>(null);

  const awaitingLabel = useMemo(() => `${plans.length} awaiting review`, [plans.length]);

  const openList = (
    <View>
      <SectionHeader>{isCompact ? "open plans · other sessions" : "open plans · all sessions"}</SectionHeader>
      {rest.length === 0 ? (
        <Text style={[type.sub, styles.openEmpty, { color: tokens.ink4 }]}>No other plans awaiting review.</Text>
      ) : (
        rest.map((plan, index) => (
          <OpenPlanRow
            key={plan.session_id}
            plan={plan}
            expanded={expanded === plan.session_id}
            onToggle={() => setExpanded((current) => (current === plan.session_id ? null : plan.session_id))}
            showSeparator={index < rest.length - 1}
          />
        ))
      )}
      <Text style={[type.meta, styles.footnote, { color: tokens.ink4 }]}>
        Approved plans start executing immediately and report per-step state here.
      </Text>
    </View>
  );

  let content: React.ReactNode;
  if (query.isError && !query.data) {
    content = <Text style={[type.body, { color: tokens.danger }]}>Could not load plans. Pull to retry.</Text>;
  } else if (query.isLoading) {
    content = <Text style={[type.sub, { color: tokens.ink3 }]}>Loading plans…</Text>;
  } else if (plans.length === 0) {
    content = (
      <EmptyState
        icon={ClipboardList}
        message="No plans awaiting review. Proposed work appears here while a session waits for your approval."
      />
    );
  } else if (!isCompact) {
    content = (
      <View style={styles.twoCol}>
        <View style={styles.colMain}>{featured ? <FeaturedPlanCard plan={featured} /> : null}</View>
        <View style={styles.colSide}>{openList}</View>
      </View>
    );
  } else {
    content = (
      <View style={styles.stack}>
        {featured ? <FeaturedPlanCard plan={featured} /> : null}
        {openList}
      </View>
    );
  }

  return (
    <Screen scroll refreshControl={<RefreshControl refreshing={query.isFetching} onRefresh={() => void query.refetch()} />} contentContainerStyle={styles.content}>
      <BackLink />
      <View style={styles.titleRow}>
        <Text style={[type.title, styles.title, { color: tokens.ink }]}>Plans</Text>
        <Text style={[type.monoMeta, tabularNums, { color: tokens.ink3 }]}>{awaitingLabel}</Text>
      </View>
      {content}
    </Screen>
  );
}

export default function PlansScreen() {
  return (
    <DesktopDrillDown>
      <SettingsShell active="plans">
        <PlansScreenBody />
      </SettingsShell>
    </DesktopDrillDown>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 },
  titleRow: { flexDirection: "row", alignItems: "baseline", justifyContent: "space-between", gap: space.space8 },
  title: { flexShrink: 1 },
  twoCol: { flexDirection: "row", gap: space.space32, alignItems: "flex-start" },
  colMain: { flex: 1.1, minWidth: 0 },
  colSide: { flex: 0.9, minWidth: 0 },
  stack: { gap: space.space20 },

  featured: { gap: space.space12 },
  featuredHeader: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  dash: { width: 6, height: 2 },
  spacer: { flex: 1 },
  featuredTitle: { fontSize: 15.5, lineHeight: 22, fontWeight: "600" },
  steps: { gap: space.space12 },
  step: { flexDirection: "row", gap: space.space8, alignItems: "flex-start" },
  stepBody: { flex: 1, gap: space.space2 },
  stepDetail: { marginTop: space.space2 },
  medallion: { width: 18, height: 18, borderRadius: 9, alignItems: "center", justifyContent: "center", marginTop: 2 },
  medallionPending: { width: 16, height: 16, borderRadius: 8, borderWidth: 2, marginTop: 3, marginLeft: 1 },
  actions: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  approve: { flex: 1 },
  reviseRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  reviseInput: { flex: 1 },

  openRow: { paddingVertical: space.space12 },
  openRowHead: { flexDirection: "row", alignItems: "center", gap: space.space8, minHeight: tapTarget },
  openRowText: { flex: 1, minWidth: 0 },
  openRowMeta: { marginTop: space.space2 },
  chevronOpen: { transform: [{ rotate: "180deg" }] },
  openRowSteps: { gap: space.space8, marginTop: space.space8, paddingLeft: space.space4 },
  openStep: { flexDirection: "row", gap: space.space8 },
  openStepIndex: { width: 14, flexShrink: 0, paddingTop: 2 },
  openNotes: { lineHeight: 19, paddingLeft: space.space12, borderLeftWidth: 2 },
  openLink: { alignSelf: "flex-start", minHeight: 32, justifyContent: "center" },
  openEmpty: { paddingVertical: space.space12 },
  footnote: { marginTop: space.space16, lineHeight: 17 },
});
