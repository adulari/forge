// New session (modal, "Rise" — the slide-up transition is the Stack.Screen `presentation:
// "modal"` option owned by the root layout, T2.1). FEATURES.md §1.1: POST /api/sessions,
// inline `{error}` verbatim for bad cwd / not-a-git-repo.
//
// Owns its own themed header (mirrors SessionHeader.tsx's back-control + title pattern)
// instead of expo-router's default unthemed header — see _layout.tsx's Stack.Screen for
// this route (headerShown: false).
import { router } from "expo-router";
import { X } from "lucide-react-native";
import React, { useCallback, useEffect, useState } from "react";
import { Platform, StyleSheet, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { Button } from "../components/ds/Button";
import { Card } from "../components/ds/Card";
import { Checkbox } from "../components/ds/Checkbox";
import { IconButton } from "../components/ds/IconButton";
import { Input } from "../components/ds/Input";
import { Screen } from "../components/ds/Screen";
import { ApiError } from "../lib/api";
import { goBackOr } from "../lib/nav";
import { useCreateSession } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { gutter, space } from "../theme/tokens";
import { type as typeScale } from "../theme/typography";
import { useBreakpoint } from "../theme/useBreakpoint";

const FORM_MAX_WIDTH = 560;

export default function NewSessionScreen() {
  const tokens = useTokens();
  const { isCompact } = useBreakpoint();
  const [cwd, setCwd] = useState("");
  const [title, setTitle] = useState("");
  const [model, setModel] = useState("");
  const [worktree, setWorktree] = useState(false);
  const [validationError, setValidationError] = useState<string | null>(null);
  const create = useCreateSession();

  const handleSubmit = useCallback(() => {
    const trimmedCwd = cwd.trim();
    if (!trimmedCwd) {
      setValidationError("working directory is required");
      return;
    }
    setValidationError(null);
    create.mutate(
      {
        cwd: trimmedCwd,
        title: title.trim() || undefined,
        model: model.trim() || undefined,
        worktree,
      },
      {
        onSuccess: (res) => {
          router.replace(`/session/${res.id}`);
        },
      },
    );
  }, [cwd, title, model, worktree, create]);

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
          <View style={styles.field}>
            <Input
              label="Working directory (required)"
              mono
              value={cwd}
              onChangeText={(t) => {
                setCwd(t);
                if (validationError) setValidationError(null);
                if (create.isError) create.reset();
              }}
              placeholder="/path/to/project"
              autoCapitalize="none"
              autoCorrect={false}
              returnKeyType="next"
            />
            <Text style={[typeScale.sub, { color: tokens.ink3 }]}>absolute path to a git repo</Text>
          </View>

          <Input
            label="Title (optional)"
            value={title}
            onChangeText={setTitle}
            placeholder="session title"
            returnKeyType="next"
          />

          <Input
            label="Model (optional)"
            mono
            value={model}
            onChangeText={setModel}
            placeholder="e.g. claude-sonnet-5"
            autoCapitalize="none"
            autoCorrect={false}
            returnKeyType="done"
            onSubmitEditing={handleSubmit}
          />

          <View style={styles.worktreeBlock}>
            <View style={styles.worktreeRow}>
              <Checkbox value={worktree} onValueChange={setWorktree} accessibilityLabel="Isolated git worktree" />
              <Text style={[typeScale.body, { color: tokens.ink }]}>Isolated git worktree</Text>
            </View>
            <Text style={[typeScale.sub, styles.worktreeHint, { color: tokens.ink3 }]}>
              run in an isolated branch/worktree
            </Text>
          </View>

          {validationError ? (
            <Text style={[typeScale.sub, { color: tokens.danger }]}>{validationError}</Text>
          ) : null}
          {serverError ? <Text style={[typeScale.sub, { color: tokens.danger }]}>{serverError}</Text> : null}

          <Button label="Create session" onPress={handleSubmit} loading={create.isPending} fullWidth />
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
  field: { gap: space.space4 },
  worktreeBlock: { gap: space.space4 },
  worktreeRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  worktreeHint: { paddingLeft: 22 + space.space8 },
});
