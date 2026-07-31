import {
  CloudCog,
  KeyRound,
  ServerCog,
  ShieldCheck,
  Trash2,
} from "lucide-react-native";
import React, { useEffect, useMemo, useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  RefreshControl,
  StyleSheet,
  Text,
  View,
} from "react-native";

import { BackLink } from "../components/ds/BackLink";
import { Badge, type BadgeTone } from "../components/ds/Badge";
import { Button } from "../components/ds/Button";
import { ConfirmDialog } from "../components/ds/ConfirmDialog";
import { EmptyState } from "../components/ds/EmptyState";
import { IconButton } from "../components/ds/IconButton";
import { Input } from "../components/ds/Input";
import { Screen } from "../components/ds/Screen";
import { SectionHeader } from "../components/ds/SectionHeader";
import { Segmented } from "../components/ds/Segmented";
import { Sheet } from "../components/ds/Sheet";
import { Switch } from "../components/ds/Switch";
import { useToast } from "../components/ds/ToastHost";
import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import {
  type AzureProviderRequest,
  type CustomProviderRequest,
  type OAuthAccountRequest,
  type ProviderOAuthAccount,
  type ProviderRow,
  ApiError,
} from "../lib/api";
import { useAuth } from "../lib/auth";
import {
  useModels,
  useProviders,
  useRemoveAzureProvider,
  useRemoveCustomProvider,
  useRemoveOAuthAccount,
  useRemoveProviderKeys,
  useSaveAzureProvider,
  useSaveCustomProvider,
  useSetProviderEnabled,
  useStoreProviderKey,
  useSwitchOAuthAccount,
} from "../lib/queries";
import { supportsDirectDaemonEndpoints } from "../lib/transport";
import { useTokens } from "../theme/ThemeProvider";
import { radii, space } from "../theme/tokens";
import { monoFamily, type, tabularNums } from "../theme/typography";
import { SettingsShell } from "./(tabs)/settings";

type KeyMode = "append" | "replace";
type OAuthProviderId = OAuthAccountRequest["provider"];
type PendingRemoval =
  | { kind: "keys"; provider: ProviderRow }
  | { kind: "custom"; provider: ProviderRow }
  | { kind: "azure"; provider: ProviderRow }
  | { kind: "oauth"; provider: ProviderRow; account: ProviderOAuthAccount };

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof ApiError || error instanceof Error ? error.message : fallback;
}

function statusTone(provider: ProviderRow): BadgeTone {
  if (!provider.enabled) return "neutral";
  if (provider.auth_status === "ready" || provider.auth_status === "configured") return "success";
  if (provider.auth_status === "expired" || provider.auth_status === "missing") return "danger";
  if (provider.auth_status === "stopped" || provider.auth_status === "unverified") return "warn";
  return "neutral";
}

function statusLabel(provider: ProviderRow): string {
  if (!provider.enabled) return "disabled";
  switch (provider.auth_status) {
    case "configured":
      return "configured";
    case "ready":
      return "ready";
    case "expired":
      return "expired";
    case "stopped":
      return "stopped";
    case "unverified":
      return "verify login";
    case "missing":
    default:
      return "needs setup";
  }
}

function expiryLabel(account: ProviderOAuthAccount): string {
  if (account.expiry_status === "expired") return "expired";
  if (account.expires_at == null) return "expiry unknown";
  return `expires ${new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(new Date(account.expires_at * 1000))}`;
}

