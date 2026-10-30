import React, { useState } from "react";
import { ActivityIndicator, Linking, Text, View } from "react-native";

import type { RemoteInput } from "../../lib/ws";
import { usePrAvailable, useCreatePr, usePrStatus } from "../../lib/queriesPr";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";
import { Switch } from "../ds/Switch";
import { useToast } from "../ds/ToastHost";
import { PrDetailView } from "../pr/PrDetailView";

interface Props {
  visible: boolean;
  onClose: () => void;
  sessionId: string;
  /** Hands `/pr` to the agent; omitted where there is no live session to prompt. */
  send?: (input: RemoteInput) => boolean;
}

function CreateForm({ sessionId, onAgent }: { sessionId: string; onAgent: ((title: string) => void) | null }) {
  const tokens = useTokens();
  const toast = useToast();
  const create = useCreatePr();
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [base, setBase] = useState("");
  const [draft, setDraft] = useState(false);
  const submit = () => {
    if (!title.trim()) return;
    create.mutate(
      { session: sessionId, title: title.trim(), body, base: base.trim() || undefined, draft },
      {
        onSuccess: (result) => {
          toast.show(result.url ? `Opened ${result.url}` : "Pull request opened", { tone: "success" });
        },
      },
    );
  };
  return (
    <View style={{ paddingHorizontal: space.space16, paddingBottom: space.space16, gap: space.space12 }}>
      <Text style={[type.heading, { color: tokens.ink }]}>Create pull request</Text>
      <Text style={[type.sub, { color: tokens.ink3 }]}>
        Pushes the current branch (never forced) and opens a pull request with the session&apos;s provenance. Commit your work first.
      </Text>
      <Input label="Title" value={title} onChangeText={setTitle} placeholder="feat: improve onboarding" autoCapitalize="sentences" accessibilityLabel="Pull request title" returnKeyType="next" />
      <Input label="Description" value={body} onChangeText={setBody} multiline placeholder="What changed and why" accessibilityLabel="Pull request description" />
      <Input label="Base branch (optional)" value={base} onChangeText={setBase} placeholder="repository default" autoCapitalize="none" mono accessibilityLabel="Base branch" />
      <View style={{ flexDirection: "row", alignItems: "center", justifyContent: "space-between" }}>
        <Text style={[type.sub, { color: tokens.ink2 }]}>Open as draft</Text>
        <Switch value={draft} onValueChange={setDraft} accessibilityLabel="Open as draft" />
      </View>
      {create.error ? <Text style={[type.sub, { color: tokens.danger }]}>{create.error.message}</Text> : null}
      <Button label="Push & create pull request" onPress={submit} disabled={!title.trim()} loading={create.isPending} fullWidth />
      {onAgent ? <Button label="Let the agent prepare it" variant="ghost" onPress={() => onAgent(title)} fullWidth /> : null}
    </View>
  );
}

export function PullRequestSheet({ visible, onClose, sessionId, send }: Props) {
  const tokens = useTokens();
  const direct = usePrAvailable();
  const status = usePrStatus(visible && direct ? sessionId : null);
  const agent = send
    ? (title: string) => {
        if (send({ kind: "prompt", text: title.trim() ? `/pr ${title.trim()}` : "/pr" })) onClose();
      }
    : null;

  let content: React.ReactNode;
  if (!direct) {
    content = (
      <View style={{ paddingHorizontal: space.space16, gap: space.space12 }}>
        <Text style={[type.heading, { color: tokens.ink }]}>Create pull request</Text>
        <Text style={[type.sub, { color: tokens.ink3 }]}>
          Pull requests need a direct connection to the host. Ask the agent to prepare one instead.
        </Text>
        {agent ? <Button label="Prepare pull request" onPress={() => agent("")} fullWidth /> : null}
      </View>
    );
  } else if (status.isPending) {
    content = <ActivityIndicator color={tokens.accent} style={{ padding: space.space32 }} />;
  } else if (status.data && !status.data.available) {
    content = (
      <View style={{ paddingHorizontal: space.space16, gap: space.space12 }}>
        <Text style={[type.heading, { color: tokens.ink }]}>GitHub CLI unavailable</Text>
        <Text style={[type.sub, { color: tokens.ink3 }]}>{status.data.error}</Text>
        <Button label="Get the GitHub CLI" variant="secondary" onPress={() => void Linking.openURL("https://cli.github.com").catch(() => undefined)} />
      </View>
    );
  } else if (status.data?.pr) {
    content = <PrDetailView sessionId={sessionId} number={status.data.pr.number} />;
  } else {
    content = <CreateForm sessionId={sessionId} onAgent={agent} />;
  }
  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Pull request" snapPoints={[0.9]}>
      {content}
    </Sheet>
  );
}
