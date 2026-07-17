import { CalendarDays, CreditCard, ExternalLink } from "lucide-react-native";
import React, { useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { BackLink } from "../../components/ds/BackLink";
import { Badge } from "../../components/ds/Badge";
import { Banner } from "../../components/ds/Banner";
import { Button } from "../../components/ds/Button";
import { Card } from "../../components/ds/Card";
import { KeyValueRow } from "../../components/ds/KeyValueRow";
import { Screen } from "../../components/ds/Screen";
import { SectionHeader } from "../../components/ds/SectionHeader";
import { useAnywhere } from "../../lib/AnywhereProvider";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";

export default function AnywhereBillingScreen() {
  const anywhere = useAnywhere();
  const tokens = useTokens();
  const [opening, setOpening] = useState<"annual" | "monthly" | "portal" | null>(null);
  const subscription = anywhere.subscription;
  const state = subscription?.state ?? anywhere.account?.entitlement ?? "checking";
  const hasCustomer = subscription?.subscription_status != null;

  const openCheckout = async (period: "annual" | "monthly") => {
    setOpening(period);
    try { await anywhere.checkout(period); } catch { /* Provider exposes the safe service error. */ }
    finally { setOpening(null); }
  };

  const openPortal = async () => {
    setOpening("portal");
    try { await anywhere.openBillingPortal(); } catch { /* Provider exposes the safe service error. */ }
    finally { setOpening(null); }
  };

  return (
    <Screen scroll contentContainerStyle={styles.content}>
      <BackLink label="Anywhere" />
      <View style={styles.titleRow}>
        <View style={styles.titleGrow}>
          <Text style={[type.title, { color: tokens.ink }]}>Plan and billing</Text>
          <Text style={[type.sub, { color: tokens.ink2 }]}>Annual is selected by default. Paddle handles payment details outside Forge.</Text>
        </View>
        <Badge label={state.replaceAll("_", " ")} tone={state === "active" || state === "trialing" ? "success" : state === "grace" ? "warn" : "neutral"} />
      </View>
      {anywhere.error ? <Banner tone="danger" message={anywhere.error} /> : null}

      <Card variant="feature" style={styles.plan}>
        <View style={styles.planHeading}>
          <CalendarDays size={22} color={tokens.accent} />
          <View style={styles.titleGrow}>
            <Text style={[type.heading, { color: tokens.ink }]}>Yearly</Text>
            <Text style={[type.sub, { color: tokens.ink2 }]}>€79 per year · save €41</Text>
          </View>
          <Badge label="default" tone="accent" />
        </View>
        <Button label="Choose yearly" loading={opening === "annual"} disabled={opening !== null && opening !== "annual"} onPress={() => void openCheckout("annual")} fullWidth />
      </Card>

      <Card style={styles.plan}>
        <View style={styles.planHeading}>
          <CreditCard size={22} color={tokens.ink2} />
          <View style={styles.titleGrow}>
            <Text style={[type.heading, { color: tokens.ink }]}>Monthly</Text>
            <Text style={[type.sub, { color: tokens.ink2 }]}>€10 per month</Text>
          </View>
        </View>
        <Button label="Choose monthly" variant="secondary" loading={opening === "monthly"} disabled={opening !== null && opening !== "monthly"} onPress={() => void openCheckout("monthly")} fullWidth />
      </Card>

      <SectionHeader>Subscription status</SectionHeader>
      <Card padded={false}>
        <KeyValueRow label="Access" value={state.replaceAll("_", " ")} />
        <KeyValueRow label="Paid through" value={formatTimestamp(subscription?.paid_through)} />
        <KeyValueRow label="Trial ends" value={formatTimestamp(subscription?.trial_ends_at)} />
        <KeyValueRow label="Renewal" value={subscription?.cancel_at_period_end ? "cancels at period end" : "automatic"} />
      </Card>
      {hasCustomer ? (
        <Button label="Manage billing in Paddle" variant="secondary" icon={<ExternalLink size={18} color={tokens.ink} />} loading={opening === "portal"} disabled={opening !== null && opening !== "portal"} onPress={() => void openPortal()} fullWidth />
      ) : null}
      <Text style={[type.meta, { color: tokens.ink3 }]}>Forge stays open source and free. Anywhere is optional; direct, LAN, and user-managed tunnel access do not depend on this plan.</Text>
    </Screen>
  );
}

function formatTimestamp(timestamp: number | null | undefined): string {
  return timestamp == null ? "—" : new Date(timestamp * 1000).toLocaleDateString();
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space24, paddingBottom: space.space32, gap: space.space16 },
  titleRow: { flexDirection: "row", alignItems: "flex-start", gap: space.space12 },
  titleGrow: { flex: 1 },
  plan: { gap: space.space12 },
  planHeading: { flexDirection: "row", alignItems: "center", gap: space.space12 },
});