function ProviderItem({
  provider,
  modelHealth,
  mutationBusy,
  onEnabled,
  onKey,
  onRemoveKeys,
  onEditCustom,
  onEditAzure,
  onRemoveCustom,
  onRemoveAzure,
  onSwitchAccount,
  onRemoveAccount,
}: {
  provider: ProviderRow;
  modelHealth?: { total: number; ready: number };
  mutationBusy: boolean;
  onEnabled: (provider: ProviderRow, enabled: boolean) => void;
  onKey: (provider: ProviderRow) => void;
  onRemoveKeys: (provider: ProviderRow) => void;
  onEditCustom: (provider: ProviderRow) => void;
  onEditAzure: (provider: ProviderRow) => void;
  onRemoveCustom: (provider: ProviderRow) => void;
  onRemoveAzure: (provider: ProviderRow) => void;
  onSwitchAccount: (provider: ProviderRow, account: ProviderOAuthAccount) => void;
  onRemoveAccount: (provider: ProviderRow, account: ProviderOAuthAccount) => void;
}) {
  const tokens = useTokens();
  const source = provider.environment_key_present
    ? `environment · ${provider.env_var}`
    : provider.stored_key_fingerprints.length > 0
      ? `secure store · ${provider.stored_key_fingerprints.join(", ")}`
      : provider.keyless
        ? "no API key required"
        : provider.env_var
          ? `set ${provider.env_var} or store a key`
          : "no credential configured";
  const meta = [
    provider.id,
    provider.kind.replace("_", " "),
    provider.free ? "free" : null,
    modelHealth ? `${modelHealth.ready}/${modelHealth.total} models ready` : null,
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <View style={[styles.provider, { borderBottomColor: tokens.hairline }]}>
      <View style={styles.providerHeader}>
        <View
          style={[
            styles.statusDot,
            {
              backgroundColor: !provider.enabled
                ? tokens.ink4
                : provider.configured
                  ? tokens.success
                  : provider.auth_status === "unverified" || provider.auth_status === "stopped"
                    ? tokens.warn
                    : tokens.danger,
            },
          ]}
        />
        <View style={styles.providerTitle}>
          <Text style={[styles.providerName, { color: provider.enabled ? tokens.ink : tokens.ink2 }]} numberOfLines={1}>
            {provider.label}
          </Text>
          <Text style={[type.monoMeta, tabularNums, { color: tokens.ink4 }]} numberOfLines={1}>
            {meta}
          </Text>
        </View>
        {provider.restart_required ? <Badge label="restart" tone="warn" /> : null}
        <Badge label={statusLabel(provider)} tone={statusTone(provider)} />
        <Switch
          value={provider.enabled}
          onValueChange={(enabled) => onEnabled(provider, enabled)}
          disabled={mutationBusy}
          accessibilityLabel={`${provider.label} enabled`}
        />
      </View>

      {provider.kind === "api_key" || provider.kind === "custom" || provider.kind === "azure" ? (
        <Text style={[type.monoMeta, { color: tokens.ink4 }]}>{source}</Text>
      ) : null}
      {provider.endpoint ? (
        <Text style={[type.monoMeta, { color: tokens.ink4 }]} numberOfLines={2}>
          {provider.endpoint}
        </Text>
      ) : null}
      {provider.version ? (
        <Text style={[type.monoMeta, { color: tokens.ink4 }]}>
          version {provider.version}
          {provider.serving === false ? " · runtime not serving" : ""}
        </Text>
      ) : null}
      {provider.login_command ? (
        <Text style={[type.monoMeta, styles.command, { color: tokens.ink3 }]}>
          setup: {provider.login_command}
        </Text>
      ) : null}

      {provider.kind === "oauth" ? (
        <View style={styles.accounts}>
          {provider.accounts.length === 0 ? (
            <Text style={[type.sub, { color: tokens.ink3 }]}>
              No stored accounts. Run <Text style={styles.inlineMono}>{provider.login_command}</Text> on the host.
            </Text>
          ) : (
            provider.accounts.map((account) => (
              <View key={account.id} style={[styles.account, { backgroundColor: tokens.bg2 }]}>
                <View style={styles.accountBody}>
                  <View style={styles.accountTitle}>
                    <Text style={[type.bodyBold, { color: tokens.ink }]} numberOfLines={1}>
                      {account.id}
                    </Text>
                    {account.active ? <Badge label="active" tone="success" /> : null}
                  </View>
                  <Text
                    style={[
                      type.monoMeta,
                      { color: account.expiry_status === "expired" ? tokens.danger : tokens.ink4 },
                    ]}
                  >
                    {expiryLabel(account)}
                  </Text>
                </View>
                {!account.active ? (
                  <Button
                    label="Use"
                    variant="ghost"
                    disabled={mutationBusy}
                    onPress={() => onSwitchAccount(provider, account)}
                  />
                ) : null}
                <IconButton
                  icon={<Trash2 size={16} strokeWidth={1.75} color={tokens.ink4} />}
                  accessibilityLabel={`Remove account ${account.id}`}
                  disabled={mutationBusy}
                  onPress={() => onRemoveAccount(provider, account)}
                />
              </View>
            ))
          )}
        </View>
      ) : null}

      <View style={styles.actions}>
        {!provider.keyless && ["api_key", "custom", "azure"].includes(provider.kind) ? (
          <Button
            label={provider.stored_key_fingerprints.length > 0 ? "Add / replace key" : "Store key"}
            variant="secondary"
            icon={<KeyRound size={15} strokeWidth={1.75} color={tokens.ink2} />}
            disabled={mutationBusy}
            onPress={() => onKey(provider)}
          />
        ) : null}
        {provider.stored_key_fingerprints.length > 0 ? (
          <Button
            label="Remove stored keys"
            variant="ghost"
            disabled={mutationBusy}
            onPress={() => onRemoveKeys(provider)}
          />
        ) : null}
        {provider.kind === "custom" ? (
          <>
            <Button label="Edit endpoint" variant="ghost" disabled={mutationBusy} onPress={() => onEditCustom(provider)} />
            <Button label="Remove" variant="danger" disabled={mutationBusy} onPress={() => onRemoveCustom(provider)} />
          </>
        ) : null}
        {provider.kind === "azure" ? (
          <>
            <Button label="Edit Azure" variant="ghost" disabled={mutationBusy} onPress={() => onEditAzure(provider)} />
            <Button label="Remove" variant="danger" disabled={mutationBusy} onPress={() => onRemoveAzure(provider)} />
          </>
        ) : null}
      </View>
    </View>
  );
}

