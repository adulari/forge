// Checks: the project's test, lint and build actions (`[[actions]]` in .forge/config.toml), run in
// their own daemon terminals. Each shows pass or fail; a failing one names the changed files it
// mentions (they gain a "failing" mark in the queue), links file:line into the editor, and can hand
// its failure to the agent.
import { CheckCircle2, CircleDashed, Loader2, Play, Square, Wand2, XCircle } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { openTerminalSocket, type TerminalSocket } from "../../lib/api";
import { runProjectAction, stopProjectAction, type ProjectAction } from "../../lib/apiProjectActions";
import { useAuth } from "../../lib/auth";
import { useDirectBase, useProjectActions } from "../../lib/queriesProjectActions";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, sansFamilyFor } from "../../theme/typography";
import { BoundedList } from "../ds/BoundedList";
import { EmptyState } from "../ds/EmptyState";
import { useToast } from "../ds/ToastHost";
import { appendOutput, checkActions, checkState, failingChangedFiles, fixPrompt, parseFileRefs, resolveRefPath, splitLineRefs, stripAnsi, type CheckState, type FileRef } from "./checksModel";
import { useIde } from "./IdeContext";
import { revealLine, setActionFailing } from "./ideSignals";
import { IdeIconButton, IdeTextButton } from "./ideUi";
import { useAgentSend } from "./ReviewDraft";
import { rowBackground } from "./rowStyle";

// Output per action lives outside React: a socket feeds it, the open pane subscribes.
const outputs = new Map<string, string>();
const listeners = new Set<() => void>();
let version = 0;
const outputKey = (sessionId: string, actionId: string) => `${sessionId}/${actionId}`;
function setOutput(key: string, text: string) {
  outputs.set(key, text);
  version += 1;
  listeners.forEach((listener) => listener());
}
function useOutput(key: string | null): string {
  useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => version,
    () => version,
  );
  return key ? (outputs.get(key) ?? "") : "";
}

/** Attaches to the action's terminal and keeps its output, for as long as the panel is open. */
function useActionCapture(sessionId: string, action: ProjectAction, baseUrl: string | null) {
  useEffect(() => {
    if (!baseUrl || !action.terminal_id) return;
    const key = outputKey(sessionId, action.id);
    let text = "";
    let socket: TerminalSocket | null = null;
    try {
      socket = openTerminalSocket(
        baseUrl,
        sessionId,
        {
          onOutput: (chunk) => {
            text = appendOutput(text, chunk);
            setOutput(key, text);
          },
          onClear: () => {
            text = "";
            setOutput(key, text);
          },
        },
        { terminalId: action.terminal_id, size: { cols: 160, rows: 40 } },
      );
    } catch {
      socket = null;
    }
    return () => socket?.close();
    // A new run of the action gets a new terminal id; re-attach then.
  }, [baseUrl, sessionId, action.id, action.terminal_id]);
}

const STATE_ICON: Record<CheckState, React.ComponentType<{ size?: number; color?: string; strokeWidth?: number }>> = {
  idle: CircleDashed,
  running: Loader2,
  pass: CheckCircle2,
  fail: XCircle,
};

