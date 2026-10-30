import * as Clipboard from "expo-clipboard";
import React, { useState } from "react";
import { StyleSheet, View } from "react-native";

import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { useToast } from "../ds/ToastHost";
import { useInvokeSkill } from "../../lib/skillsQueries";
import { space } from "../../theme/tokens";

export function SkillInvokeForm({ name, session }: { name: string; session?: string | null }) {
  const toast = useToast();
  const invoke = useInvokeSkill();
  const [task, setTask] = useState("");

  const run = () =>
    invoke.mutate(
      { name, task, session },
      {
        onSuccess: (result) => {
          void Clipboard.setStringAsync(result.prompt).then(() => toast.show(`Copied ${result.prompt} (used ${result.uses}x)`, { tone: "success" }));
        },
        onError: (error) => toast.show(error.message || "Could not use the skill", { tone: "danger" }),
      },
    );

  return (
    <View style={styles.form}>
      <Input label="Task (optional)" value={task} onChangeText={setTask} placeholder={`What should ${name} do?`} onSubmitEditing={run} returnKeyType="go" />
      <Button label="Copy /skill line" variant="secondary" loading={invoke.isPending} onPress={run} accessibilityLabel={`Copy the /skill ${name} line to paste into a session`} />
    </View>
  );
}

const styles = StyleSheet.create({ form: { gap: space.space8, marginTop: space.space8 } });