function KeySheet({
  provider,
  onClose,
}: {
  provider: ProviderRow | null;
  onClose: () => void;
}) {
  const tokens = useTokens();
  const toast = useToast();
  const mutation = useStoreProviderKey();
  const [key, setKey] = useState("");
  const [mode, setMode] = useState<KeyMode>("append");

  useEffect(() => {
    if (provider) {
      setKey("");
      setMode(provider.stored_key_fingerprints.length > 0 ? "append" : "replace");
    }
  }, [provider]);

  const close = () => {
    setKey("");
    onClose();
  };
  const submit = async () => {
    if (!provider || !key.trim()) return;
    try {
      const response = await mutation.mutateAsync({ provider: provider.id, key, mode });
      setKey("");
      toast.show(response.notice ?? `${provider.id} key stored.`, { tone: "success" });
      onClose();
    } catch (error) {
      toast.show(errorMessage(error, "Could not store provider key."), { tone: "danger" });
    }
  };

  return (
    <Sheet visible={provider != null} onClose={close} accessibilityLabel="Store provider API key" snapPoints={[0.58]}>
      <View style={styles.sheetContent}>
        <Text accessibilityRole="header" style={[type.heading, { color: tokens.ink }]}>
          {provider ? `Secure key · ${provider.id}` : "Secure key"}
        </Text>
        <Text style={[type.sub, { color: tokens.ink3 }]}>
          The key is written to the host keyring or encrypted fallback. It is never returned to this app.
        </Text>
        <Input
          label="API key"
          value={key}
          onChangeText={setKey}
          secureTextEntry
          autoCapitalize="none"
          autoCorrect={false}
          textContentType="password"
          mono
          clearable={false}
          maxLength={16 * 1024}
          accessibilityLabel={`${provider?.id ?? "provider"} API key`}
        />
        <Segmented
          options={[
            { value: "append", label: "Add for rotation" },
            { value: "replace", label: "Replace all stored" },
          ]}
          value={mode}
          onChange={setMode}
        />
        {provider?.environment_key_present ? (
          <Text style={[type.sub, { color: tokens.warnBgInk }]}>
            {provider.env_var} is also present in the daemon environment. Replacing stored keys does not alter it.
          </Text>
        ) : null}
        <View style={styles.sheetActions}>
          <Button label="Cancel" variant="ghost" onPress={close} disabled={mutation.isPending} />
          <Button
            label="Store securely"
            onPress={() => void submit()}
            loading={mutation.isPending}
            disabled={!key.trim()}
          />
        </View>
      </View>
    </Sheet>
  );
}

