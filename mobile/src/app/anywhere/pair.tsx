import React, { useEffect, useState } from "react";
import { StyleSheet, Text } from "react-native";
import { BackLink } from "../../components/ds/BackLink";
import { Banner } from "../../components/ds/Banner";
import { Button } from "../../components/ds/Button";
import { Card } from "../../components/ds/Card";
import { Input } from "../../components/ds/Input";
import { Screen } from "../../components/ds/Screen";
import { useAnywhere } from "../../lib/AnywhereProvider";
import { pairingCapability, parsePairingChallenge, type PairingCapability } from "../../lib/anywherePairing";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";

export default function AnywherePairScreen() {
  const anywhere = useAnywhere(); const tokens = useTokens(); const [capability, setCapability] = useState<PairingCapability | null>(null); const [challenge, setChallenge] = useState(""); const [error, setError] = useState<string | null>(null);
  const credentials = anywhere.credentials; const accessToken = anywhere.accessToken;
  useEffect(() => { if (!credentials) return; void accessToken().then((token) => pairingCapability(credentials.serviceUrl ?? "https://app.forge.adulari.dev", token)).then(setCapability).catch((reason: unknown) => setError(reason instanceof Error ? reason.message : "Pairing unavailable")); }, [accessToken, credentials]);
  const review = () => { try { if (!anywhere.credentials) return; parsePairingChallenge(challenge, anywhere.credentials.serviceUrl ?? "https://app.forge.adulari.dev"); setError("Challenge is valid, but enrollment is disabled until this service exposes /v1/pairings."); } catch (reason) { setError(reason instanceof Error ? reason.message : "Invalid QR challenge"); } };
  return <Screen scroll keyboardAvoiding contentContainerStyle={styles.content}><BackLink label="Devices" /><Text style={[type.title, { color: tokens.ink }]}>Pair with QR</Text><Text style={[type.sub, { color: tokens.ink2 }]}>Pairing challenges expire within ten minutes and contain only the new device’s public exchange key. Account keys are wrapped after explicit approval.</Text>{error ? <Banner tone="danger" message={error} /> : null}{capability && !capability.supported ? <Banner tone="warn" message={capability.message} /> : null}<Card style={styles.card}><Input label="Scanned pairing challenge" value={challenge} onChangeText={setChallenge} autoCapitalize="none" autoCorrect={false} multiline /><Button label="Review challenge" disabled={!challenge || capability?.supported !== true} onPress={review} fullWidth /></Card><Text style={[type.meta, { color: tokens.ink3 }]}>No challenge, recovery word, private key, or QR payload is stored by this screen.</Text></Screen>;
}
const styles = StyleSheet.create({ content: { paddingTop: space.space24, paddingBottom: space.space32, gap: space.space16 }, card: { gap: space.space12 } });