function ActionRow({
  action,
  selected,
  sessionId,
  baseUrl,
  onSelect,
  changedPaths,
}: {
  action: ProjectAction;
  selected: boolean;
  sessionId: string;
  baseUrl: string | null;
  onSelect: () => void;
  changedPaths: readonly string[];
}) {
  const tokens = useTokens();
  const toast = useToast();
  const state = checkState(action);
  useActionCapture(sessionId, action, baseUrl);
  const output = useOutput(outputKey(sessionId, action.id));
  const Icon = STATE_ICON[state];
  const color = state === "fail" ? tokens.danger : state === "pass" ? tokens.success : tokens.ink3;

  // A failed run names files; mark the changed ones. Debounced: output arrives in bursts.
  const changedKey = changedPaths.join("\0");
  useEffect(() => {
    if (state !== "fail") {
      setActionFailing(sessionId, action.id, []);
      return;
    }
    const timer = setTimeout(() => setActionFailing(sessionId, action.id, failingChangedFiles(changedKey ? changedKey.split("\0") : [], parseFileRefs(output))), 400);
    return () => clearTimeout(timer);
  }, [state, output, changedKey, sessionId, action.id]);
  useEffect(() => () => setActionFailing(sessionId, action.id, []), [sessionId, action.id]);

  const run = async () => {
    if (!baseUrl) return;
    try {
      await (state === "running" ? stopProjectAction(baseUrl, sessionId, action.id) : runProjectAction(baseUrl, sessionId, action.id));
    } catch (error) {
      toast.show(error instanceof Error ? error.message : "Could not start the check.", { tone: "danger" });
    }
  };
  return (
    <Pressable
      onPress={onSelect}
      accessibilityRole="button"
      accessibilityState={{ selected }}
      accessibilityLabel={`${action.name}: ${state}`}
      style={({ hovered }: { hovered?: boolean; pressed: boolean }) => [styles.action, { backgroundColor: rowBackground(tokens, { hovered, selected }) }]}
    >
      <Icon size={13} strokeWidth={1.75} color={color} />
      <View style={styles.actionText}>
        <Text numberOfLines={1} style={[styles.actionName, { color: tokens.ink }]}>
          {action.name}
        </Text>
        <Text numberOfLines={1} style={[styles.actionMeta, { color: state === "fail" ? tokens.danger : tokens.ink4 }]}>
          {state === "fail" ? `failed${action.exit_code != null ? ` (exit ${action.exit_code})` : ""}` : state === "pass" ? "passed" : state === "running" ? "running" : action.command}
        </Text>
      </View>
      <IdeIconButton
        icon={state === "running" ? <Square size={11} strokeWidth={2} color={tokens.ink2} /> : <Play size={11} strokeWidth={2} color={tokens.ink2} />}
        label={state === "running" ? `Stop ${action.name}` : `Run ${action.name}`}
        onPress={() => void run()}
      />
    </Pressable>
  );
}

const OutputLine = React.memo(function OutputLine({ line, onOpen }: { line: string; onOpen: (ref: FileRef) => void }) {
  const tokens = useTokens();
  const segments = splitLineRefs(line);
  return (
    <Text style={[styles.out, { color: tokens.ink2 }]} numberOfLines={1}>
      {segments.map((segment, index) =>
        segment.ref ? (
          <Text key={index} onPress={() => onOpen(segment.ref as FileRef)} accessibilityRole="link" style={{ color: tokens.ink, textDecorationLine: "underline" }}>
            {segment.text}
          </Text>
        ) : (
          segment.text
        ),
      )}
    </Text>
  );
});

