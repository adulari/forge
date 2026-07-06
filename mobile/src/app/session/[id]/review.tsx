// Review segment (BUILD_PLAN §6 "Review" / §7 Batch 3 W8). Plan card mirrors app.js's
// `renderPlan` (present_plan proposal; while the turn-end approval question is pending with a
// "Build it" option, Approve/Revise/Cancel ANSWER that question by 1-based option number — same
// seq-checked path a tapped option takes). Diff card mirrors `renderDiff` (either the ONE
// pending-permission change or everything that landed this turn). Content only — the session
// shell owns the Screen header/status strip (UI_RULES.md #1-#2).
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { KeyboardAvoidingView, Platform, Pressable, ScrollView, Text, TextInput, View } from "react-native";

import {
  Badge,
  type BadgeTone,
  BoundedList,
  Card,
  Chip,
  ConfirmButton,
  EmptyState,
  Loading,
  PrimaryButton,
} from "../../../components/ui";
import { theme } from "../../../lib/theme";
import { useSessionCtx } from "../../../lib/sessionContext";
import type { DiffFile, Plan, QuestionOption, RemoteInput } from "../../../lib/ws";

const monoStyle = {
  fontFamily: Platform.select({ ios: "Menlo", android: "monospace", default: "ui-monospace" }),
  fontSize: 12,
  lineHeight: 18,
} as const;

export function optionNumber(options: QuestionOption[], label: string): number {
  const i = options.findIndex((o) => o.label === label);
  return i < 0 ? 0 : i + 1;
}

// ---------------------------------------------------------------------------
// Plan card
// ---------------------------------------------------------------------------

interface PlanCardProps {
  plan: Plan;
  question: string | null;
  questionOptions: QuestionOption[];
  promptSeq: number;
  send: (input: RemoteInput) => void;
}

function PlanCardBase({ plan, question, questionOptions, promptSeq, send }: PlanCardProps) {
  const approveN = question ? optionNumber(questionOptions, "Build it") : 0;
  const cancelN = question ? optionNumber(questionOptions, "Cancel") : 0;
  const decidable = approveN > 0;

  // prompt_seq discipline (UI_RULES.md #16): disable the card's buttons after the first tap
  // until a new snapshot (new prompt_seq) arrives — never retry a stale/ignored answer.
  const [answeredSeq, setAnsweredSeq] = useState<number | null>(null);
  const [reviseOpen, setReviseOpen] = useState(false);
  const [reviseText, setReviseText] = useState("");

  const lastPlanTitleRef = useRef(plan.title);
  useEffect(() => {
    if (lastPlanTitleRef.current !== plan.title) {
      lastPlanTitleRef.current = plan.title;
      setReviseOpen(false);
      setReviseText("");
      setAnsweredSeq(null);
    }
  }, [plan.title]);

  const locked = answeredSeq === promptSeq;

  const onApprove = useCallback(() => {
    if (approveN <= 0) return;
    send({ kind: "answer", text: String(approveN), seq: promptSeq });
    setAnsweredSeq(promptSeq);
  }, [approveN, promptSeq, send]);

  const onCancel = useCallback(() => {
    if (cancelN <= 0) return;
    send({ kind: "answer", text: String(cancelN), seq: promptSeq });
    setAnsweredSeq(promptSeq);
  }, [cancelN, promptSeq, send]);

  const onToggleRevise = useCallback(() => {
    setReviseOpen((open) => {
      const next = !open;
      if (next && !reviseText) setReviseText("revise the plan: ");
      return next;
    });
  }, [reviseText]);

  const onSubmitRevise = useCallback(() => {
    const value = reviseText.trim();
    if (!value) return;
    send({ kind: "answer", text: value, seq: promptSeq });
    setAnsweredSeq(promptSeq);
    setReviseOpen(false);
    setReviseText("");
  }, [reviseText, promptSeq, send]);

  return (
    <Card variant="feature" className="gap-8">
      <Text className="text-accent text-[12px] font-bold">⬡ PLAN</Text>
      <Text className="text-ink text-[15px] font-bold">{plan.title}</Text>
      <View className="gap-6">
        {plan.steps.map((step, i) => (
          <View key={i} className="flex-row gap-6">
            <Text className="text-dim text-[13px]">{i + 1}.</Text>
            <View className="flex-1">
              <Text className="text-ink text-[14px] font-semibold">{step.title}</Text>
              {step.detail ? (
                <Text className="text-dim text-[13px] mt-2">{step.detail}</Text>
              ) : null}
            </View>
          </View>
        ))}
      </View>
      {plan.notes ? <Text className="text-notes text-[13px]">⚠ {plan.notes}</Text> : null}

      {decidable ? (
        <View className="gap-8">
          <View className="flex-row gap-8">
            <View className="flex-1">
              <ConfirmButton label="Approve & build" tone="ok" onPress={onApprove} disabled={locked} />
            </View>
            {cancelN > 0 ? (
              <View className="flex-1">
                <ConfirmButton label="Cancel" tone="no" onPress={onCancel} disabled={locked} />
              </View>
            ) : null}
          </View>
          <Chip label="Revise" selected={reviseOpen} onPress={onToggleRevise} disabled={locked} />
          {reviseOpen ? (
            <View className="gap-8">
              <TextInput
                value={reviseText}
                onChangeText={setReviseText}
                placeholder="describe the revision…"
                placeholderTextColor={theme.colors.dim}
                multiline
                className="bg-panelDeep border border-border rounded-md px-10 py-8 text-ink text-[14px]"
                style={{ minHeight: 44, maxHeight: 120 }}
              />
              <PrimaryButton
                label="Submit revision"
                onPress={onSubmitRevise}
                disabled={locked || !reviseText.trim()}
                fullWidth={false}
              />
            </View>
          ) : null}
        </View>
      ) : question ? (
        // Question is pending but doesn't belong to this plan (no "Build it" option) — a
        // one-line pointer only, mirroring app.js's suppression of the duplicate generic card.
        <Text className="text-dim text-[13px]">⬡ {question}</Text>
      ) : null}
    </Card>
  );
}