function CustomProviderSheet({
  visible,
  provider,
  onClose,
}: {
  visible: boolean;
  provider: ProviderRow | null;
  onClose: () => void;
}) {
  const tokens = useTokens();
  const toast = useToast();
  const mutation = useSaveCustomProvider();
  const [namespace, setNamespace] = useState("");
  const [label, setLabel] = useState("");
  const [endpoint, setEndpoint] = useState("");
  const [envVar, setEnvVar] = useState("");
  const [models, setModels] = useState("");
  const [free, setFree] = useState(false);

  useEffect(() => {
    if (!visible) return;
    setNamespace(provider?.id ?? "");
    setLabel(provider?.label ?? "");
    setEndpoint(provider?.endpoint ?? "");
    setEnvVar(provider?.env_var ?? "");
    setModels(provider?.models.join(", ") ?? "");
    setFree(provider?.free ?? false);
  }, [provider, visible]);

  const submit = async () => {
    const body: CustomProviderRequest = {
      namespace: namespace.trim(),
      base_url: endpoint.trim(),
      api_key_env: envVar.trim() || undefined,
      free,
      models: models
        .split(",")
        .map((model) => model.trim())
        .filter(Boolean),
      label: label.trim() || undefined,
    };
    try {
      const response = await mutation.mutateAsync(body);
      toast.show(response.notice ?? "Custom provider saved.", { tone: "success" });
      onClose();
    } catch (error) {
      toast.show(errorMessage(error, "Could not save custom provider."), { tone: "danger" });
    }
  };

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Custom provider" snapPoints={[0.86]}>
      <View style={styles.sheetContent}>
        <Text accessibilityRole="header" style={[type.heading, { color: tokens.ink }]}>
          {provider ? "Edit custom endpoint" : "Add custom endpoint"}
        </Text>
        <Text style={[type.sub, { color: tokens.ink3 }]}>
          OpenAI-compatible endpoints become available after the daemon restarts.
        </Text>
        <Input label="Namespace" value={namespace} onChangeText={setNamespace} disabled={provider != null} autoCapitalize="none" mono maxLength={64} />
        <Input label="Display name (optional)" value={label} onChangeText={setLabel} maxLength={160} />
        <Input label="Base URL" value={endpoint} onChangeText={setEndpoint} autoCapitalize="none" autoCorrect={false} keyboardType="url" mono maxLength={2048} />
        <Input label="Key environment variable (optional)" value={envVar} onChangeText={(value) => setEnvVar(value.toUpperCase())} autoCapitalize="characters" autoCorrect={false} mono maxLength={128} />
        <Input label="Models (comma-separated, optional)" value={models} onChangeText={setModels} autoCapitalize="none" autoCorrect={false} mono />
        <View style={styles.toggleRow}>
          <View style={styles.flexFill}>
            <Text style={[type.bodyBold, { color: tokens.ink }]}>Free / local endpoint</Text>
            <Text style={[type.sub, { color: tokens.ink3 }]}>Allows Forge to treat these models as non-metered.</Text>
          </View>
          <Switch value={free} onValueChange={setFree} accessibilityLabel="Free or local endpoint" />
        </View>
        <View style={styles.sheetActions}>
          <Button label="Cancel" variant="ghost" onPress={onClose} disabled={mutation.isPending} />
          <Button
            label="Save endpoint"
            onPress={() => void submit()}
            loading={mutation.isPending}
            disabled={!namespace.trim() || !endpoint.trim()}
          />
        </View>
      </View>
    </Sheet>
  );
}

