import { useLocalSearchParams } from "expo-router";
import * as Linking from "expo-linking";
import React, { useEffect, useState } from "react";
import { StyleSheet, Text } from "react-native";

import { Banner } from "../../components/ds/Banner";
import { Card } from "../../components/ds/Card";
import { Screen } from "../../components/ds/Screen";
import { DEFAULT_ANYWHERE_SERVICE_URL } from "../../lib/anywhereApi";
import { retrieveReplayShare, trustedReplaySigner, type ReplayShare } from "../../lib/anywhereShare";
import { useAnywhere } from "../../lib/AnywhereProvider";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";

export default function ReplayShareScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const anywhere = useAnywhere();
  const tokens = useTokens();
  const linkedUrl = Linking.useURL();
  const [share, setShare] = useState<ReplayShare | null>(null);
  const [error, setError] = useState<string | null>(null);
  const serviceUrl = anywhere.credentials?.serviceUrl ?? process.env.EXPO_PUBLIC_FORGE_ANYWHERE_URL ?? DEFAULT_ANYWHERE_SERVICE_URL;

  useEffect(() => {
    const browserHref = typeof window !== "undefined" ? window.location.href : null;
    const href = linkedUrl ?? browserHref;
    if (!id || !href) { setError("Open the complete replay link, including its #key fragment."); return; }
    let active = true;
    void retrieveReplayShare({ serviceUrl, shareId: id, href, resolveSigningKey: (sender) => trustedReplaySigner(anywhere.credentials?.signingPublicKeys ?? {}, sender) })
      .then((value) => { if (active) setShare(value); })
      .catch((reason: unknown) => { if (active) setError(reason instanceof Error ? reason.message : "Replay could not be opened"); });
    return () => { active = false; setShare(null); };
  }, [anywhere.credentials?.signingPublicKeys, id, linkedUrl, serviceUrl]);

  return <Screen scroll contentContainerStyle={styles.content}>
    <Text style={[type.title, { color: tokens.ink }]}>Encrypted replay</Text>
    <Text style={[type.sub, { color: tokens.ink2 }]}>The decryption and signing keys stay in this URL’s fragment. Forge downloads only ciphertext and never caches decrypted replay data.</Text>
    {error ? <Banner tone="danger" message={error} /> : null}
    {!error && !share ? <Text style={[type.sub, { color: tokens.ink3 }]}>Verifying signature and decrypting…</Text> : null}
    {share ? <>
      <Card style={styles.gap}>
        <Text style={[type.heading, { color: tokens.ink }]}>Session {share.session_id}</Text>
        <Text style={[type.meta, { color: tokens.ink3 }]}>Expires {new Date(share.expires_at_ms).toLocaleString()}</Text>
      </Card>
      <Card><Text selectable style={[type.code, { color: tokens.ink }]}>{JSON.stringify(share.replay, null, 2)}</Text></Card>
    </> : null}
  </Screen>;
}

const styles = StyleSheet.create({ content: { paddingTop: space.space24, paddingBottom: space.space32, gap: space.space16 }, gap: { gap: space.space8 } });
