import React, { useEffect, useState } from "react";
import { KeyboardAvoidingView, Platform, ScrollView, Text } from "react-native";

import type { McpServerRow } from "../../lib/api";
import { joinArgs, splitArgs, useEditMcpServer } from "../../lib/mcpManage";
import { useCreateMcpServer } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Segmented } from "../ds/Segmented";
import { Sheet } from "../ds/Sheet";

type Transport = "stdio" | "http" | "sse";

/** Add a server, or edit one when `editing` is set (the name is fixed; everything else is replaced). */
export function AddMcpServerSheet({ visible, onClose, editing }: { visible: boolean; onClose: () => void; editing?: McpServerRow | null }) {
  const tokens = useTokens();
  const create = useCreateMcpServer();
  const edit = useEditMcpServer();
  const mutation = editing ? edit : create;
  const [name, setName] = useState("");
  const [transport, setTransport] = useState<Transport>("stdio");
  const [command, setCommand] = useState("");
  const [args, setArgs] = useState("");
  const [url, setUrl] = useState("");
  const [tokenEnv, setTokenEnv] = useState("");
  useEffect(() => {
    if (!visible) return;
    setName(editing?.name ?? "");
    setTransport(editing?.transport ?? "stdio");
    setCommand(editing?.command ?? "");
    setArgs(joinArgs(editing?.args));
    setUrl(editing?.url ?? "");
    setTokenEnv(editing?.token_env ?? "");
  }, [visible, editing]);
  const submit = () =>
    mutation.mutate(
      {
        name,
        transport,
        command: transport === "stdio" ? command : undefined,
        args: transport === "stdio" ? splitArgs(args) : undefined,
        url: transport === "stdio" ? undefined : url,
        token_env: tokenEnv || undefined,
      },
      { onSuccess: onClose },
    );
  const title = editing ? `Edit ${editing.name}` : "Add MCP server";
  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel={title} snapPoints={[0.85]}>
      <KeyboardAvoidingView behavior={Platform.OS === "ios" ? "padding" : undefined} style={{ flex: 1 }}>
        <ScrollView contentContainerStyle={{ paddingHorizontal: space.space16, paddingBottom: space.space32, gap: space.space12 }} keyboardShouldPersistTaps="handled">
          <Text style={[type.heading, { color: tokens.ink }]}>{title}</Text>
          <Text style={[type.sub, { color: tokens.ink3 }]}>Token values stay in your environment or keyring. Only an optional environment-variable name is saved.</Text>
          <Input label="Name" value={name} onChangeText={setName} autoCapitalize="none" autoCorrect={false} disabled={!!editing} accessibilityLabel="MCP server name" />
          <Segmented options={[{ value: "stdio", label: "Stdio" }, { value: "http", label: "HTTP" }, { value: "sse", label: "SSE" }]} value={transport} onChange={(value) => setTransport(value as Transport)} />
          {transport === "stdio" ? (
            <>
              <Input label="Command" value={command} onChangeText={setCommand} autoCapitalize="none" autoCorrect={false} accessibilityLabel="MCP command" />
              <Input label="Arguments" value={args} onChangeText={setArgs} autoCapitalize="none" autoCorrect={false} placeholder="--serve --quiet" accessibilityLabel="MCP command arguments" />
            </>
          ) : (
            <Input label="URL" value={url} onChangeText={setUrl} autoCapitalize="none" autoCorrect={false} placeholder="https://example.com/mcp" accessibilityLabel="MCP URL" />
          )}
          <Input label="Token environment variable (optional)" value={tokenEnv} onChangeText={setTokenEnv} autoCapitalize="characters" autoCorrect={false} placeholder="GITLAB_TOKEN" accessibilityLabel="MCP token environment variable" />
          {mutation.isError ? <Text style={[type.sub, { color: tokens.danger }]}>{mutation.error instanceof Error ? mutation.error.message : "Could not save server"}</Text> : null}
          <Button label={editing ? "Save changes" : "Add server"} onPress={submit} disabled={!name.trim() || (transport === "stdio" ? !command.trim() : !url.trim()) || mutation.isPending} fullWidth />
        </ScrollView>
      </KeyboardAvoidingView>
    </Sheet>
  );
}
