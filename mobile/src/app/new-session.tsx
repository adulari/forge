// New session (modal, "Rise" — the slide-up transition is the Stack.Screen `presentation:
// "modal"` option owned by the root layout, T2.1). FEATURES.md §1.1: POST /api/sessions,
// inline `{error}` verbatim for bad cwd / not-a-git-repo.
import { router } from "expo-router";
import React, { useCallback, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { Button } from "../components/ds/Button";
import { Card } from "../components/ds/Card";
import { Checkbox } from "../components/ds/Checkbox";
import { Input } from "../components/ds/Input";
import { Screen } from "../components/ds/Screen";
import { ApiError } from "../lib/api";
import { useCreateSession } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type as typeScale } from "../theme/typography";

export default function NewSessionScreen() {
  const tokens = useTokens();
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

  const serverError =
    create.error instanceof ApiError
      ? create.error.message
      : create.isError
        ? "create failed"
        : null;

  return (
    <Screen scroll keyboardAvoiding contentContainerStyle={styles.content}>
      <Card style={styles.card}>
        <Input
          label="Working directory"
          mono
          value={cwd}
          onChangeText={setCwd}
          placeholder="/path/to/project"
          autoCapitalize="none"
          autoCorrect={false}
          returnKeyType="next"
        />

        <Input
          label="Title (optional)"
          value={title}
          onChangeText={setTitle}
          placeholder="session title"
          returnKeyType="next"
        />

        <Input
          label="Model (optional)"
          value={model}
          onChangeText={setModel}
          placeholder="e.g. claude-sonnet-5"
          autoCapitalize="none"
          autoCorrect={false}
          returnKeyType="done"
          onSubmitEditing={handleSubmit}
        />

        <View style={styles.worktreeRow}>
          <Checkbox value={worktree} onValueChange={setWorktree} accessibilityLabel="Isolated git worktree" />
          <Text style={[typeScale.body, { color: tokens.ink }]}>Isolated git worktree</Text>
        </View>

        {validationError ? (
          <Text style={[typeScale.sub, { color: tokens.danger }]}>{validationError}</Text>
        ) : null}
        {serverError ? <Text style={[typeScale.sub, { color: tokens.danger }]}>{serverError}</Text> : null}

        <Button label="Create session" onPress={handleSubmit} loading={create.isPending} fullWidth />
      </Card>
    </Screen>
  );
}

const styles = StyleSheet.create({
  content: { paddingVertical: space.space16 },
  card: { gap: space.space16 },
  worktreeRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
});
