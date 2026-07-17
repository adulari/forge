import { router } from "expo-router";
import { Bell, BriefcaseBusiness, Cloud, CreditCard, GitCompareArrows, HardDrive, History, Laptop, RefreshCw, ShieldCheck, Smartphone } from "lucide-react-native";
import React, { useMemo, useState } from "react";
import { ActivityIndicator, StyleSheet, Text, View } from "react-native";

import { BackLink } from "../../components/ds/BackLink";
import { Badge } from "../../components/ds/Badge";
import { Banner } from "../../components/ds/Banner";
import { Button } from "../../components/ds/Button";
import { Card } from "../../components/ds/Card";
import { Input } from "../../components/ds/Input";
import { KeyValueRow } from "../../components/ds/KeyValueRow";
import { ListRow } from "../../components/ds/ListRow";
import { Screen } from "../../components/ds/Screen";
import { SectionHeader } from "../../components/ds/SectionHeader";
import { useAnywhere } from "../../lib/AnywhereProvider";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";

export default function AnywhereAccountScreen() {
  const anywhere = useAnywhere();
  const tokens = useTokens();
  const [answers, setAnswers] = useState<Record<number, string>>({});
  const [recovery, setRecovery] = useState("");
  const words = useMemo(() => anywhere.recoveryWords?.split(" ") ?? [], [anywhere.recoveryWords]);

  if (anywhere.phase === "loading") {
    return <Screen contentContainerStyle={styles.center}><ActivityIndicator color={tokens.ink3} /></Screen>;
  }

  if (anywhere.phase === "new_recovery" && words.length === 24) {
    return (
      <Screen scroll keyboardAvoiding contentContainerStyle={styles.content}>
        <BackLink label="Connect" />
        <View style={styles.hero}>
          <Text style={[type.title, { color: tokens.ink }]}>Save your recovery words</Text>
          <Text style={[type.body, { color: tokens.ink2 }]}>These words are the only way to recover encrypted history on a new device. Forge cannot reset them.</Text>
        </View>
        <Card variant="feature" style={styles.wordGrid} accessibilityLabel="24 word recovery phrase">
          {words.map((word, index) => (
            <View key={`${index}-${word}`} style={styles.word}>
              <Text style={[type.meta, { color: tokens.ink3 }]}>{index + 1}</Text>
              <Text selectable style={[type.body, { color: tokens.ink, fontFamily: monoFamily.regular }]}>{word}</Text>
            </View>
          ))}
        </Card>
        <Banner tone="warn" message="Store this phrase offline. It will not be shown again after confirmation." />
        <Card style={styles.cardGap}>
          <Text style={[type.bodyBold, { color: tokens.ink }]}>Confirm three words</Text>
          {anywhere.recoverySample.map((index) => (
            <Input key={index} label={`Word ${index + 1}`} value={answers[index] ?? ""} onChangeText={(value) => setAnswers((current) => ({ ...current, [index]: value }))} autoCapitalize="none" autoCorrect={false} />
          ))}
          {anywhere.error ? <Text style={[type.sub, { color: tokens.danger }]}>{anywhere.error}</Text> : null}
          <Button label="Confirm and enable encryption" onPress={() => void anywhere.confirmNewRecovery(answers)} disabled={anywhere.recoverySample.some((index) => !answers[index]?.trim())} fullWidth />
        </Card>
      </Screen>
    );
  }

  if (anywhere.phase === "existing_recovery") {
    return (
      <Screen scroll keyboardAvoiding contentContainerStyle={styles.content}>
        <BackLink label="Connect" />
        <View style={styles.hero}>
          <Text style={[type.title, { color: tokens.ink }]}>Recover encrypted access</Text>
          <Text style={[type.body, { color: tokens.ink2 }]}>GitHub confirmed your identity. Enter your 24 recovery words to enroll this device and decrypt your own data.</Text>
        </View>
        <Card style={styles.cardGap}>
          <Input label="24-word recovery phrase" value={recovery} onChangeText={setRecovery} autoCapitalize="none" autoCorrect={false} multiline numberOfLines={4} secureTextEntry />
          {anywhere.error ? <Text style={[type.sub, { color: tokens.danger }]}>{anywhere.error}</Text> : null}
          <Button label="Recover this device" onPress={() => void anywhere.recoverExisting(recovery)} disabled={recovery.trim().split(/\s+/).length !== 24} fullWidth />
        </Card>
      </Screen>
    );
  }

  const signedOut = !anywhere.credentials;
  if (signedOut) {
    return (
      <Screen scroll contentContainerStyle={styles.content}>
        <BackLink label="Connect" />
        <View style={styles.hero}>
          <Text style={[type.display, { color: tokens.ink }]}>Forge Anywhere</Text>
          <Text style={[type.body, { color: tokens.ink2 }]}>Leave your desk without leaving your Forge session. Reach enrolled hosts with end-to-end encryption while local and LAN access stay unchanged.</Text>
        </View>
        <Card variant="feature" style={styles.cardGap}>
          <View style={styles.iconTitle}><ShieldCheck size={22} color={tokens.accent} /><Text style={[type.heading, { color: tokens.ink }]}>Private by design</Text></View>
          <Text style={[type.sub, { color: tokens.ink2 }]}>The service routes encrypted envelopes. Your daemon token, prompts, diffs, and recovery secret never leave your devices.</Text>
          <KeyValueRow label="Trial" value="14 days · no card" />
          <KeyValueRow label="Plans" value="€10 monthly · €79 yearly" />
          <KeyValueRow label="Included" value="3 hosts · 5 GB" />
        </Card>
        {anywhere.phase === "authorizing" && anywhere.flow ? (
          <Card style={styles.cardGap}>
            <Text style={[type.heading, { color: tokens.ink }]}>Finish on GitHub</Text>
            <Text style={[type.sub, { color: tokens.ink2 }]}>Enter this one-time code in the GitHub tab that just opened. Keep Forge open here while it waits for approval.</Text>
            <Text selectable style={[type.display, styles.code, { color: tokens.accent }]}>{anywhere.flow.user_code}</Text>
            <Button label="Open GitHub in a new tab" variant="secondary" onPress={() => void anywhere.openLoginPage()} fullWidth />
            <View style={styles.waiting}><ActivityIndicator color={tokens.ink3} /><Text style={[type.sub, { color: tokens.ink2 }]}>Waiting for authorization…</Text></View>
          </Card>
        ) : (
          <Button label={anywhere.phase === "starting" ? "Starting secure login…" : "Start 14-day trial with GitHub"} onPress={() => void anywhere.startLogin()} loading={anywhere.phase === "starting"} fullWidth />
        )}
        {anywhere.error ? <Banner tone="danger" message={anywhere.error} actionLabel="Try again" onAction={() => void anywhere.startLogin()} /> : null}
        <Button label="Use a direct Forge server" variant="ghost" onPress={() => router.replace("/connect")} fullWidth />
      </Screen>
    );
  }

  const entitlement = anywhere.account?.entitlement ?? "checking";
  const enrolled = anywhere.credentials;
  if (!enrolled) return null;
  return (
    <Screen scroll contentContainerStyle={styles.content}>
      <BackLink label="Settings" />
      <View style={styles.titleRow}>
        <View style={styles.titleGrow}>
          <Text style={[type.title, { color: tokens.ink }]}>Forge Anywhere</Text>
          <Text style={[type.sub, { color: tokens.ink2 }]}>{enrolled.githubLogin ? `Signed in as @${enrolled.githubLogin}` : "Encrypted account connected"}</Text>
        </View>
        <Badge label={entitlement.replaceAll("_", " ")} tone={entitlement === "active" || entitlement === "trialing" ? "success" : entitlement === "grace" ? "warn" : "neutral"} />
      </View>
      {anywhere.error ? <Banner tone="danger" message={anywhere.error} actionLabel="Retry" onAction={() => void anywhere.refresh()} /> : null}
      <Card padded={false}>
        <ListRow title="Hosts" subtitle={`${anywhere.hosts.length} of 3 enrolled`} leading={<Laptop size={20} color={tokens.ink2} />} onPress={() => router.push("/anywhere/hosts")} />
        <ListRow title="Devices" subtitle={`${anywhere.devices.length} enrolled`} leading={<Smartphone size={20} color={tokens.ink2} />} onPress={() => router.push("/anywhere/devices")} />
        <ListRow title="Notifications" subtitle={pushStatusLabel(anywhere.pushStatus)} leading={<Bell size={20} color={tokens.ink2} />} onPress={() => router.push("/anywhere/notifications")} />
        <ListRow title="Remote jobs" subtitle="Encrypted offline queue" leading={<BriefcaseBusiness size={20} color={tokens.ink2} />} onPress={() => router.push("/anywhere/jobs")} />
        <ListRow title="Workspace handoff" subtitle="Move an idle workspace safely" leading={<GitCompareArrows size={20} color={tokens.ink2} />} onPress={() => router.push("/anywhere/handoff")} />
        <ListRow title="Offline history" subtitle="Device-encrypted synced records" leading={<History size={20} color={tokens.ink2} />} onPress={() => router.push("/anywhere/history")} />
        <ListRow title="Encrypted storage" subtitle={formatBytes(anywhere.account?.storage_used_bytes ?? 0)} leading={<HardDrive size={20} color={tokens.ink2} />} onPress={() => router.push("/anywhere/storage")} />
        <ListRow title="Plan and billing" subtitle="€79 yearly · €10 monthly" leading={<CreditCard size={20} color={tokens.ink2} />} onPress={() => router.push("/anywhere/billing")} showSeparator={false} />
      </Card>
      <SectionHeader>Account status</SectionHeader>
      <Card padded={false}>
        <KeyValueRow label="Entitlement" value={entitlement.replaceAll("_", " ")} />
        <KeyValueRow label="Trial ends" value={formatDate(anywhere.account?.trial_ends_at)} />
        <KeyValueRow label="Encryption" value={`epoch ${enrolled.keyEpoch}`} />
        <KeyValueRow label="Service" value="app.forge.adulari.dev" />
      </Card>
      <Button label="Refresh status" variant="secondary" icon={<RefreshCw size={18} color={tokens.ink} />} onPress={() => void anywhere.refresh()} fullWidth />
      <Button label="Log out of Anywhere" variant="ghost" onPress={() => void anywhere.logout()} fullWidth />
      <View style={styles.footnote}><Cloud size={16} color={tokens.ink3} /><Text style={[type.meta, styles.titleGrow, { color: tokens.ink3 }]}>Logging out removes this device’s local tokens and keys. Direct Forge servers remain available.</Text></View>
    </Screen>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB used`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB used`;
}

function pushStatusLabel(status: "unsupported" | "denied" | "unsubscribed" | "subscribed"): string {
  if (status === "subscribed") return "Generic alerts enabled";
  if (status === "denied") return "Blocked in iOS Settings";
  if (status === "unsupported") return "Available on iPhone";
  return "Off";
}

function formatDate(value: string | null | undefined): string {
  if (!value) return "—";
  const timestamp = Number(value) * 1000;
  return Number.isFinite(timestamp) ? new Date(timestamp).toLocaleDateString() : "—";
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space24, paddingBottom: space.space32, gap: space.space16 },
  center: { flex: 1, justifyContent: "center", alignItems: "center" },
  hero: { gap: space.space8, maxWidth: 720 },
  cardGap: { gap: space.space12 },
  iconTitle: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  code: { textAlign: "center", letterSpacing: 2 },
  waiting: { flexDirection: "row", alignItems: "center", justifyContent: "center", gap: space.space8 },
  wordGrid: { flexDirection: "row", flexWrap: "wrap", gap: space.space8 },
  word: { width: "30%", minWidth: 96, flexGrow: 1, flexDirection: "row", gap: space.space8 },
  titleRow: { flexDirection: "row", alignItems: "flex-start", gap: space.space12 },
  titleGrow: { flex: 1 },
  footnote: { flexDirection: "row", gap: space.space8, alignItems: "flex-start" },
});
