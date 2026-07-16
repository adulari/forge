// New session (modal, "Rise" — the slide-up transition is the Stack.Screen `presentation:
// "modal"` option owned by the root layout, T2.1). FEATURES.md §1.1: POST /api/sessions,
// inline `{error}` verbatim for bad cwd / not-a-git-repo.
//
// Owns its own themed header (mirrors SessionHeader.tsx's back-control + title pattern)
// instead of expo-router's default unthemed header — see _layout.tsx's Stack.Screen for
// this route (headerShown: false).
import { router, useLocalSearchParams } from "expo-router";
import { ChevronDown, ChevronUp, X } from "lucide-react-native";
import React, { useCallback, useEffect, useRef, useState } from "react";
import { Platform, StyleSheet, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { Button } from "../components/ds/Button";
import { Card } from "../components/ds/Card";
import { Checkbox } from "../components/ds/Checkbox";
import { IconButton } from "../components/ds/IconButton";
import { Input } from "../components/ds/Input";
import { Screen } from "../components/ds/Screen";
import { Segmented } from "../components/ds/Segmented";
import { ModelPicker } from "../components/session/ModelPicker";
import { ProjectPicker } from "../components/session/ProjectPicker";
import { ApiError } from "../lib/api";
import { useAuth } from "../lib/auth";
import { goBackOr } from "../lib/nav";
import { lastProjectStorageKey } from "../lib/projectSelection";
import { useCreateSession, useProjects } from "../lib/queries";
import { getSecureItem, setSecureItem } from "../lib/secureStore";
import { useTokens } from "../theme/ThemeProvider";
import { gutter, space } from "../theme/tokens";
import { type as typeScale } from "../theme/typography";
import { useBreakpoint } from "../theme/useBreakpoint";

const FORM_MAX_WIDTH = 560;

export default function NewSessionScreen() {
  const tokens = useTokens();
  const { isCompact } = useBreakpoint();
  const params = useLocalSearchParams<{ cwd?: string | string[] }>();
  const requestedCwd = Array.isArray(params.cwd) ? params.cwd[0] : params.cwd;
  const { activeServerId } = useAuth();
  const projects = useProjects();
  const initializedProjectKey = useRef<string | null>(null);
  const projectSelectionRevision = useRef(0);
  const [cwd, setCwd] = useState(requestedCwd ?? "");
  const [title, setTitle] = useState("");
  const [model, setModel] = useState("");
  const [worktree, setWorktree] = useState(false);
  const [temper, setTemper] = useState<"Read-only" | "Ask" | "Auto-edit" | "Full">("Ask");
  const [advancedVisible, setAdvancedVisible] = useState(false);
  const create = useCreateSession();

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

  const rememberProject = useCallback((path: string) => {
    projectSelectionRevision.current += 1;
    setCwd(path);
    if (create.isError) create.reset();
    if (activeServerId && path) void setSecureItem(lastProjectStorageKey(activeServerId), path);
  }, [activeServerId, create]);

  const handleSubmit = useCallback(() => {
    if (create.isPending) return;
    const trimmedCwd = cwd.trim();
    create.mutate(
      {
        cwd: trimmedCwd || undefined,
        title: title.trim() || undefined,
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
  }, [cwd, title, model, worktree, temper, create, activeServerId]);

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

  const serverError =
    create.error instanceof ApiError
      ? create.error.message
      : create.isError
        ? "create failed"
        : null;

  const headerGutter = { paddingHorizontal: isCompact ? gutter.compact : gutter.medium };

  return (
    <View style={[styles.flex, { backgroundColor: tokens.bg1 }]}>
      <SafeAreaView edges={["top", "left", "right"]} style={{ backgroundColor: tokens.bg1 }}>
        <View style={[headerGutter, styles.headerRow]}>
          <IconButton
            icon={<X size={20} strokeWidth={1.75} color={tokens.ink} />}
            onPress={onClose}
            accessibilityLabel="Close"
          />
          <Text style={[typeScale.heading, styles.headerTitle, { color: tokens.ink }]} numberOfLines={1}>
            New session
          </Text>
        </View>
      </SafeAreaView>

      <Screen
        edges={["left", "right", "bottom"]}
        scroll
        keyboardAvoiding
        contentContainerStyle={styles.content}
      >
        <Card style={[styles.card, isCompact ? undefined : styles.cardWide]}>
          <View style={styles.intro}>
            <Text style={[typeScale.heading, styles.formTitle, { color: tokens.ink }]}>Start a new session</Text>
            <Text style={[typeScale.sub, { color: tokens.ink3 }]}>Your current project is ready. Change it only when you need another workspace.</Text>
          </View>

          <ProjectPicker value={cwd} onChange={rememberProject} />

          <Input
            label="Title (optional)"
            value={title}
            onChangeText={setTitle}
            placeholder="session title"
            returnKeyType="next"
          />

          <Button
            label={advancedVisible ? "Hide advanced options" : "Show advanced options"}
            variant="ghost"
            icon={advancedVisible ? <ChevronUp size={18} color={tokens.ink2} /> : <ChevronDown size={18} color={tokens.ink2} />}
            onPress={() => setAdvancedVisible((visible) => !visible)}
            accessibilityLabel={advancedVisible ? "Hide advanced session options" : "Show advanced session options"}
          />

          {advancedVisible ? (
            <View style={styles.advanced}>
              <ModelPicker value={model} onChange={setModel} />

              <View style={styles.worktreeBlock}>
                <Text style={[typeScale.body, { color: tokens.ink }]}>Run mode</Text>
                <Segmented options={[{ value: "Read-only", label: "Read" }, { value: "Ask", label: "Ask" }, { value: "Auto-edit", label: "Edit" }, { value: "Full", label: "Full" }]} value={temper} onChange={setTemper} />
                <Text style={[typeScale.sub, { color: tokens.ink3 }]}>{temper === "Read-only" ? "inspect without making changes" : temper === "Ask" ? "ask before edits and commands" : temper === "Auto-edit" ? "apply edits automatically; ask for risky commands" : "full autonomy, use only in trusted projects"}</Text>
              </View>

              <View style={styles.worktreeBlock}>
                <View style={styles.worktreeRow}>
                  <Checkbox value={worktree} onValueChange={setWorktree} accessibilityLabel="Isolated git worktree" />
                  <Text style={[typeScale.body, { color: tokens.ink }]}>Isolated git worktree</Text>
                </View>
                <Text style={[typeScale.sub, styles.worktreeHint, { color: tokens.ink3 }]}>run in an isolated branch/worktree</Text>
              </View>
            </View>
          ) : null}

          {serverError ? <Text accessibilityRole="alert" style={[typeScale.sub, { color: tokens.danger }]}>{serverError}</Text> : null}

          <Button label="Create session" onPress={handleSubmit} loading={create.isPending} fullWidth accessibilityLabel="Create session" accessibilityHint="Creates a session in the selected working directory" />
        </Card>
      </Screen>
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
  content: { paddingVertical: space.space16, alignItems: "center" },
  card: { gap: space.space16, width: "100%" },
  cardWide: { maxWidth: FORM_MAX_WIDTH },
  intro: { gap: space.space4 },
  formTitle: { marginBottom: space.space4 },
  advanced: { gap: space.space16 },
  worktreeBlock: { gap: space.space4 },
  worktreeRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  worktreeHint: { paddingLeft: 22 + space.space8 },
});
