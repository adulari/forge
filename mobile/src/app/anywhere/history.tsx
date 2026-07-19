import { AlertTriangle, RefreshCw, Trash2 } from "lucide-react-native";
import React, { useCallback, useEffect, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { BackLink } from "../../components/ds/BackLink";
import { Badge } from "../../components/ds/Badge";
import { Banner } from "../../components/ds/Banner";
import { Button } from "../../components/ds/Button";
import { Card } from "../../components/ds/Card";
import { Screen } from "../../components/ds/Screen";
import { useAnywhere } from "../../lib/AnywhereProvider";
import { clearOfflineHistory, decryptSyncChange, fetchSyncFeed, markSyncConflicts, readOfflineHistory, syncPayloadText, writeOfflineHistory, type OfflineHistoryEntry } from "../../lib/anywhereSyncBrowser";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";

export default function AnywhereHistoryScreen() {
  const anywhere = useAnywhere(); const tokens = useTokens();
  const [entries, setEntries] = useState<OfflineHistoryEntry[]>([]);
  const [expanded, setExpanded] = useState<number | null>(null);
  const [busy, setBusy] = useState(false); const [error, setError] = useState<string | null>(null);
  const credentials = anywhere.credentials;

  useEffect(() => { if (credentials) void readOfflineHistory(credentials).then(setEntries); }, [credentials]);
  const refresh = useCallback(async () => {
    if (!credentials) return; setBusy(true); setError(null);
    try {
      const token = await anywhere.accessToken();
      const feed = await fetchSyncFeed(credentials.serviceUrl ?? "https://app.forge.adulari.dev", token, 0);
      const decrypted = await Promise.all(feed.changes.map((change) => decryptSyncChange(change, credentials)));
      const next = markSyncConflicts(decrypted).sort((a, b) => b.cursor - a.cursor);
      await writeOfflineHistory(credentials, next); setEntries(next);
    } catch (reason) { setError(reason instanceof Error ? reason.message : "History could not be refreshed"); }
    finally { setBusy(false); }
  }, [anywhere, credentials]);
  const clear = async () => { if (!credentials) return; await clearOfflineHistory(credentials); setEntries([]); };

  return <Screen scroll contentContainerStyle={styles.content}>
    <BackLink label="Anywhere" />
    <Text style={[type.title, { color: tokens.ink }]}>Offline encrypted history</Text>
    <Text style={[type.sub, { color: tokens.ink2 }]}>Objects are signature-checked, decrypted in memory, then cached as device-bound ciphertext. Provider credentials and host-only data are excluded upstream.</Text>
    {error ? <Banner tone="danger" message={error} /> : null}
    <View style={styles.actions}><Button label="Refresh" loading={busy} icon={<RefreshCw size={18} color={tokens.ink} />} variant="secondary" onPress={() => void refresh()} style={styles.grow} /><Button label="Clear offline" icon={<Trash2 size={18} color={tokens.danger} />} variant="ghost" onPress={() => void clear()} style={styles.grow} /></View>
    {entries.length === 0 ? <Card><Text style={[type.sub, { color: tokens.ink3 }]}>No decrypted records are cached on this device.</Text></Card> : entries.map((entry) => <Card key={`${entry.cursor}-${entry.record.stable_id}`} style={styles.card}>
      <View style={styles.row}><Text style={[type.bodyBold, styles.grow, { color: tokens.ink }]}>{entry.record.kind.replaceAll("_", " ")}</Text>{entry.conflict ? <Badge label="conflict copy" tone="warn" /> : entry.record.operation === "tombstone" ? <Badge label="deleted" tone="neutral" /> : null}</View>
      <Text style={[type.meta, { color: tokens.ink3 }]}>Revision {entry.record.revision} · {new Date(entry.createdAt).toLocaleString()}</Text>
      {entry.conflict ? <View style={styles.row}><AlertTriangle size={16} color={tokens.warn} /><Text style={[type.sub, styles.grow, { color: tokens.warn }]}>Divergent file bases were preserved; choose a copy on a host.</Text></View> : null}
      <Button label={expanded === entry.cursor ? "Hide content" : "Browse content"} variant="ghost" onPress={() => setExpanded(expanded === entry.cursor ? null : entry.cursor)} />
      {expanded === entry.cursor ? <Text selectable style={[type.codeSmall, { color: tokens.ink }]}>{syncPayloadText(entry)}</Text> : null}
    </Card>)}
  </Screen>;
}
const styles = StyleSheet.create({ content: { paddingTop: space.space24, paddingBottom: space.space32, gap: space.space16 }, actions: { flexDirection: "row", gap: space.space8 }, grow: { flex: 1 }, card: { gap: space.space8 }, row: { flexDirection: "row", alignItems: "center", gap: space.space8 } });
