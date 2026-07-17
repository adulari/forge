import React, { useState } from "react";
import { StyleSheet, Text, View } from "react-native";
import { BackLink } from "../../components/ds/BackLink";
import { Badge } from "../../components/ds/Badge";
import { Banner } from "../../components/ds/Banner";
import { Button } from "../../components/ds/Button";
import { Card } from "../../components/ds/Card";
import { Input } from "../../components/ds/Input";
import { ListRow } from "../../components/ds/ListRow";
import { Screen } from "../../components/ds/Screen";
import { useAnywhere } from "../../lib/AnywhereProvider";
import { cancelCapsule, capsuleStatus, handoffOutcome, handoffRecovery, pendingCapsules, type CapsuleStatus, type HandoffOutcome, type PendingCapsule } from "../../lib/anywhereHandoff";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";

export default function AnywhereHandoffScreen() {
  const anywhere = useAnywhere(); const tokens = useTokens();
  const [source, setSource] = useState(""); const [destination, setDestination] = useState(""); const [capsuleId, setCapsuleId] = useState("");
  const [pending, setPending] = useState<PendingCapsule[]>([]); const [status, setStatus] = useState<CapsuleStatus | null>(null);
  const [outcome, setOutcome] = useState<HandoffOutcome | null>(null); const [error, setError] = useState<string | null>(null); const [busy, setBusy] = useState(false);
  const run = async (work: (service: string, token: string) => Promise<void>) => { const credentials = anywhere.credentials; if (!credentials) return; setBusy(true); setError(null); try { await work(credentials.serviceUrl ?? "https://app.forge.adulari.dev", await anywhere.accessToken()); } catch (reason) { setOutcome("indeterminate"); setError(reason instanceof Error ? reason.message : "Handoff status is uncertain"); } finally { setBusy(false); } };
  const find = () => run(async (service, token) => { const result = await pendingCapsules(service, token, destination); setPending(result.capsules); });
  const refresh = () => run(async (service, token) => { const next = await capsuleStatus(service, token, capsuleId); setStatus(next); setOutcome(handoffOutcome(next)); });
  const cancel = () => run(async (service, token) => { const next = await cancelCapsule(service, token, capsuleId); setStatus(next); setOutcome(handoffOutcome(next)); });
  return <Screen scroll keyboardAvoiding contentContainerStyle={styles.content}>
    <BackLink label="Anywhere" /><Text style={[type.title, { color: tokens.ink }]}>Workspace handoff</Text>
    <Text style={[type.sub, { color: tokens.ink2 }]}>A host creates the encrypted capsule only at an idle checkpoint. The destination must acknowledge import before the service transfers the session lease.</Text>
    {error ? <Banner tone="danger" message={error} /> : null}
    <Card padded={false}>{anywhere.hosts.map((host, index) => <ListRow key={host.id} title={host.name} subtitle={host.id === source ? "source host" : host.id === destination ? "destination host" : "Tap once for source, twice for destination"} trailing={host.id === source ? <Badge label="source" tone="neutral" /> : host.id === destination ? <Badge label="destination" tone="accent" /> : null} onPress={() => { if (!source || source === host.id) { setSource(host.id); if (destination === host.id) setDestination(""); } else setDestination(host.id); }} showSeparator={index !== anywhere.hosts.length - 1} />)}</Card>
    <Button label="Find pending capsules" variant="secondary" loading={busy} disabled={!destination || source === destination} onPress={() => void find()} fullWidth />
    {pending.map((capsule) => <Card key={capsule.capsule_id} style={styles.card}><Text style={[type.bodyBold, { color: tokens.ink }]}>Pending from {name(anywhere.hosts, capsule.source_host_id)}</Text><Text style={[type.meta, { color: tokens.ink3 }]}>Expires {new Date(capsule.expires_at_ms).toLocaleString()} · encrypted {formatBytes(capsule.ciphertext_bytes)}</Text><Button label="Track this handoff" variant="ghost" onPress={() => { setCapsuleId(capsule.capsule_id); setOutcome("pending"); }} /></Card>)}
    <Card style={styles.card}><Input label="Capsule id" value={capsuleId} onChangeText={setCapsuleId} autoCapitalize="none" autoCorrect={false} /><View style={styles.actions}><Button label="Refresh status" variant="secondary" loading={busy} disabled={!/^[0-9a-f]{32}$/.test(capsuleId)} onPress={() => void refresh()} style={styles.grow} /><Button label="Cancel safely" variant="danger" disabled={!status || !["reserved", "ready", "claimed"].includes(status.state)} onPress={() => void cancel()} style={styles.grow} /></View></Card>
    {outcome ? <Card variant={outcome === "accepted" ? "feature" : "default"} style={styles.card}><View style={styles.row}><Text style={[type.heading, styles.grow, { color: tokens.ink }]}>Handoff {outcome}</Text><Badge label={outcome} tone={outcome === "accepted" ? "success" : outcome === "failed" || outcome === "indeterminate" ? "warn" : "neutral"} /></View><Text style={[type.sub, { color: tokens.ink2 }]}>{handoffRecovery(outcome)}</Text></Card> : null}
    <Text style={[type.meta, { color: tokens.ink3 }]}>Starting a capsule remains a host-side action because only the host can pause tools, inspect Git safety, and export the workspace without exposing plaintext to this controller.</Text>
  </Screen>;
}
function name(hosts: { id: string; name: string }[], id: string): string { return hosts.find((host) => host.id === id)?.name ?? id.slice(0, 8); }
function formatBytes(bytes: number): string { return bytes < 1024 ** 2 ? `${Math.round(bytes / 1024)} KB` : `${(bytes / 1024 ** 2).toFixed(1)} MB`; }
const styles = StyleSheet.create({ content: { paddingTop: space.space24, paddingBottom: space.space32, gap: space.space16 }, card: { gap: space.space8 }, actions: { flexDirection: "row", gap: space.space8 }, row: { flexDirection: "row", alignItems: "center", gap: space.space8 }, grow: { flex: 1 } });