function AzureProviderSheet({
  visible,
  provider,
  onClose,
}: {
  visible: boolean;
  provider: ProviderRow | null;
  onClose: () => void;
}) {
  const tokens = useTokens();
  const toast = useToast();
  const mutation = useSaveAzureProvider();
  const [resource, setResource] = useState("");
  const [endpoint, setEndpoint] = useState("");
  const [apiVersion, setApiVersion] = useState("");
  const [envVar, setEnvVar] = useState("AZURE_OPENAI_API_KEY");
  const [deployments, setDeployments] = useState("");
  const [label, setLabel] = useState("");

  useEffect(() => {
    if (!visible) return;
    setResource(provider?.azure_resource ?? "");
    setEndpoint(provider?.azure_resource ? "" : provider?.endpoint ?? "");
    setApiVersion(provider?.azure_api_version ?? "");
    setEnvVar(provider?.env_var ?? "AZURE_OPENAI_API_KEY");
    setDeployments(provider?.models.join(", ") ?? "");
    setLabel(provider?.label ?? "");
  }, [provider, visible]);

  const submit = async () => {
    const body: AzureProviderRequest = {
      resource: resource.trim() || undefined,
      endpoint: endpoint.trim() || undefined,
      api_version: apiVersion.trim() || undefined,
      api_key_env: envVar.trim() || undefined,
      deployments: deployments
        .split(",")
        .map((deployment) => deployment.trim())
        .filter(Boolean),
      label: label.trim() || undefined,
    };
    try {
      const response = await mutation.mutateAsync(body);
      toast.show(response.notice ?? "Azure provider saved.", { tone: "success" });
      onClose();
    } catch (error) {
      toast.show(errorMessage(error, "Could not save Azure provider."), { tone: "danger" });
    }
  };

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Azure OpenAI provider" snapPoints={[0.86]}>
      <View style={styles.sheetContent}>
        <Text accessibilityRole="header" style={[type.heading, { color: tokens.ink }]}>
          {provider ? "Edit Azure OpenAI" : "Add Azure OpenAI"}
        </Text>
        <Text style={[type.sub, { color: tokens.ink3 }]}>
          Use a resource name for Azure public cloud, or an explicit endpoint for sovereign clouds and proxies.
        </Text>
        <Input label="Resource name" value={resource} onChangeText={setResource} autoCapitalize="none" autoCorrect={false} mono maxLength={256} />
        <Input label="Explicit endpoint (overrides resource)" value={endpoint} onChangeText={setEndpoint} autoCapitalize="none" autoCorrect={false} keyboardType="url" mono maxLength={2048} />
        <Input label="API version (optional)" value={apiVersion} onChangeText={setApiVersion} autoCapitalize="none" autoCorrect={false} mono maxLength={128} />
        <Input label="Key environment variable" value={envVar} onChangeText={(value) => setEnvVar(value.toUpperCase())} autoCapitalize="characters" autoCorrect={false} mono maxLength={128} />
        <Input label="Deployments (comma-separated)" value={deployments} onChangeText={setDeployments} autoCapitalize="none" autoCorrect={false} mono />
        <Input label="Display name (optional)" value={label} onChangeText={setLabel} maxLength={160} />
        <View style={styles.sheetActions}>
          <Button label="Cancel" variant="ghost" onPress={onClose} disabled={mutation.isPending} />
          <Button
            label="Save Azure"
            onPress={() => void submit()}
            loading={mutation.isPending}
            disabled={!resource.trim() && !endpoint.trim()}
          />
        </View>
      </View>
    </Sheet>
  );
}

