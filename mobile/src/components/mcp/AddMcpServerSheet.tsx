import React, { useState } from "react";
import { KeyboardAvoidingView, Platform, ScrollView, Text } from "react-native";

import { useCreateMcpServer } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Segmented } from "../ds/Segmented";
import { Sheet } from "../ds/Sheet";

export function AddMcpServerSheet({ visible, onClose }: { visible: boolean; onClose: () => void }) {
  const tokens = useTokens();
  const create = useCreateMcpServer();
  const [name, setName] = useState("");
  const [transport, setTransport] = useState<"stdio" | "http" | "sse">("stdio");
  const [command, setCommand] = useState("");
  const [args, setArgs] = useState("");
  const [url, setUrl] = useState("");
  const [tokenEnv, setTokenEnv] = useState("");
  const submit = () => create.mutate({ name, transport, command: transport === "stdio" ? command : undefined, args: transport === "stdio" ? args.split(/\s+/).filter(Boolean) : undefined, url: transport === "stdio" ? undefined : url, token_env: tokenEnv || undefined }, { onSuccess: () => { setName(""); setCommand(""); setArgs(""); setUrl(""); setTokenEnv(""); onClose(); } });
  return <Sheet visible={visible} onClose={onClose} accessibilityLabel="Add MCP server" snapPoints={[0.85]}><KeyboardAvoidingView behavior={Platform.OS === "ios" ? "padding" : undefined} style={{ flex: 1 }}><ScrollView contentContainerStyle={{ paddingHorizontal: space.space16, paddingBottom: space.space32, gap: space.space12 }} keyboardShouldPersistTaps="handled"><Text style={[type.heading, { color: tokens.ink }]}>Add MCP server</Text><Text style={[type.sub, { color: tokens.ink3 }]}>Token values stay in your environment or keyring. Only an optional environment-variable name is saved.</Text><Input label="Name" value={name} onChangeText={setName} autoCapitalize="none" autoCorrect={false} accessibilityLabel="MCP server name" /><Segmented options={[{ value: "stdio", label: "Stdio" }, { value: "http", label: "HTTP" }, { value: "sse", label: "SSE" }]} value={transport} onChange={(value) => setTransport(value as typeof transport)} />{transport === "stdio" ? <><Input label="Command" value={command} onChangeText={setCommand} autoCapitalize="none" autoCorrect={false} accessibilityLabel="MCP command" /><Input label="Arguments" value={args} onChangeText={setArgs} autoCapitalize="none" autoCorrect={false} placeholder="--serve --quiet" accessibilityLabel="MCP command arguments" /></> : <Input label="URL" value={url} onChangeText={setUrl} autoCapitalize="none" autoCorrect={false} placeholder="https://example.com/mcp" accessibilityLabel="MCP URL" />}<Input label="Token environment variable (optional)" value={tokenEnv} onChangeText={setTokenEnv} autoCapitalize="characters" autoCorrect={false} placeholder="GITLAB_TOKEN" accessibilityLabel="MCP token environment variable" />{create.isError ? <Text style={[type.sub, { color: tokens.danger }]}>{create.error instanceof Error ? create.error.message : "Could not save server"}</Text> : null}<Button label="Add server" onPress={submit} disabled={!name.trim() || (transport === "stdio" ? !command.trim() : !url.trim()) || create.isPending} fullWidth /></ScrollView></KeyboardAvoidingView></Sheet>;
}