// Snapshots resend `plan`/`question_options` fresh on every frame even when unchanged — content
// compare (same trick as DiffFileRow below) so re-renders track real changes, not websocket ticks.
const PlanCard = React.memo(PlanCardBase, (prev, next) => {
  return (
    prev.promptSeq === next.promptSeq &&
    prev.question === next.question &&
    JSON.stringify(prev.plan) === JSON.stringify(next.plan) &&
    JSON.stringify(prev.questionOptions) === JSON.stringify(next.questionOptions)
  );
});

// ---------------------------------------------------------------------------
// Diff card
// ---------------------------------------------------------------------------

const DiffHeaderBase = ({ pending }: { pending: boolean }) => (
  <Text
    className={
      pending
        ? "text-accent text-[12px] font-bold px-2"
        : "text-dim text-[12px] font-bold uppercase tracking-[0.5px] px-2"
    }
  >
    {pending ? "⚠ proposed change — review before allowing" : "✎ changes this turn"}
  </Text>
);
const DiffHeader = React.memo(DiffHeaderBase);

const kindTone: Record<DiffFile["kind"], BadgeTone> = {
  created: "ok",
  modified: "default",
  deleted: "no",
};

function DiffFileRowBase({ file }: { file: DiffFile }) {
  const [expanded, setExpanded] = useState(true);
  return (
    <Card variant="feature" className="gap-6">
      <Pressable
        onPress={() => setExpanded((e) => !e)}
        className="flex-row items-center gap-8"
        style={{ minHeight: 44 }}
      >
        <Text className="text-dim text-[12px]">{expanded ? "▾" : "▸"}</Text>
        <Text
          numberOfLines={1}
          ellipsizeMode="head"
          className="flex-1 text-ink text-[13px] font-semibold"
        >
          {file.path}
        </Text>
        <Badge label={file.kind} tone={kindTone[file.kind]} />
        <Badge label={`+${file.adds || 0}`} tone="ok" />
        <Badge label={`−${file.dels || 0}`} tone="no" />
      </Pressable>

      {expanded ? (
        file.binary ? (
          <Text className="text-dim text-[12px] px-2">binary file</Text>
        ) : (
          <View className="bg-codeBg rounded-md overflow-hidden">
            <ScrollView horizontal showsHorizontalScrollIndicator={false}>
              <View className="px-8 py-6">
                {file.hunks.map((hunk, hi) => (
                  <View key={hi} className={hi > 0 ? "mt-6" : undefined}>
                    <Text className="text-hunk" style={monoStyle}>
                      {hunk.header}
                    </Text>
                    {hunk.lines.map((line, li) => (
                      <Text
                        key={li}
                        className={
                          line[0] === "+"
                            ? "text-ok"
                            : line[0] === "-"
                              ? "text-no"
                              : "text-dim"
                        }
                        style={monoStyle}
                      >
                        {line}
                      </Text>
                    ))}
                  </View>
                ))}
              </View>
            </ScrollView>
          </View>
        )
      ) : null}

      {expanded && file.skipped_lines ? (
        <Text className="text-dim text-[12px] px-2">
          … +{file.skipped_lines} more lines (full diff in the TUI)
        </Text>
      ) : null}
    </Card>
  );
}

