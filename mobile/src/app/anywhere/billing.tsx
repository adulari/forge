// Forge Anywhere — billing & entitlement (mobile.dc.html "AW Billing", lines 1185-1231).
import { router } from "expo-router";
import { Check } from "lucide-react-native";
import React, { useEffect, useState } from "react";
import { Linking, Pressable, StyleSheet, Text, View } from "react-native";

import { BackLink } from "../../components/ds/BackLink";
import { Button } from "../../components/ds/Button";
import { Screen } from "../../components/ds/Screen";
import { EntitlementBadge } from "../../components/anywhere/EntitlementBadge";
import { useAnywhere } from "../../lib/anywhere/store";
import type { AnywhereAccount, BillingPlan, EntitlementState } from "../../lib/anywhere/types";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, tabularNums, type } from "../../theme/typography";

const CHECKOUT_URL = "https://forge.dev/billing/checkout";

function summaryLine(account: AnywhereAccount): string {
  switch (account.entitlement) {
    case "not-started":
      return "Trial begins when your first host connects — no card required.";
    case "trial": {
      const days = account.trialDaysLeft ?? 0;
      return `${days} day${days === 1 ? "" : "s"} left. No card on file. At expiry Anywhere goes read-only immediately; local Forge is unaffected.`;
    }
    case "active":
      return `Active${account.plan ? ` · ${account.plan === "yearly" ? "annual" : "monthly"} plan` : ""}. Cancel anytime — access continues to the paid-through date.`;
    case "grace": {
      const days = account.graceDaysLeft ?? 0;
      return `${days} day${days === 1 ? "" : "s"} left in grace — payment retry in progress. Full access continues.`;
    }
    case "read-only":
      return "Read-only — download, restore, delete, export and billing only. Checkout restores full access instantly.";
    case "suspended": {
      const days = account.deletesInDays ?? 0;
      return `${days} day${days === 1 ? "" : "s"} until this data is permanently deleted. Billing, export and deletion only.`;
    }
    case "webhook-pending":
      return "Payment received — entitlement updating, usually under a minute.";
    default: {
      const _exhaustive: never = account.entitlement;
      return _exhaustive;
    }
  }
}

interface LifecycleRow {
  key: EntitlementState;
  label: string;
  copy: string;
}

// The comp's "entitlement lifecycle" table lists 6 rows (NOT STARTED/ACTIVE/GRACE/
// READ-ONLY/SUSPENDED/WEBHOOK…) — TRIAL is added here too (not in the original comp
// list) so the seeded demo account's actual state can be highlighted; without it
// the "highlight current state" behavior below would never fire against the mock's
// default trialing account.
const LIFECYCLE: LifecycleRow[] = [
  { key: "not-started", label: "NOT STARTED", copy: "Trial begins when your first host connects" },
  { key: "trial", label: "TRIAL", copy: "14 days, starts when the first host connects · no card required" },
  { key: "active", label: "ACTIVE", copy: "Renews on the billing date · cancel keeps access to the paid-through date" },
  { key: "grace", label: "GRACE", copy: "Paid-plan payment failed — full access continues 7 days while we retry. Trial expiry skips grace." },
  { key: "read-only", label: "READ-ONLY", copy: "Immediately at trial expiry (no grace), or after paid grace ends. 30 days: download, restore, delete, export, billing. New relay work, uploads, shares and capsules are blocked. Checkout restores full access instantly." },
  { key: "suspended", label: "SUSPENDED", copy: "Billing, export and deletion only — until the 90-day retention deadline" },
  { key: "webhook-pending", label: "WEBHOOK…", copy: "Payment received — entitlement updating, usually under a minute" },
];

function lifecycleColor(key: EntitlementState, tokens: ReturnType<typeof useTokens>): string {
  switch (key) {
    case "active":
      return tokens.success;
    case "trial":
    case "grace":
      return tokens.warn;
    case "read-only":
    case "suspended":
      return tokens.danger;
    case "not-started":
    case "webhook-pending":
    default:
      return tokens.ink3;
  }
}

