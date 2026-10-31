// Hearth Fork sheet — a parallel line from any point; the original keeps running. The child can
// pin its own model, effort and access and start with a first prompt; "Move" forks and then
// archives the source, so a thread moves to another model instead of branching.
import { router } from "expo-router";
import { ChevronDown, GitBranch } from "lucide-react-native";
import React, { useMemo, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import type { HistoryRow } from "../../lib/api";
import { ApiError, isUnsupportedForkFieldError } from "../../lib/api";
import { FORK_EFFORTS, FORK_TEMPERS, buildForkRequest, type ForkEffort, type ForkTemper } from "../../lib/forkRequest";
import { useArchiveSession, useForkSession, useHistory, useRenameSession, useSessions } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Chip } from "../ds/Chip";
import { ModelPicker } from "./ModelPicker";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";


function preview(row: HistoryRow) {
  return row.content.replace(/\s+/g, " ").trim();
}

function worktreeLabel(path: string): string {
  const seg = path.replace(/\/+$/, "").split("/").pop() ?? path;
  return seg.length > 16 ? `${seg.slice(0, 15)}…` : seg;
}

export function ForkSheet({
  visible,
  onClose,
  sessionId,
  onForked,
}: {
  visible: boolean;
  onClose: () => void;
  sessionId: string;
  /** Where the new session opens; defaults to pushing its route (pane 0). */
  onForked?: (id: string) => void;
}) {
  const tokens = useTokens();
  const history = useHistory(sessionId);
  const sessions = useSessions();
  const fork = useForkSession();
  const rename = useRenameSession();
  const archive = useArchiveSession();
  const [name, setName] = useState("");
  const [model, setModel] = useState("");
  const [effort, setEffort] = useState<ForkEffort | null>(null);
  const [temper, setTemper] = useState<ForkTemper | null>(null);
  const [prompt, setPrompt] = useState("");
  const [pending, setPending] = useState<"fork" | "move" | null>(null);
  const [selectedSeq, setSelectedSeq] = useState<number | null>(null);
  const [pickerOpen, setPickerOpen] = useState(false);

  // Only the person's own prompts are fork points: a harness nudge is stored `role='user'` for
  // the model but its `kind` says it was Forge talking ("Your last response was empty…").
  const prompts = useMemo(
    () =>
      history.data?.pages
        .flat()
        .filter((row) => row.role === "user" && row.kind !== "nudge" && row.kind !== "job_notice" && row.kind !== "harness_notice")
        .slice()
        .reverse() ?? [],
    [history.data?.pages],
  );
  // Default to the latest prompt — forking "from here" is the common case — not the first one.
  const base = useMemo(() => {
    if (prompts.length === 0) return null;
    return prompts.find((row) => row.seq === selectedSeq) ?? prompts[prompts.length - 1];
  }, [prompts, selectedSeq]);
  const baseSession = useMemo(() => sessions.data?.find((s) => s.id === sessionId) ?? null, [sessions.data, sessionId]);

  const close = () => {
    setPickerOpen(false);
    onClose();
  };
  const reset = () => {
    setName("");
    setModel("");
    setEffort(null);
    setTemper(null);
    setPrompt("");
    setSelectedSeq(null);
    setPending(null);
  };
  const startFork = (mode: "fork" | "move") => {
    if (!base) return;
    setPending(mode);
    const draft = { atSeq: base.seq, model: model || null, effort, temper, prompt, title: name };
    const finish = (session: { id: string }, renameTo: string) => {
      // A daemon from before the extended fork body only took `at_seq`: the name is then applied
      // with the session menu's rename call, and the extra fields are dropped.
      if (renameTo) rename.mutate({ id: session.id, title: renameTo });
      if (mode === "move") archive.mutate(sessionId);
      reset();
      close();
      if (onForked) onForked(session.id);
      else router.push(`/session/${session.id}`);
    };
    fork.mutate(
      { id: sessionId, body: buildForkRequest(draft) },
      {
        onSuccess: (session) => finish(session, ""),
        onError: (error) => {
          if (!isUnsupportedForkFieldError(error)) {
            setPending(null);
            return;
          }
          fork.mutate(
            { id: sessionId, body: { at_seq: base.seq } },
            { onSuccess: (session) => finish(session, name.trim()), onError: () => setPending(null) },
          );
        },
      },
    );
  };
  const cancel = () => {
    reset();
    close();
  };

  return (
    <Sheet visible={visible} onClose={cancel} accessibilityLabel="Fork session" snapPoints={[0.92]}>
      <View style={styles.content}>
        <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Fork session</Text>
        <Text style={[typeScale.sub, styles.subtitle, { color: tokens.ink3 }]}>
          A parallel line from any point — the original keeps running.
        </Text>

        <Text style={[typeScale.meta, styles.fieldLabel, { color: tokens.ink3 }]}>Name</Text>
        <Input
          value={name}
          onChangeText={setName}
          placeholder="retry-loop experiment"
          autoCapitalize="sentences"
          numberOfLines={1}
          accessibilityLabel="Fork name"
          returnKeyType="done"
        />

        <Text style={[typeScale.meta, styles.fieldLabel, { color: tokens.ink3 }]}>Fork from</Text>
        {base ? (
          <Pressable
            onPress={() => setPickerOpen((open) => !open)}
            accessibilityRole="button"
            accessibilityState={{ expanded: pickerOpen }}
            accessibilityLabel={`Base point: ${preview(base)}`}
            style={[styles.baseRow, { backgroundColor: tokens.bg0, borderColor: pickerOpen ? tokens.accent : tokens.border }]}
          >
            <Text style={[typeScale.body, styles.baseText, { color: tokens.ink }]} numberOfLines={1}>{preview(base)}</Text>
            <Text style={[styles.mono, tabularNums, { color: tokens.ink4 }]}>msg {base.seq}</Text>
            <ChevronDown size={15} strokeWidth={1.75} color={tokens.ink3} style={pickerOpen ? styles.chevronOpen : undefined} />
          </Pressable>
        ) : (
          <Text style={[typeScale.sub, styles.helper, { color: tokens.ink3 }]}>
            No prompts to fork from yet — send a message first.
          </Text>
        )}

        {pickerOpen && base ? (
          <ScrollView style={styles.picker} keyboardShouldPersistTaps="handled" nestedScrollEnabled>
            {prompts.map((row, index) => {
              const active = row.seq === base.seq;
              return (
                <Pressable
                  key={row.seq}
                  onPress={() => {
                    setSelectedSeq(row.seq);
                    setPickerOpen(false);
                  }}
                  accessibilityRole="radio"
                  accessibilityState={{ selected: active }}
                  style={[
                    styles.pickerRow,
                    index > 0 ? { borderTopColor: tokens.hairline, borderTopWidth: StyleSheet.hairlineWidth } : null,
                    active ? { backgroundColor: tokens.selection } : null,
                  ]}
                >
                  <Text style={[typeScale.sub, styles.baseText, { color: active ? tokens.ink : tokens.ink2 }]} numberOfLines={1}>{preview(row)}</Text>
                  <Text style={[styles.mono, tabularNums, { color: tokens.ink4 }]}>msg {row.seq}</Text>
                </Pressable>
              );
            })}
          </ScrollView>
        ) : null}

        <Text style={[typeScale.meta, styles.fieldLabel, { color: tokens.ink3 }]}>Model</Text>
        <ModelPicker value={model} onChange={setModel} />
        {!model ? (
          <Text style={[typeScale.sub, { color: tokens.ink3 }]}>Leave on Automatic to keep this session’s model.</Text>
        ) : null}

        <Text style={[typeScale.meta, styles.fieldLabel, { color: tokens.ink3 }]}>Effort</Text>
        <View style={styles.chips} accessibilityRole="radiogroup" accessibilityLabel="Fork effort">
          <Chip label="Same" selected={effort === null} onPress={() => setEffort(null)} />
          {FORK_EFFORTS.map((level) => (
            <Chip key={level} label={level === "whitehot" ? "Whitehot" : level} selected={effort === level} onPress={() => setEffort(level)} />
          ))}
        </View>

        <Text style={[typeScale.meta, styles.fieldLabel, { color: tokens.ink3 }]}>Access</Text>
        <View style={styles.chips} accessibilityRole="radiogroup" accessibilityLabel="Fork access">
          <Chip label="Same" selected={temper === null} onPress={() => setTemper(null)} />
          {FORK_TEMPERS.map((level) => (
            <Chip key={level} label={level} selected={temper === level} onPress={() => setTemper(level)} />
          ))}
        </View>

        <Text style={[typeScale.meta, styles.fieldLabel, { color: tokens.ink3 }]}>First prompt</Text>
        <Input
          value={prompt}
          onChangeText={setPrompt}
          placeholder="Optional — what the fork should do first"
          multiline
          numberOfLines={3}
          accessibilityLabel="Fork first prompt"
        />

        {baseSession?.worktree ? (
          <View style={styles.worktreeRow}>
            <GitBranch size={15} strokeWidth={1.75} color={tokens.ink3} />
            <Text style={[typeScale.sub, styles.baseText, { color: tokens.ink2 }]}>Isolated git worktree</Text>
            <Text style={[styles.mono, { color: tokens.ink4 }]} numberOfLines={1}>{worktreeLabel(baseSession.worktree)}</Text>
          </View>
        ) : null}

        {fork.isError ? (
          <Text style={[typeScale.sub, styles.helper, { color: tokens.danger }]} numberOfLines={2}>
            {fork.error instanceof ApiError && fork.error.message ? `Could not fork: ${fork.error.message}` : "Could not fork this session. Try again."}
          </Text>
        ) : null}

        <View style={styles.actions}>
          <Button label="Cancel" variant="ghost" onPress={cancel} disabled={pending !== null} />
          <Button
            label="Move"
            variant="secondary"
            onPress={() => startFork("move")}
            disabled={!base || pending === "fork"}
            loading={pending === "move"}
          />
          <Button
            label="Fork"
            onPress={() => startFork("fork")}
            disabled={!base || pending === "move"}
            loading={pending === "fork"}
            icon={<GitBranch size={16} strokeWidth={2} color={tokens.bg2} />}
          />
        </View>
        <Text style={[typeScale.sub, { color: tokens.ink3 }]}>Move forks, then archives this session.</Text>
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space20, paddingBottom: space.space24, gap: space.space8 },
  subtitle: { marginTop: 2, marginBottom: space.space4 },
  fieldLabel: { marginTop: space.space8 },
  baseRow: { flexDirection: "row", alignItems: "center", gap: space.space8, minHeight: 48, paddingHorizontal: space.space12, borderWidth: 1, borderRadius: radii.radius12 },
  baseText: { flex: 1, minWidth: 0 },
  chevronOpen: { transform: [{ rotate: "180deg" }] },
  picker: { maxHeight: 168, borderWidth: 1, borderColor: "transparent", marginTop: space.space4 },
  pickerRow: { minHeight: 44, flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space12, paddingVertical: space.space8 },
  chips: { flexDirection: "row", flexWrap: "wrap", gap: space.space8 },
  actions: { flexDirection: "row", justifyContent: "flex-end", gap: space.space8, marginTop: space.space8 },
  worktreeRow: { flexDirection: "row", alignItems: "center", gap: space.space8, minHeight: 44 },
  helper: { marginVertical: space.space4 },
  mono: { fontFamily: monoFamily.regular, fontSize: 11, lineHeight: 15 },
});
