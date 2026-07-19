// "Forge a task" (mobile.dc.html:378, desktop.dc.html:252) — Hearth core rule 6: the task
// composer replaces every "new session" affordance. CreateSessionRequest has no separate
// initial-prompt field yet, so per HANDOFF core rule 8 ("session titles are task titles")
// the composer's free text maps straight onto the existing `title` param — no data-layer
// change needed, this is a reskin.
//
// Compact (mobile): full-bleed screen, header + scrollable body + a CTA pinned near the
// bottom. Medium/expanded (desktop/web): expo-router's `presentation: "modal"` gives no
// backdrop dimming on web, so this screen renders its own centered 600pt card + scrim,
// matching the desktop "Forge a Task" modal — same pattern OverlayPanel/CommandPalette use
// for their wide variant.
import { router, useLocalSearchParams } from "expo-router";
import { Check, ChevronDown, X } from "lucide-react-native";
import React, { useCallback, useEffect, useRef, useState } from "react";
import { Platform, StyleSheet, Text, TextInput, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { HostDot } from "../components/anywhere/HostDot";
import { Button } from "../components/ds/Button";
import { Checkbox } from "../components/ds/Checkbox";
import { IconButton } from "../components/ds/IconButton";
import { ListRow } from "../components/ds/ListRow";
import { Screen } from "../components/ds/Screen";
import { SectionHeader } from "../components/ds/SectionHeader";
import { Segmented } from "../components/ds/Segmented";
import { Sheet } from "../components/ds/Sheet";
import { useToast } from "../components/ds/ToastHost";
import { ModelPicker } from "../components/session/ModelPicker";
import { ProjectPicker } from "../components/session/ProjectPicker";
import { ApiError } from "../lib/api";
import { hostStateText } from "../lib/anywhere/format";
import { anywhereClient, useAnywhere, useAnywhereHosts } from "../lib/anywhere/store";
import type { AnywhereHost } from "../lib/anywhere/types";
import { useAuth } from "../lib/auth";
import { goBackOr } from "../lib/nav";
import { lastProjectStorageKey } from "../lib/projectSelection";
import { useCreateSession, useProjects } from "../lib/queries";
import { getSecureItem, setSecureItem } from "../lib/secureStore";
import { useTheme, useTokens } from "../theme/ThemeProvider";
import { depthDark, depthLight, gutter, radii, shadowStyle, space } from "../theme/tokens";
import { type as typeScale, webInputTextStyle } from "../theme/typography";
import { useBreakpoint } from "../theme/useBreakpoint";

// Forge Anywhere: which host a new session targets. `default` = the active server over
// direct transport (today's only real path); `anywhere` = a signed-in Anywhere host.
type HostChoice = { kind: "default" } | { kind: "anywhere"; host: AnywhereHost };

const MODAL_WIDTH = 600;

type Temper = "Read-only" | "Ask" | "Auto-edit" | "Full";

const TEMPER_OPTIONS: { value: Temper; label: string }[] = [
  { value: "Read-only", label: "Read" },
  { value: "Ask", label: "Ask" },
  { value: "Auto-edit", label: "Edit" },
  { value: "Full", label: "Full" },
];

const TEMPER_HINT: Record<Temper, string> = {
  "Read-only": "inspect without making changes",
  Ask: "ask before edits and commands",
  "Auto-edit": "apply edits automatically; ask for risky commands",
  Full: "full autonomy, use only in trusted projects",
};

// Forge Anywhere: "Host" row (design mobile.dc.html "AW Forge a Task", lines 921-926) —
// mirrors ProjectPicker/ModelPicker's trigger-ListRow + Sheet idiom. Defined locally
// rather than as its own src/components/session/ file since this task's edit scope is
// new-session.tsx only.
function HostPicker({
  value,
  onChange,
  activeServerName,
  hosts,
}: {
  value: HostChoice;
  onChange: (choice: HostChoice) => void;
  activeServerName: string;
  hosts: AnywhereHost[];
}) {
  const tokens = useTokens();
  const [visible, setVisible] = useState(false);

  const selectedLabel = value.kind === "default" ? activeServerName : value.host.name;
  const selectedMeta = value.kind === "default" ? "direct · default" : `${hostStateText(value.host)} · anywhere`;

  const select = (choice: HostChoice) => {
    onChange(choice);
    setVisible(false);
  };

  return (
    <View>
      <ListRow
        title={selectedLabel}
        subtitle={selectedMeta}
        leading={
          value.kind === "anywhere" ? (
            <HostDot state={value.host.state} />
          ) : (
            <View style={[styles.defaultHostDot, { backgroundColor: tokens.success }]} />
          )
        }
        trailing={<ChevronDown size={18} strokeWidth={1.75} color={tokens.ink3} />}
        onPress={() => setVisible(true)}
        accessibilityLabel={`Host: ${selectedLabel}. Change host`}
      />
      <Sheet visible={visible} onClose={() => setVisible(false)} accessibilityLabel="Choose host">
        <View style={styles.hostSheetContent}>
          <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Choose host</Text>
          <View>
            <SectionHeader>This device</SectionHeader>
            <ListRow
              title={`${activeServerName} (default)`}
              subtitle="direct"
              leading={<View style={[styles.defaultHostDot, { backgroundColor: tokens.success }]} />}
              trailing={value.kind === "default" ? <Check size={16} strokeWidth={2} color={tokens.success} /> : undefined}
              onPress={() => select({ kind: "default" })}
              showSeparator={false}
            />
          </View>
          {hosts.length > 0 ? (
            <View>
              <SectionHeader>Forge Anywhere</SectionHeader>
              {hosts.map((host, index) => (
                <ListRow
                  key={host.id}
                  title={host.name}
                  subtitle={`${hostStateText(host)} · anywhere`}
                  leading={<HostDot state={host.state} />}
                  trailing={
                    value.kind === "anywhere" && value.host.id === host.id ? (
                      <Check size={16} strokeWidth={2} color={tokens.success} />
                    ) : undefined
                  }
                  onPress={() => select({ kind: "anywhere", host })}
                  showSeparator={index < hosts.length - 1}
                />
              ))}
            </View>
          ) : null}
        </View>
      </Sheet>
    </View>
  );
}

export default function NewSessionScreen() {
  const tokens = useTokens();
  const { scheme } = useTheme();
  const { isCompact } = useBreakpoint();
  const params = useLocalSearchParams<{ cwd?: string | string[]; title?: string | string[] }>();
  const requestedCwd = Array.isArray(params.cwd) ? params.cwd[0] : params.cwd;
  // Fleet/rail TaskComposer hands its typed text over as ?title= — the composer IS the
  // new-session affordance (HANDOFF rule 6), so that text must survive the navigation.
  const requestedTitle = Array.isArray(params.title) ? params.title[0] : params.title;
  const { activeServerId, servers } = useAuth();
  const projects = useProjects();
  const initializedProjectKey = useRef<string | null>(null);
  const projectSelectionRevision = useRef(0);
  const [cwd, setCwd] = useState(requestedCwd ?? "");
  const [task, setTask] = useState(requestedTitle ?? "");
  const [model, setModel] = useState("");
  const [worktree, setWorktree] = useState(true);
  const [temper, setTemper] = useState<Temper>("Ask");
  const create = useCreateSession();
  const toast = useToast();

  // Forge Anywhere: "Host" row only appears once signed in with at least one host —
  // zero visual/behavioral change otherwise.
  const { signedIn: anywhereSignedIn } = useAnywhere();
  const { hosts: anywhereHosts } = useAnywhereHosts();
  const activeServerName = servers.find((s) => s.id === activeServerId)?.name ?? "this server";
  const showHostPicker = anywhereSignedIn && anywhereHosts.length > 0;
  const [hostChoice, setHostChoice] = useState<HostChoice>({ kind: "default" });
  const isOfflineHostQueue =
    showHostPicker && hostChoice.kind === "anywhere" && hostChoice.host.state.kind === "offline";
  const [queuing, setQueuing] = useState(false);
  const [queueError, setQueueError] = useState<string | null>(null);

  useEffect(() => {
    if (!activeServerId || !projects.data) return;
    const initializationKey = `${activeServerId}:${requestedCwd ?? ""}`;
    if (initializedProjectKey.current === initializationKey) return;
    initializedProjectKey.current = initializationKey;
    if (requestedCwd) {
      setCwd(requestedCwd);
      return;
    }
    const revision = projectSelectionRevision.current;
    setCwd(projects.data.default_cwd);
    let cancelled = false;
    void getSecureItem(lastProjectStorageKey(activeServerId)).then((remembered) => {
      if (!cancelled && remembered && projectSelectionRevision.current === revision) {
        setCwd(remembered);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [activeServerId, projects.data, requestedCwd]);

  const rememberProject = useCallback(
    (path: string) => {
      projectSelectionRevision.current += 1;
      setCwd(path);
      if (create.isError) create.reset();
      if (activeServerId && path) void setSecureItem(lastProjectStorageKey(activeServerId), path);
    },
    [activeServerId, create],
  );

  const handleSubmit = useCallback(() => {
    // Forge Anywhere: an offline host queues a remote job instead of creating a session
    // directly — queued jobs are immutable (cancel/replace only, never edit), so this is
    // a distinct action from the normal create flow, not a variant of it.
    if (isOfflineHostQueue) {
      if (queuing || hostChoice.kind !== "anywhere") return;
      setQueuing(true);
      setQueueError(null);
      const host = hostChoice.host;
      anywhereClient
        .queueJob({ hostId: host.id, prompt: task.trim() })
        .then(() => {
          toast.show(`queued for ${host.name}`);
          router.push("/anywhere/jobs");
        })
        .catch((err) => {
          setQueueError(err instanceof Error ? err.message : "could not queue job");
        })
        .finally(() => setQueuing(false));
      return;
    }

    if (create.isPending) return;
    const trimmedCwd = cwd.trim();
    create.mutate(
      {
        cwd: trimmedCwd || undefined,
        title: task.trim() || undefined,
        model: model.trim() || undefined,
        worktree,
        temper,
      },
      {
        onSuccess: (res) => {
          if (activeServerId) void setSecureItem(lastProjectStorageKey(activeServerId), res.cwd);
          router.replace(`/session/${res.id}`);
        },
      },
    );
  }, [isOfflineHostQueue, queuing, hostChoice, task, toast, create, cwd, model, worktree, temper, activeServerId]);

  const onClose = useCallback(() => goBackOr("/(tabs)"), []);

  // Web/desktop: Escape closes the modal, same as the X button. Bypasses the typing-target
  // guard other hotkeys use — a modal-dismiss key should work even while a field is focused.
  useEffect(() => {
    if (Platform.OS !== "web") return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  const serverError = isOfflineHostQueue
    ? queueError
    : create.error instanceof ApiError
      ? create.error.message
      : create.isError
        ? "create failed"
        : null;

  const submitPending = isOfflineHostQueue ? queuing : create.isPending;
  const canSubmit = !submitPending;
  const submitLabel = isOfflineHostQueue ? "Queue remote job" : "Forge session";

  const whereAndHow = (
    <View>
      <Text style={[typeScale.section, styles.sectionLabel, { color: tokens.ink4 }]}>Where &amp; how</Text>
      <View>
        <ProjectPicker value={cwd} onChange={rememberProject} />
        {showHostPicker ? (
          <HostPicker value={hostChoice} onChange={setHostChoice} activeServerName={activeServerName} hosts={anywhereHosts} />
        ) : null}
        <ModelPicker value={model} onChange={setModel} />
      </View>
      <View style={styles.temperBlock}>
        <Segmented options={TEMPER_OPTIONS} value={temper} onChange={setTemper} />
        <Text style={[typeScale.sub, { color: tokens.ink3 }]}>{TEMPER_HINT[temper]}</Text>
      </View>
      <View style={styles.worktreeRow}>
        <Checkbox value={worktree} onValueChange={setWorktree} accessibilityLabel="Isolated git worktree" />
        <Text style={[typeScale.body, styles.worktreeLabel, { color: tokens.ink }]}>Isolated git worktree</Text>
        <Text style={[typeScale.meta, { color: tokens.ink4 }]}>recommended</Text>
      </View>
      {showHostPicker && hostChoice.kind === "anywhere" ? (
        isOfflineHostQueue ? (
          // Design mobile.dc.html "AW Forge a Task", line 950 — queue-privacy footnote,
          // shown only for the offline-host queue path.
          <Text style={[typeScale.meta, styles.hostNote, { color: tokens.ink4 }]}>
            Prompt, path and title are encrypted in the queue. The relay sees size, kind and timestamps only.
          </Text>
        ) : (
          // Real session creation isn't routed to a chosen Anywhere host yet
          // (CreateSessionRequest has no host/transport param) — an online host still
          // creates on the active server, this note says so honestly instead of implying
          // the session actually runs on the selected host.
          <Text style={[typeScale.meta, styles.hostNote, { color: tokens.ink3 }]}>
            runs via {activeServerName} until relay sessions land
          </Text>
        )
      ) : null}
      {serverError ? (
        <Text accessibilityRole="alert" style={[typeScale.sub, styles.errorText, { color: tokens.danger }]}>
          {serverError}
        </Text>
      ) : null}
    </View>
  );

  if (isCompact) {
    return (
      <View style={[styles.flex, { backgroundColor: tokens.bg1 }]}>
        <SafeAreaView edges={["top", "left", "right"]} style={{ backgroundColor: tokens.bg1 }}>
          <View style={[{ paddingHorizontal: gutter.compact }, styles.headerRow]}>
            <IconButton
              icon={<X size={19} strokeWidth={1.75} color={tokens.ink2} />}
              onPress={onClose}
              accessibilityLabel="Close"
            />
            <Text style={[typeScale.headingBold, styles.headerTitle, { color: tokens.ink }]} numberOfLines={1}>
              Forge a task
            </Text>
          </View>
        </SafeAreaView>

        <Screen edges={["left", "right", "bottom"]} scroll keyboardAvoiding contentContainerStyle={styles.content}>
          <TaskDescriptionBox value={task} onChangeText={setTask} disabled={create.isPending} />
          {whereAndHow}
          <View style={styles.spacer} />
          <Button
            label={submitLabel}
            onPress={handleSubmit}
            loading={submitPending}
            disabled={!canSubmit}
            fullWidth
            accessibilityLabel={submitLabel}
            accessibilityHint={isOfflineHostQueue ? "Queues a remote job for when the host returns" : "Creates a session for this task"}
          />
        </Screen>
      </View>
    );
  }

  const depth = scheme === "dark" ? depthDark : depthLight;

  return (
    <View style={[styles.modalRoot, { backgroundColor: tokens.overlayScrim }]}>
      <View
        style={[
          styles.modalCard,
          { backgroundColor: tokens.bg2, borderColor: tokens.borderStrong, borderRadius: radii.radius16 },
          shadowStyle(depth.sheet),
        ]}
        accessibilityViewIsModal
        accessibilityLabel="Forge a task"
      >
        <View style={styles.modalHeaderRow}>
          <Text style={[typeScale.headingBold, styles.headerTitle, { color: tokens.ink }]}>Forge a task</Text>
          <IconButton
            icon={<X size={17} strokeWidth={1.75} color={tokens.ink2} />}
            onPress={onClose}
            accessibilityLabel="Close"
          />
        </View>

        <TaskDescriptionBox value={task} onChangeText={setTask} disabled={create.isPending} wide />
        {whereAndHow}

        <View style={styles.modalFooterRow}>
          <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>↵ forge · esc cancel</Text>
          <View style={styles.flexSpacer} />
          <Button
            label={submitLabel}
            onPress={handleSubmit}
            loading={submitPending}
            disabled={!canSubmit}
            accessibilityLabel={submitLabel}
            accessibilityHint={isOfflineHostQueue ? "Queues a remote job for when the host returns" : "Creates a session for this task"}
          />
        </View>
      </View>
    </View>
  );
}

// The one free-text box that stands in for every "new session" affordance (Hearth core rule
// 6). Not the pill `TaskComposer` (Fleet's send-a-prompt-to-a-live-session control) — this is
// the taller "describe what to forge" variant, so it gets its own small local component
// instead of overloading that one's props.
function TaskDescriptionBox({
  value,
  onChangeText,
  disabled,
  wide,
}: {
  value: string;
  onChangeText: (text: string) => void;
  disabled?: boolean;
  wide?: boolean;
}) {
  const tokens = useTokens();
  const { isCompact } = useBreakpoint();
  // One step darker than the surrounding surface, per HANDOFF: bg2 on the mobile screen
  // (which sits on bg1), bg1 inside the desktop/web bg2 modal card.
  const boxBg = isCompact ? tokens.bg2 : tokens.bg1;

  return (
    <View
      style={[
        styles.taskBox,
        wide && styles.taskBoxWide,
        { backgroundColor: boxBg, borderColor: tokens.borderStrong },
      ]}
    >
      <TextInput
        value={value}
        onChangeText={onChangeText}
        placeholder="Describe a task to forge…"
        placeholderTextColor={tokens.ink3}
        editable={!disabled}
        multiline
        autoFocus={!isCompact}
        cursorColor={tokens.accent}
        selectionColor={tokens.accent}
        style={[styles.taskInput, webInputTextStyle, { color: tokens.ink }]}
        accessibilityLabel="Describe a task to forge"
      />
      <Text style={[typeScale.meta, styles.taskHint, { color: tokens.ink4 }]}>
        Forge plans the session from your sentence — project, model and mode below only if you
        want control.
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1 },
  headerRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingVertical: space.space8,
  },
  headerTitle: { flex: 1 },
  content: { paddingTop: space.space12, paddingBottom: space.space16, gap: space.space4, flexGrow: 1 },
  spacer: { flex: 1, minHeight: space.space24 },
  taskBox: { borderWidth: 1, borderRadius: radii.radius16, padding: space.space16, marginBottom: space.space8 },
  taskBoxWide: { marginTop: space.space16 },
  taskInput: { fontSize: 16, lineHeight: 24, minHeight: 48, textAlignVertical: "top", padding: 0 },
  taskHint: { marginTop: space.space12, lineHeight: 16 },
  sectionLabel: { paddingTop: space.space24, paddingBottom: space.space4 },
  temperBlock: { gap: space.space4, paddingVertical: space.space12 },
  worktreeRow: { flexDirection: "row", alignItems: "center", gap: space.space12, minHeight: 52 },
  worktreeLabel: { flex: 1 },
  errorText: { paddingTop: space.space8 },
  defaultHostDot: { width: 7, height: 7, borderRadius: 3.5 },
  hostSheetContent: { paddingHorizontal: space.space16, paddingBottom: space.space32, gap: space.space12 },
  hostNote: { paddingTop: space.space8 },
  modalRoot: { flex: 1, alignItems: "center", justifyContent: "flex-start", paddingTop: 110, padding: space.space24 },
  modalCard: { width: MODAL_WIDTH, maxWidth: "100%", borderWidth: 1, padding: space.space24 },
  modalHeaderRow: { flexDirection: "row", alignItems: "center" },
  modalFooterRow: { flexDirection: "row", alignItems: "center", gap: space.space12, marginTop: space.space16 },
  flexSpacer: { flex: 1 },
});