export default function AnywhereBillingScreen() {
  const tokens = useTokens();
  const { account, signedIn, loading } = useAnywhere();
  const [plan, setPlan] = useState<BillingPlan>("yearly");

  useEffect(() => {
    if (!loading && !signedIn) router.replace("/anywhere");
  }, [loading, signedIn]);

  if (loading || !signedIn || !account) return null;

  return (
    <Screen scroll contentContainerStyle={styles.content}>
      <View style={styles.header}>
        <BackLink label="Anywhere" onPress={() => router.replace("/anywhere")} />
        <View style={styles.headerRow}>
          <Text style={[type.headingBold, styles.headerTitle, { color: tokens.ink }]}>Billing</Text>
          <EntitlementBadge account={account} />
        </View>
      </View>
      <Text style={[type.sub, styles.summary, { color: tokens.ink2 }]}>{summaryLine(account)}</Text>

      <View style={styles.section}>
        <Text style={[type.section, { color: tokens.ink4 }]}>choose a plan</Text>
        <PlanRow
          selected={plan === "yearly"}
          onPress={() => setPlan("yearly")}
          label="Annual"
          badge="2 months free"
          price="€79"
          period="/yr"
        />
        <View style={[styles.hairline, { backgroundColor: tokens.hairline }]} />
        <PlanRow selected={plan === "monthly"} onPress={() => setPlan("monthly")} label="Monthly" price="€10" period="/mo" />

        <Button
          label="Continue in browser — Paddle checkout"
          fullWidth
          style={styles.checkoutButton}
          onPress={() => {
            // Placeholder Paddle checkout URL — the real relay backend will mint a
            // session-scoped checkout link server-side; this hits the marketing
            // domain's static entry point until that endpoint exists.
            void Linking.openURL(CHECKOUT_URL);
          }}
        />
        <Text style={[type.meta, styles.checkoutFootnote, { color: tokens.ink4 }]}>
          You&apos;ll return here when payment completes. Includes 3 hosts, unlimited devices,
          5 GB, jobs, handoff, shares.
        </Text>
      </View>

      <View style={styles.section}>
        <Text style={[type.section, { color: tokens.ink4 }]}>entitlement lifecycle</Text>
        {LIFECYCLE.map((row, i) => {
          const current = row.key === account.entitlement;
          return (
            <View key={row.key}>
              <View
                style={[
                  styles.lifecycleRow,
                  current ? { borderLeftWidth: 2, borderLeftColor: tokens.accent, paddingLeft: space.space8 } : styles.lifecycleRowInset,
                ]}
              >
                <Text
                  style={[type.monoMeta, tabularNums, styles.lifecycleLabel, { color: lifecycleColor(row.key, tokens), fontFamily: monoFamily.bold }]}
                >
                  {row.label}
                </Text>
                <Text style={[type.sub, styles.lifecycleCopy, { color: current ? tokens.ink : tokens.ink2 }]}>{row.copy}</Text>
              </View>
              {i < LIFECYCLE.length - 1 ? <View style={[styles.hairline, { backgroundColor: tokens.hairline }]} /> : null}
            </View>
          );
        })}
      </View>

      <Text style={[type.meta, styles.footer, { color: tokens.ink4 }]}>
        None of these states touch local Forge, Direct access or your own tunnels.
      </Text>
    </Screen>
  );
}

function PlanRow({ selected, onPress, label, badge, price, period }: {
  selected: boolean;
  onPress: () => void;
  label: string;
  badge?: string;
  price: string;
  period: string;
}) {
  const tokens = useTokens();
  return (
    <Pressable onPress={onPress} accessibilityRole="radio" accessibilityState={{ checked: selected }} accessibilityLabel={label} style={styles.planRow}>
      <View style={[styles.radio, { borderColor: selected ? "transparent" : tokens.borderStrong, backgroundColor: selected ? tokens.accent : "transparent" }]}>
        {selected ? <Check size={9} strokeWidth={3.5} color={tokens.onAccent} /> : null}
      </View>
      <View style={styles.planLabelRow}>
        <Text style={[type.bodyBold, { color: selected ? tokens.ink : tokens.ink2 }]}>{label}</Text>
        {badge ? <Text style={[type.meta, styles.planBadge, { color: tokens.success }]}>{badge}</Text> : null}
      </View>
      <Text style={[type.code, tabularNums, { color: selected ? tokens.ink : tokens.ink2 }]}>
        {price}
        <Text style={{ color: tokens.ink3 }}>{period}</Text>
      </Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space48 },
  header: { gap: space.space4 },
  headerRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  headerTitle: { flex: 1 },
  summary: { marginLeft: space.space4, marginTop: space.space4, lineHeight: 19 },
  section: { marginTop: space.space20 },
  planRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space12 },
  radio: { width: 17, height: 17, borderRadius: 9, borderWidth: 1.5, alignItems: "center", justifyContent: "center" },
  planLabelRow: { flex: 1, flexDirection: "row", alignItems: "center", gap: 6 },
  planBadge: { fontWeight: "700" },
  hairline: { height: StyleSheet.hairlineWidth, marginLeft: 27 },
  checkoutButton: { marginTop: space.space12 },
  checkoutFootnote: { textAlign: "center", marginTop: space.space8, lineHeight: 16 },
  lifecycleRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space8 },
  lifecycleRowInset: { paddingLeft: space.space8 + 2 },
  lifecycleLabel: { width: 78 },
  lifecycleCopy: { flex: 1, lineHeight: 17 },
  footer: { marginTop: space.space16 },
});
