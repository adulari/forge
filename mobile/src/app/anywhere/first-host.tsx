import { Redirect, router } from "expo-router";
import { Check, Laptop, ShieldCheck } from "lucide-react-native";
import React, { useCallback, useEffect, useState } from "react";
import { Platform, StyleSheet, Text, View } from "react-native";

import { BackLink } from "../../components/ds/BackLink";
import { Button } from "../../components/ds/Button";
import { Input } from "../../components/ds/Input";
import { Screen } from "../../components/ds/Screen";
import { useToast } from "../../components/ds/ToastHost";
import { useAnywhere } from "../../lib/AnywhereProvider";
import { isTauri } from "../../lib/platform";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";

export default function FirstHostScreen() {
  const anywhere = useAnywhere();
  const tokens = useTokens();
  const toast = useToast();
  const [name, setName] = useState("localhost");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!isTauri) return;
    void import("@tauri-apps/api/core").then(({ invoke }) => invoke<string>("system_host_name")).then(setName).catch(() => undefined);
  }, []);

  const prepare = useCallback(async () => {
    setBusy(true);
    try {
      const result = await anywhere.prepareLocalHost(name);
      if (result === "activated") {
        toast.show("This computer is now active as a Forge Anywhere host.", { tone: "neutral" });
        router.replace("/anywhere");
      }
    } catch (reason) {
      toast.show(reason instanceof Error ? reason.message : "This host could not be prepared.", { tone: "danger" });
    } finally { setBusy(false); }
  }, [anywhere, name, toast]);

  const confirm = useCallback(async () => {
    setBusy(true);
    try {
      await anywhere.confirmLocalHost();
      toast.show("This computer is now active as a Forge Anywhere host.", { tone: "neutral" });
      router.replace("/anywhere");
    } catch (reason) {
      toast.show(reason instanceof Error ? reason.message : "This host could not be activated.", { tone: "danger" });
    } finally { setBusy(false); }
  }, [anywhere, toast]);

  if (anywhere.phase !== "ready") return <Redirect href="/anywhere" />;

  if (!isTauri) return <Screen scroll contentContainerStyle={styles.screen}><View style={styles.shell}><BackLink label="Forge Anywhere" /><Text style={[typeScale.title, styles.title, { color: tokens.ink }]}>Activate a host</Text><Text style={[typeScale.body, styles.subtitle, { color: tokens.ink2 }]}>{Platform.OS === "web" ? "A browser can control Forge Anywhere, but a host needs Forge Desktop running on the computer." : "Mobile can approve and control hosts, but it cannot run one. Open Forge Desktop on the computer you want to use."}</Text></View></Screen>;

  const approval = anywhere.localHostApproval;
  return <Screen scroll keyboardAvoiding contentContainerStyle={styles.screen}><View style={styles.shell}>
    <BackLink label="Forge Anywhere" />
    <View style={styles.header}><View style={[styles.icon, { backgroundColor: tokens.selection }]}><Laptop size={23} color={tokens.accent} /></View><View style={styles.headerCopy}><Text accessibilityRole="header" style={[typeScale.title, { color: tokens.ink }]}>Activate this computer</Text><Text style={[typeScale.sub, { color: tokens.ink3 }]}>Forge Desktop handles enrollment and connector startup for you.</Text></View></View>

    {!approval ? <View style={styles.form}>
      <Input label="Host name" value={name} onChangeText={setName} maxLength={80} autoCapitalize="none" accessibilityHint="The name shown to your other Forge devices" />
      <View style={styles.promise}><Check size={17} color={tokens.success} /><Text style={[typeScale.sub, styles.promiseCopy, { color: tokens.ink2 }]}>A separate encrypted device identity is created for the host.</Text></View>
      <View style={styles.promise}><Check size={17} color={tokens.success} /><Text style={[typeScale.sub, styles.promiseCopy, { color: tokens.ink2 }]}>The managed connector starts in the background after approval.</Text></View>
      <Button label="Prepare this host" onPress={() => void prepare()} loading={busy} disabled={!name.trim()} fullWidth style={styles.action} />
    </View> : <View style={styles.form}>
      <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Approve the local host</Text>
      <Text style={[typeScale.body, { color: tokens.ink2 }]}>Forge created a distinct identity for <Text style={typeScale.bodyBold}>{approval.name}</Text>. Confirm this safety code to install it on this computer.</Text>
      <View style={[styles.safety, { backgroundColor: tokens.bg2, borderColor: tokens.borderStrong }]}><View style={styles.safetyHeader}><ShieldCheck size={18} color={tokens.info} /><Text style={[typeScale.bodyBold, { color: tokens.ink }]}>Local pairing safety code</Text></View><Text selectable style={[styles.code, tabularNums, { color: tokens.ink }]}>{approval.safetyCode}</Text></View>
      <View style={styles.actions}><Button label="Cancel" variant="ghost" disabled={busy} onPress={anywhere.cancelLocalHost} style={styles.flex} /><Button label="Approve and activate" variant="allow" loading={busy} onPress={() => void confirm()} style={styles.flex} /></View>
    </View>}
  </View></Screen>;
}

const styles = StyleSheet.create({
  screen: { paddingTop: space.space12, paddingBottom: space.space48 }, shell: { width: "100%", maxWidth: 640, alignSelf: "center" }, title: { marginTop: space.space12 }, subtitle: { marginTop: space.space4 },
  header: { flexDirection: "row", alignItems: "center", gap: space.space12, marginTop: space.space12 }, icon: { width: 46, height: 46, borderRadius: radii.radius12, alignItems: "center", justifyContent: "center" }, headerCopy: { flex: 1 },
  form: { marginTop: space.space24, gap: space.space12 }, promise: { flexDirection: "row", alignItems: "flex-start", gap: space.space8 }, promiseCopy: { flex: 1 }, action: { marginTop: space.space4 },
  safety: { borderWidth: 1, borderRadius: radii.radius12, padding: space.space16, gap: space.space8 }, safetyHeader: { flexDirection: "row", alignItems: "center", gap: space.space8 }, code: { fontFamily: monoFamily.bold, fontSize: 27, lineHeight: 35, letterSpacing: 2.2 },
  actions: { flexDirection: "row", flexWrap: "wrap", gap: space.space8 }, flex: { flexGrow: 1, flexBasis: 190 },
});
