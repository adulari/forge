import React, { useState } from "react";
import { Text, View } from "react-native";

import type { RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";

export function PullRequestSheet({ visible, onClose, send }: { visible: boolean; onClose: () => void; send: (input: RemoteInput) => boolean }) {
  const tokens = useTokens();
  const [title, setTitle] = useState("");
  const submit = () => { if (send({ kind: "prompt", text: title.trim() ? `/pr ${title.trim()}` : "/pr" })) { setTitle(""); onClose(); } };
  return <Sheet visible={visible} onClose={onClose} accessibilityLabel="Create pull request" snapPoints={[0.6]}><View style={{ paddingHorizontal: space.space16, paddingBottom: space.space16, gap: space.space12 }}><Text style={[type.heading, { color: tokens.ink }]}>Create pull request</Text><Text style={[type.sub, { color: tokens.ink3 }]}>Forge will inspect the work, create a branch if needed, commit only relevant files, push, and open a PR with session provenance.</Text><Input label="PR title (optional)" value={title} onChangeText={setTitle} placeholder="feat: improve onboarding" autoCapitalize="sentences" accessibilityLabel="Pull request title" returnKeyType="send" onSubmitEditing={submit} /><Button label="Prepare pull request" onPress={submit} fullWidth /></View></Sheet>;
}