export default function ChecksPanel({ sessionId }: { sessionId: string }) {
  const tokens = useTokens();
  const toast = useToast();
  const send = useAgentSend();
  const { baseUrl } = useAuth();
  const direct = useDirectBase();
  const { changes, openFile, openChange } = useIde();
  const actionsQuery = useProjectActions(sessionId);
  const checks = useMemo(() => checkActions(actionsQuery.data?.actions ?? []), [actionsQuery.data]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const selected = checks.find((action) => action.id === selectedId) ?? checks.find((action) => checkState(action) === "fail") ?? checks[0] ?? null;
  const changedPaths = useMemo(() => changes.get(sessionId)?.files.map((file) => file.path) ?? [], [changes, sessionId]);
  const output = useOutput(selected ? outputKey(sessionId, selected.id) : null);
  const lines = useMemo(() => stripAnsi(output).split("\n").slice(-1500), [output]);
  const listRef = useRef<{ scrollToEnd?: (o?: { animated?: boolean }) => void } | null>(null);
  useEffect(() => {
    listRef.current?.scrollToEnd?.({ animated: false });
  }, [lines.length, selected?.id]);

  const open = useCallback(
    (ref: FileRef) => {
      const path = resolveRefPath(ref, changedPaths);
      if (changedPaths.includes(path)) openChange(sessionId, path);
      else openFile(sessionId, path, true);
      revealLine(sessionId, path, ref.line);
    },
    [changedPaths, openChange, openFile, sessionId],
  );

  if (!direct || !baseUrl) return <EmptyState icon={CircleDashed} message="Checks need a direct connection to the daemon on this machine." />;
  if (actionsQuery.data?.untrusted) return <EmptyState icon={CircleDashed} message="This project defines actions but is not trusted yet. Trust it from the Ports dock to run checks." />;
  if (!actionsQuery.isPending && checks.length === 0) {
    return <EmptyState icon={CircleDashed} message={'No checks yet. Add [[actions]] named "test", "lint" or "build" to .forge/config.toml and they run here.'} />;
  }
  const failed = selected && checkState(selected) === "fail";
  const runAll = async () => {
    for (const action of checks) {
      if (checkState(action) === "running") continue;
      try {
        await runProjectAction(baseUrl, sessionId, action.id);
      } catch (error) {
        toast.show(error instanceof Error ? error.message : `Could not start ${action.name}.`, { tone: "danger" });
        return;
      }
    }
  };
  return (
    <View style={styles.root}>
      <View style={[styles.list, { borderRightColor: tokens.hairline }]}>
        {checks.map((action) => (
          <ActionRow
            key={action.id}
            action={action}
            selected={selected?.id === action.id}
            sessionId={sessionId}
            baseUrl={direct}
            onSelect={() => setSelectedId(action.id)}
            changedPaths={changedPaths}
          />
        ))}
        <View style={styles.listFoot}>
          <IdeTextButton label="Run all" icon={<Play size={11} strokeWidth={2} color={tokens.ink2} />} onPress={() => void runAll()} />
        </View>
      </View>
      <View style={styles.pane}>
        {selected ? (
          <>
            <View style={[styles.paneHead, { borderBottomColor: tokens.hairline }]}>
              <Text numberOfLines={1} style={[styles.paneTitle, { color: tokens.ink2 }]}>
                {selected.command}
              </Text>
              {failed ? (
                <IdeTextButton
                  label="Ask agent to fix"
                  tone="accent"
                  icon={<Wand2 size={12} strokeWidth={1.75} color={tokens.accent} />}
                  onPress={() => {
                    const where = send(sessionId, fixPrompt(selected.name, selected.command, selected.exit_code, output));
                    toast.show(where === "sent" ? "Sent the failure to the agent" : "The failure will send when the agent opens", { tone: "neutral" });
                  }}
                />
              ) : null}
            </View>
            {output ? (
              <BoundedList<string>
                ref={listRef as never}
                data={lines}
                keyExtractor={(_, index) => String(index)}
                renderItem={({ item }) => <OutputLine line={item} onOpen={open} />}
                getItemLayout={(_, index) => ({ length: LINE, offset: LINE * index, index })}
                ListEmptyComponent={<View />}
              />
            ) : (
              <View style={styles.hint}>
                <Text style={[styles.actionMeta, { color: tokens.ink4 }]}>Not run yet. Press play to run it in this project.</Text>
              </View>
            )}
          </>
        ) : null}
      </View>
    </View>
  );
}

const LINE = 16;
const styles = StyleSheet.create({
  root: { flex: 1, flexDirection: "row", minHeight: 0 },
  list: { width: 240, borderRightWidth: StyleSheet.hairlineWidth },
  action: { minHeight: 36, flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space12 },
  actionText: { flex: 1, minWidth: 0 },
  actionName: { fontFamily: sansFamilyFor(500), fontSize: 12 },
  actionMeta: { fontFamily: monoFamily.regular, fontSize: 10 },
  listFoot: { padding: space.space8, flexDirection: "row" },
  pane: { flex: 1, minWidth: 0, minHeight: 0 },
  paneHead: { minHeight: 30, flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: space.space8, paddingHorizontal: space.space12, borderBottomWidth: StyleSheet.hairlineWidth },
  paneTitle: { flex: 1, minWidth: 0, fontFamily: monoFamily.regular, fontSize: 10.5 },
  out: { height: LINE, paddingHorizontal: space.space12, fontFamily: monoFamily.regular, fontSize: 11, lineHeight: LINE },
  hint: { padding: space.space16 },
});