function ProvidersScreenBody() {
  const tokens = useTokens();
  const toast = useToast();
  const { baseUrl } = useAuth();
  const direct = baseUrl != null && supportsDirectDaemonEndpoints(baseUrl);
  const query = useProviders();
  const modelsQuery = useModels();
  const setEnabled = useSetProviderEnabled();
  const removeKeys = useRemoveProviderKeys();
  const switchAccount = useSwitchOAuthAccount();
  const removeAccount = useRemoveOAuthAccount();
  const removeCustom = useRemoveCustomProvider();
  const removeAzure = useRemoveAzureProvider();
  const [keyProvider, setKeyProvider] = useState<ProviderRow | null>(null);
  const [customSheet, setCustomSheet] = useState<{ visible: boolean; provider: ProviderRow | null }>({
    visible: false,
    provider: null,
  });
  const [azureSheet, setAzureSheet] = useState<{ visible: boolean; provider: ProviderRow | null }>({
    visible: false,
    provider: null,
  });
  const [pendingRemoval, setPendingRemoval] = useState<PendingRemoval | null>(null);

  const modelHealth = useMemo(
    () =>
      new Map(
        (modelsQuery.data?.providers ?? []).map(({ provider, models }) => [
          provider,
          { total: models.length, ready: models.filter((model) => model.health == null).length },
        ]),
      ),
    [modelsQuery.data?.providers],
  );
  const providers = useMemo(() => query.data?.providers ?? [], [query.data?.providers]);
  const sections = useMemo(
    () => [
      {
        key: "subscription",
        title: "Subscription accounts & CLI bridges",
        rows: providers.filter((provider) => provider.kind === "oauth" || provider.kind === "cli"),
      },
      {
        key: "api",
        title: "API providers",
        rows: providers.filter((provider) => provider.kind === "api_key"),
      },
      {
        key: "custom",
        title: "Custom, enterprise & local",
        rows: providers.filter((provider) => ["custom", "azure", "local"].includes(provider.kind)),
      },
    ],
    [providers],
  );
  const mutationBusy =
    setEnabled.isPending ||
    removeKeys.isPending ||
    switchAccount.isPending ||
    removeAccount.isPending ||
    removeCustom.isPending ||
    removeAzure.isPending;

  const enabled = async (provider: ProviderRow, value: boolean) => {
    try {
      const response = await setEnabled.mutateAsync({ provider: provider.id, enabled: value });
      if (response.notice) toast.show(response.notice, { tone: "neutral" });
    } catch (error) {
      toast.show(errorMessage(error, `Could not ${value ? "enable" : "disable"} ${provider.id}.`), {
        tone: "danger",
      });
    }
  };
  const activateAccount = async (provider: ProviderRow, account: ProviderOAuthAccount) => {
    try {
      const response = await switchAccount.mutateAsync({
        provider: provider.id as OAuthProviderId,
        account_id: account.id,
      });
      toast.show(response.notice ?? "Active account changed.", { tone: "success" });
    } catch (error) {
      toast.show(errorMessage(error, "Could not switch account."), { tone: "danger" });
    }
  };

  const confirmRemoval = async () => {
    const pending = pendingRemoval;
    if (!pending) return;
    try {
      let response;
      if (pending.kind === "keys") {
        response = await removeKeys.mutateAsync(pending.provider.id);
      } else if (pending.kind === "custom") {
        response = await removeCustom.mutateAsync(pending.provider.id);
      } else if (pending.kind === "azure") {
        response = await removeAzure.mutateAsync();
      } else {
        response = await removeAccount.mutateAsync({
          provider: pending.provider.id as OAuthProviderId,
          account_id: pending.account.id,
        });
      }
      toast.show(response.notice ?? "Removed.", { tone: "success" });
      setPendingRemoval(null);
    } catch (error) {
      toast.show(errorMessage(error, "Could not remove provider data."), { tone: "danger" });
    }
  };

  const removalTitle =
    pendingRemoval?.kind === "keys"
      ? `Remove stored ${pendingRemoval.provider.id} keys?`
      : pendingRemoval?.kind === "oauth"
        ? `Remove ${pendingRemoval.account.id}?`
        : pendingRemoval
          ? `Remove ${pendingRemoval.provider.label}?`
          : "Remove provider data?";
  const removalMessage =
    pendingRemoval?.kind === "keys"
      ? "Only Forge-owned keyring/encrypted keys are removed. Environment variables remain unchanged."
      : pendingRemoval?.kind === "oauth"
        ? "This removes the selected OAuth account from the host's secure storage. Other accounts remain."
        : "The provider configuration and its Forge-owned stored keys are removed. Environment variables remain unchanged, and the daemon must restart to unload it.";

  return (
    <Screen
      scroll
      refreshControl={
        direct ? (
          <RefreshControl
            refreshing={query.isFetching || modelsQuery.isFetching}
            onRefresh={() => void Promise.all([query.refetch(), modelsQuery.refetch()])}
          />
        ) : undefined
      }
      contentContainerStyle={styles.content}
    >
      <View style={styles.headerRow}>
        <BackLink />
        <View style={styles.flexFill} />
        {direct ? (
          <>
            <Pressable
              onPress={() => setAzureSheet({ visible: true, provider: providers.find((provider) => provider.kind === "azure") ?? null })}
              accessibilityRole="button"
              accessibilityLabel="Add or edit Azure OpenAI"
            >
              <Text style={[styles.add, { color: tokens.accent }]}>Azure</Text>
            </Pressable>
            <Pressable
              onPress={() => setCustomSheet({ visible: true, provider: null })}
              accessibilityRole="button"
              accessibilityLabel="Add custom provider"
            >
              <Text style={[styles.add, { color: tokens.accent }]}>+ Endpoint</Text>
            </Pressable>
          </>
        ) : null}
      </View>
      <Text style={[type.title, { color: tokens.ink }]}>Providers &amp; accounts</Text>
      <Text style={[type.sub, { color: tokens.ink3 }]}>
        Credentials, subscription accounts, endpoints, and routing availability on this Forge host.
      </Text>

      {!direct ? (
        <View style={[styles.directBanner, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}>
          <ShieldCheck size={22} strokeWidth={1.75} color={tokens.warnBgInk} />
          <View style={styles.flexFill}>
            <Text style={[type.bodyBold, { color: tokens.ink }]}>Direct connection required</Text>
            <Text style={[type.sub, { color: tokens.ink3 }]}>
              Provider metadata and credentials never traverse Forge Anywhere. Connect to the daemon&apos;s local, LAN, or tunnel URL to manage them.
            </Text>
          </View>
        </View>
      ) : query.isLoading ? (
        <View style={styles.loading}>
          <ActivityIndicator color={tokens.accent} />
          <Text style={[type.sub, { color: tokens.ink3 }]}>Reading secure provider metadata…</Text>
        </View>
      ) : query.isError && !query.data ? (
        <EmptyState
          icon={ServerCog}
          message="Could not load providers from this daemon."
          action={<Button label="Retry" variant="secondary" onPress={() => void query.refetch()} />}
        />
      ) : (
        <>
          {query.data?.restart_required || providers.some((provider) => provider.restart_required) ? (
            <View style={[styles.restartBanner, { backgroundColor: tokens.warnBg }]}>
              <CloudCog size={20} strokeWidth={1.75} color={tokens.warnBgInk} />
              <Text style={[type.sub, styles.flexFill, { color: tokens.warnBgInk }]}>
                Restart the Forge daemon to apply pending custom or Azure endpoint changes. Existing sessions keep their current provider registry.
              </Text>
            </View>
          ) : null}
          {sections.map((section) =>
            section.rows.length > 0 ? (
              <View key={section.key}>
                <SectionHeader>{section.title}</SectionHeader>
                {section.rows.map((provider) => (
                  <ProviderItem
                    key={provider.id}
                    provider={provider}
                    modelHealth={modelHealth.get(provider.id)}
                    mutationBusy={mutationBusy}
                    onEnabled={(row, value) => void enabled(row, value)}
                    onKey={setKeyProvider}
                    onRemoveKeys={(row) => setPendingRemoval({ kind: "keys", provider: row })}
                    onEditCustom={(row) => setCustomSheet({ visible: true, provider: row })}
                    onEditAzure={(row) => setAzureSheet({ visible: true, provider: row })}
                    onRemoveCustom={(row) => setPendingRemoval({ kind: "custom", provider: row })}
                    onRemoveAzure={(row) => setPendingRemoval({ kind: "azure", provider: row })}
                    onSwitchAccount={(row, account) => void activateAccount(row, account)}
                    onRemoveAccount={(row, account) => setPendingRemoval({ kind: "oauth", provider: row, account })}
                  />
                ))}
              </View>
            ) : null,
          )}
          {providers.length === 0 ? <EmptyState icon={ServerCog} message="No providers are available on this daemon." /> : null}
          <Text style={[type.monoMeta, styles.footer, { color: tokens.ink4 }]}>
            Raw keys and OAuth tokens never leave the host. Stored fingerprints are masked; environment keys can be detected but never read or removed here.
          </Text>
        </>
      )}

      <KeySheet provider={keyProvider} onClose={() => setKeyProvider(null)} />
      <CustomProviderSheet
        visible={customSheet.visible}
        provider={customSheet.provider}
        onClose={() => setCustomSheet({ visible: false, provider: null })}
      />
      <AzureProviderSheet
        visible={azureSheet.visible}
        provider={azureSheet.provider}
        onClose={() => setAzureSheet({ visible: false, provider: null })}
      />
      <ConfirmDialog
        visible={pendingRemoval != null}
        title={removalTitle}
        message={removalMessage}
        confirmLabel="Remove"
        destructive
        loading={removeKeys.isPending || removeCustom.isPending || removeAzure.isPending || removeAccount.isPending}
        onConfirm={() => void confirmRemoval()}
        onCancel={() => setPendingRemoval(null)}
      />
    </Screen>
  );
}

export default function ProvidersScreen() {
  return (
    <DesktopDrillDown>
      <SettingsShell active="providers">
        <ProvidersScreenBody />
      </SettingsShell>
    </DesktopDrillDown>
  );
}

const styles = StyleSheet.create({
  content: {
    paddingTop: space.space12,
    paddingBottom: space.space32,
    gap: space.space12,
  },
  headerRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space16,
  },
  flexFill: { flex: 1, minWidth: 0 },
  add: { fontSize: 15, fontWeight: "600" },
  loading: {
    alignItems: "center",
    padding: space.space32,
    gap: space.space12,
  },
  directBanner: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: space.space12,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius8,
    padding: space.space16,
  },
  restartBanner: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space12,
    borderRadius: radii.radius8,
    padding: space.space12,
  },
  provider: {
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingVertical: space.space12,
    gap: space.space8,
  },
  providerHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
  },
  statusDot: { width: 7, height: 7, borderRadius: 4 },
  providerTitle: { flex: 1, minWidth: 0, gap: 2 },
  providerName: { fontSize: 14, fontFamily: monoFamily.bold },
  command: {
    borderRadius: radii.radius4,
    paddingHorizontal: space.space8,
    paddingVertical: space.space4,
  },
  accounts: { gap: space.space8 },
  account: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    borderRadius: radii.radius8,
    paddingHorizontal: space.space12,
    paddingVertical: space.space8,
  },
  accountBody: { flex: 1, minWidth: 0, gap: 2 },
  accountTitle: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  inlineMono: { fontFamily: monoFamily.regular },
  actions: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    gap: space.space8,
  },
  sheetContent: {
    paddingHorizontal: space.space16,
    paddingBottom: space.space24,
    gap: space.space12,
  },
  sheetActions: {
    flexDirection: "row",
    justifyContent: "flex-end",
    flexWrap: "wrap",
    gap: space.space8,
  },
  toggleRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space12,
    paddingVertical: space.space4,
  },
  footer: { paddingTop: space.space8 },
});
