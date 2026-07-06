// New session (modal) — BUILD_PLAN §6 "New session". Header title comes from the root
// Stack.Screen options (title: "New session"), so this body renders only the form.
import { router, useLocalSearchParams } from "expo-router";
import React, { useCallback, useState } from "react";
import { Platform, Text, View } from "react-native";

import { ApiError } from "../lib/api";
import { useCreateSession } from "../lib/queries";
import { Card, Chip, ErrorText, PrimaryButton, Screen, SearchInput } from "../components/ui";

function hapticLight() {
  if (Platform.OS === "web") return;
  import("expo-haptics")
    .then((Haptics) => Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light))
    .catch(() => undefined);
}

function FieldLabel({ children }: { children: string }) {
  return <Text className="text-dim text-[12px] font-semibold">{children}</Text>;
}

export default function NewSessionScreen() {
  const { resume } = useLocalSearchParams<{ resume?: string }>();
  const [cwd, setCwd] = useState("");
  const [title, setTitle] = useState("");
  const [model, setModel] = useState("");
  const [worktree, setWorktree] = useState(false);
  const [validationError, setValidationError] = useState<string | null>(null);
  const create = useCreateSession();

  const handleSubmit = useCallback(() => {
    const trimmedCwd = cwd.trim();
    if (!trimmedCwd && !resume) {
      setValidationError("cwd is required");
      return;
    }
    setValidationError(null);
    hapticLight();
    create.mutate(
      {
        cwd: trimmedCwd || undefined,
        worktree,
        title: title.trim() || undefined,
        model: model.trim() || undefined,
        resume: resume || undefined,
      },
      {
        onSuccess: (res) => {
          router.replace(`/session/${res.id}`);
        },
      },
    );
  }, [cwd, title, model, worktree, resume, create]);

  const serverError =
    create.error instanceof ApiError
      ? create.error.message
      : create.isError
        ? "create failed"
        : null;

  return (
    <Screen keyboardAvoiding>
      <Card className="gap-12">
        <View className="gap-4">
          <FieldLabel>Working directory</FieldLabel>
          <SearchInput
            value={cwd}
            onChangeText={setCwd}
            placeholder="daemon cwd"
            autoCapitalize="none"
            autoCorrect={false}
            returnKeyType="next"
          />
        </View>

        <View className="gap-4">
          <FieldLabel>Title (optional)</FieldLabel>
          <SearchInput value={title} onChangeText={setTitle} placeholder="session title" returnKeyType="next" />
        </View>

        <View className="gap-4">
          <FieldLabel>Model (optional)</FieldLabel>
          <SearchInput
            value={model}
            onChangeText={setModel}
            placeholder="e.g. claude-sonnet-5"
            autoCapitalize="none"
            autoCorrect={false}
            returnKeyType="done"
            onSubmitEditing={handleSubmit}
          />
        </View>

        <Chip label="Isolated git worktree" selected={worktree} onPress={() => setWorktree((v) => !v)} />

        {validationError ? <ErrorText message={validationError} /> : null}
        {serverError ? <ErrorText message={serverError} /> : null}

        <PrimaryButton
          label={resume ? "Resume session" : "Create session"}
          onPress={handleSubmit}
          loading={create.isPending}
        />
      </Card>
    </Screen>
  );
}