// Snapshots resend the full diff on every frame (not a delta), so `file` is a fresh object
// each websocket tick even when nothing changed. Content-compare (mirrors app.js's own
// `box._sig = JSON.stringify(d)` skip-render trick) so a busy session doesn't re-render every
// hunk/line on every frame — the actual perf win for "60fps on big diffs" (UI_RULES.md #26-28).
const DiffFileRow = React.memo(
  DiffFileRowBase,
  (prev, next) => JSON.stringify(prev.file) === JSON.stringify(next.file),
);

// ---------------------------------------------------------------------------
// Screen: one virtualized list — plan card, diff header, one row per file, diff footer.
// ---------------------------------------------------------------------------

type ReviewRow =
  | { kind: "plan"; plan: Plan; question: string | null; questionOptions: QuestionOption[]; promptSeq: number }
  | { kind: "diffHeader"; pending: boolean }
  | { kind: "diffFile"; file: DiffFile }
  | { kind: "diffFooter"; skippedFiles: number };

function reviewRowKey(row: ReviewRow): string {
  return row.kind === "diffFile" ? `file:${row.file.path}` : row.kind;
}

export default function ReviewScreen() {
  const { snapshot, send } = useSessionCtx();

  const rows = useMemo<ReviewRow[]>(() => {
    if (!snapshot) return [];
    const out: ReviewRow[] = [];
    if (snapshot.plan) {
      out.push({
        kind: "plan",
        plan: snapshot.plan,
        question: snapshot.question,
        questionOptions: snapshot.question_options,
        promptSeq: snapshot.prompt_seq,
      });
    }
    const files = snapshot.diff?.files ?? [];
    if (files.length) {
      out.push({ kind: "diffHeader", pending: snapshot.diff?.pending ?? false });
      for (const file of files) out.push({ kind: "diffFile", file });
      if (snapshot.diff?.skipped_files) {
        out.push({ kind: "diffFooter", skippedFiles: snapshot.diff.skipped_files });
      }
    }
    return out;
  }, [snapshot]);

  const renderItem = useCallback(
    ({ item }: { item: ReviewRow }) => {
      switch (item.kind) {
        case "plan":
          return (
            <PlanCard
              plan={item.plan}
              question={item.question}
              questionOptions={item.questionOptions}
              promptSeq={item.promptSeq}
              send={send}
            />
          );
        case "diffHeader":
          return <DiffHeader pending={item.pending} />;
        case "diffFile":
          return <DiffFileRow file={item.file} />;
        case "diffFooter":
          return (
            <Text className="text-dim text-[12px] px-2">
              … +{item.skippedFiles} more files
            </Text>
          );
        default:
          return <></>;
      }
    },
    [send],
  );

  const emptyComponent = useMemo(() => <EmptyState title="Nothing to review." />, []);

  if (!snapshot) {
    return (
      <View className="flex-1">
        <Loading label="Connecting to session…" />
      </View>
    );
  }

  return (
    <KeyboardAvoidingView
      className="flex-1"
      behavior={Platform.OS === "ios" ? "padding" : undefined}
      keyboardVerticalOffset={Platform.OS === "ios" ? 8 : 0}
    >
      <BoundedList
        data={rows}
        keyExtractor={reviewRowKey}
        renderItem={renderItem}
        ListEmptyComponent={emptyComponent}
        contentContainerStyle={{ paddingTop: 4, paddingBottom: 16, gap: 10 }}
      />
    </KeyboardAvoidingView>
  );
}
