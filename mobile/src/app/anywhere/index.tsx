import * as Clipboard from "expo-clipboard";
import * as DocumentPicker from "expo-document-picker";
import { File as ExpoFile } from "expo-file-system";
import { router } from "expo-router";
import {
  Check,
  ChevronRight,
  Clock3,
  Cloud,
  Copy,
  Download,
  KeyRound,
  Laptop,
  RefreshCw,
  ShieldCheck,
  X,
} from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useState } from "react";
import { ActivityIndicator, Platform, Pressable, Share, StyleSheet, Text, View } from "react-native";
import Svg, { Path } from "react-native-svg";

import { BackLink } from "../../components/ds/BackLink";
import { Banner } from "../../components/ds/Banner";
import { Button } from "../../components/ds/Button";
import { Input } from "../../components/ds/Input";
import { Screen } from "../../components/ds/Screen";
import { useToast } from "../../components/ds/ToastHost";
import { useAnywhere, type AnywherePendingApproval } from "../../lib/AnywhereProvider";
import { hostFleetSummary, hostStatusText } from "../../lib/anywhereHostPresence";
import { goBackOr } from "../../lib/nav";
import { isTauri } from "../../lib/platform";
import { MAX_ACTIVE_HOSTS } from "../../lib/anywhere/types";
import { formatBytes } from "../../lib/anywhere/format";
import { useTokens } from "../../theme/ThemeProvider";
import { hexToRgba, radii, space } from "../../theme/tokens";
import { useBreakpoint } from "../../theme/useBreakpoint";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";

const LOCAL_NOTE = "Direct and LAN connections keep working if Forge Anywhere is unavailable.";

const SEALED_ENVELOPE_NOTE =
  "who can read your data — only your paired devices. The relay routes sealed envelopes: sizes, timestamps, kinds. Never contents.";

/** `TRIAL · 9D` — only rendered while the account is actually trialing; every other
 * entitlement state (active/grace/read-only/suspended/not-started) shows no badge
 * here rather than a fabricated one — Billing already owns the full lifecycle table. */
function trialBadgeLabel(entitlement: string | undefined, trialEndsAt: string | null | undefined): string | null {
  if (entitlement !== "trialing" && entitlement !== "trial") return null;
  if (!trialEndsAt) return "TRIAL";
  const daysLeft = Math.max(0, Math.ceil((Date.parse(trialEndsAt) - Date.now()) / (24 * 60 * 60 * 1000)));
  return `TRIAL · ${daysLeft}D`;
}

function pushStatusLabel(status: string): string {
  return status === "subscribed" ? "on" : status === "denied" ? "blocked" : status === "unsupported" ? "unsupported" : "off";
}

export default function AnywhereScreen() {
  const anywhere = useAnywhere();
  const tokens = useTokens();

  return (
    <Screen scroll keyboardAvoiding contentContainerStyle={styles.screen}>
      <View style={styles.shell}>
        <BackLink label="Settings" onPress={() => goBackOr("/settings")} />
        {anywhere.phase === "ready" ? <ReadyCenter /> : <SetupFlow />}
        <Text style={[typeScale.meta, styles.localNote, { color: tokens.ink3 }]}>{LOCAL_NOTE}</Text>
      </View>
    </Screen>
  );
}

function SetupFlow() {
  const anywhere = useAnywhere();
  const tokens = useTokens();

  return (
    <View style={styles.flow}>
      <View style={styles.titleBlock}>
        <View style={[styles.brandMark, { backgroundColor: tokens.selection }]}>
          <Cloud size={22} strokeWidth={1.8} color={tokens.accent} />
        </View>
        <View style={styles.titleCopy}>
          <Text accessibilityRole="header" style={[typeScale.title, { color: tokens.ink }]}>Connect Forge Anywhere</Text>
          <Text style={[typeScale.sub, styles.measure, { color: tokens.ink2 }]}>Reach your Forge hosts securely from Desktop, web, or mobile. Your encrypted work stays unreadable to the relay.</Text>
        </View>
      </View>

      <SetupProgress phase={anywhere.phase} />
      {anywhere.error && anywhere.phase !== "error" ? <Banner tone="danger" message={anywhere.error} style={styles.flushBanner} /> : null}

      {anywhere.phase === "loading" || anywhere.phase === "starting" ? <LoadingStep /> : null}
      {anywhere.phase === "signed_out" ? <WelcomeStep /> : null}
      {anywhere.phase === "reauthentication_required" ? <ReauthenticationStep /> : null}
      {anywhere.phase === "authorizing" ? <GitHubStep /> : null}
      {anywhere.phase === "awaiting_approval" ? <ApprovalWaitingStep /> : null}
      {anywhere.phase === "new_recovery" ? <NewRecoveryStep /> : null}
      {anywhere.phase === "existing_recovery" ? <RecoveryStep /> : null}
      {anywhere.phase === "error" ? <ErrorStep /> : null}
    </View>
  );
}

function SetupProgress({ phase }: { phase: ReturnType<typeof useAnywhere>["phase"] }) {
  const tokens = useTokens();
  const active = phase === "signed_out" || phase === "reauthentication_required" || phase === "starting" || phase === "authorizing" ? 0
    : phase === "awaiting_approval" || phase === "existing_recovery" || phase === "error" ? 1
      : phase === "new_recovery" ? 2 : 3;
  const labels = ["GitHub", "Approve", "Recovery", "Connected"];
  return (
    <View style={styles.progress} accessibilityRole="progressbar" accessibilityValue={{ min: 1, max: 4, now: active + 1 }}>
      {labels.map((label, index) => (
        <View key={label} style={styles.progressItem}>
          <View style={[styles.progressDot, {
            backgroundColor: index < active ? tokens.success : index === active ? tokens.accent : tokens.bg3,
            borderColor: index <= active ? (index < active ? tokens.success : tokens.accent) : tokens.borderStrong,
          }]}>
            {index < active ? <Check size={12} strokeWidth={2.5} color={tokens.successBg} /> : <Text style={[typeScale.monoMeta, { color: index === active ? tokens.onAccent : tokens.ink3 }]}>{index + 1}</Text>}
          </View>
          <Text style={[typeScale.meta, { color: index === active ? tokens.ink : tokens.ink3 }]}>{label}</Text>
        </View>
      ))}
    </View>
  );
}

function ReauthenticationStep() {
  const anywhere = useAnywhere();
  const tokens = useTokens();
  return (
    <View style={styles.step}>
      <View style={[styles.safetyPanel, { borderColor: tokens.warn, backgroundColor: tokens.warnBg }]}>
        <View style={styles.safetyHeader}><KeyRound size={18} color={tokens.warn} /><Text style={[typeScale.bodyBold, { color: tokens.ink }]}>This browser&apos;s secure session expired</Text></View>
        <Text accessibilityRole="alert" style={[typeScale.body, styles.measure, { color: tokens.ink2 }]}>Reconnect with GitHub, then approve this browser from an enrolled device. Cached hosts and devices are hidden until the connection is verified.</Text>
      </View>
      <Button label="Reconnect with GitHub" icon={<GithubMark size={18} color={tokens.onAccent} />} onPress={() => void anywhere.startLogin()} fullWidth />
      <Text style={[typeScale.meta, styles.measure, { color: tokens.ink3 }]}>Your encrypted account keys remain protected on this browser. Direct and LAN connections are unchanged.</Text>
    </View>
  );
}

function LoadingStep() {
  const tokens = useTokens();
  return (
    <View style={styles.centerStep} accessibilityRole="alert">
      <ActivityIndicator color={tokens.accent} />
      <Text style={[typeScale.body, { color: tokens.ink2 }]}>Preparing secure sign-in…</Text>
    </View>
  );
}

function WelcomeStep() {
  const anywhere = useAnywhere();
  const tokens = useTokens();
  return (
    <View style={styles.step}>
      <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Start with GitHub</Text>
      <Text style={[typeScale.body, styles.measure, { color: tokens.ink2 }]}>GitHub identifies your Forge account. It never unlocks your encrypted sessions or replaces your Recovery Kit.</Text>
      <Button label="Continue with GitHub" icon={<GithubMark size={18} color={tokens.onAccent} />} onPress={() => void anywhere.startLogin()} fullWidth style={styles.primaryAction} />
      <View style={styles.promiseList}>
        <PromiseLine icon={<ShieldCheck size={17} color={tokens.success} />} text="Returning devices are approved with a short safety code." />
        <PromiseLine icon={<KeyRound size={17} color={tokens.info} />} text="Recovery words are only used when your enrolled devices are unavailable." />
      </View>
    </View>
  );
}

function GitHubStep() {
  const anywhere = useAnywhere();
  const tokens = useTokens();
  return (
    <View style={styles.step}>
      <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Finish signing in</Text>
      <Text style={[typeScale.body, styles.measure, { color: tokens.ink2 }]}>A trusted browser window is open. Enter this one-time code if GitHub asks for it.</Text>
      <View style={[styles.codeBox, { backgroundColor: tokens.bg2, borderColor: tokens.borderStrong }]}>
        <Text selectable accessibilityLabel={`GitHub code ${anywhere.flow?.user_code ?? ""}`} style={[styles.deviceCode, { color: tokens.ink }]}>{anywhere.flow?.user_code ?? "••••-••••"}</Text>
      </View>
      <Button label="Open GitHub" variant="secondary" icon={<GithubMark size={18} color={tokens.ink} />} onPress={() => void anywhere.openLoginPage()} fullWidth />
      <View style={styles.waitingRow}><ActivityIndicator size="small" color={tokens.accent} /><Text style={[typeScale.sub, { color: tokens.ink3 }]}>Waiting for GitHub…</Text></View>
    </View>
  );
}

function ApprovalWaitingStep() {
  const anywhere = useAnywhere();
  const tokens = useTokens();
  const remaining = useCountdown(anywhere.claimantApproval?.expiresAtMs ?? 0);
  const approval = anywhere.claimantApproval;
  if (!approval) return <LoadingStep />;
  return (
    <View style={styles.step}>
      <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Approve this device</Text>
      <Text style={[typeScale.body, styles.measure, { color: tokens.ink2 }]}>Open Forge on an enrolled device. This request appears automatically in its Approval inbox.</Text>
      <View style={[styles.safetyPanel, { backgroundColor: tokens.bg2, borderColor: tokens.borderStrong }]}>
        <View style={styles.safetyHeader}><ShieldCheck size={18} color={tokens.info} /><Text style={[typeScale.bodyBold, { color: tokens.ink }]}>Compare safety codes</Text></View>
        <Text selectable style={[styles.safetyCode, tabularNums, { color: tokens.ink }]}>{approval.safetyCode}</Text>
        <Text style={[typeScale.sub, { color: tokens.ink3 }]}>The same code must appear on both devices before approval.</Text>
      </View>
      <View style={styles.detailRows}>
        <DetailRow label="Device" value={approval.deviceName} />
        <DetailRow label="Account" value={`@${approval.githubLogin}`} />
        <DetailRow label="Expires" value={remaining} icon={<Clock3 size={14} color={tokens.ink3} />} />
      </View>
      <View style={styles.waitingRow}><ActivityIndicator size="small" color={tokens.accent} /><Text style={[typeScale.sub, { color: tokens.ink3 }]}>Waiting for approval…</Text></View>
      <Button label="Use Recovery Kit instead" variant="ghost" icon={<KeyRound size={17} color={tokens.ink2} />} onPress={anywhere.useRecoveryInstead} fullWidth />
    </View>
  );
}

function NewRecoveryStep() {
  const anywhere = useAnywhere();
  const tokens = useTokens();
  const toast = useToast();
  const words = useMemo(() => anywhere.recoveryWords?.split(" ") ?? [], [anywhere.recoveryWords]);
  const [answers, setAnswers] = useState<Record<number, string>>({});
  const [saving, setSaving] = useState(false);
  // Masked-after-word-2 per the comp's "shown once" grid — purely a visual
  // over-the-shoulder guard; Copy/Share already carry every word regardless.
  const [revealed, setRevealed] = useState(false);

  const saveKit = useCallback(async () => {
    if (!anywhere.recoveryKit) return;
    setSaving(true);
    try {
      if (Platform.OS === "web") {
        const url = URL.createObjectURL(new Blob([anywhere.recoveryKit], { type: "application/json" }));
        const anchor = document.createElement("a");
        anchor.href = url;
        anchor.download = "forge-recovery-kit.forge-recovery";
        anchor.click();
        URL.revokeObjectURL(url);
      } else {
        await Share.share({ title: "Forge Recovery Kit", message: anywhere.recoveryKit });
      }
      toast.show("Recovery Kit export opened. Store it somewhere offline.", { tone: "neutral" });
    } finally { setSaving(false); }
  }, [anywhere.recoveryKit, toast]);

  const copyWords = useCallback(async () => {
    if (!anywhere.recoveryWords) return;
    await Clipboard.setStringAsync(anywhere.recoveryWords);
    toast.show("12 recovery words copied by your explicit action.", { tone: "neutral" });
  }, [anywhere.recoveryWords, toast]);

  return (
    <View style={styles.step}>
      <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Save your Recovery Kit</Text>
      <Text style={[typeScale.body, styles.measure, { color: tokens.ink2 }]}>This is the fallback if every enrolled device is lost. Save the file or write down the 12 words, then keep it offline.</Text>
      <View style={[styles.wordGrid, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}>
        {words.map((word, index) => <View key={`${index}-${word}`} style={styles.word}><Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>{index + 1}</Text><Text selectable style={[typeScale.codeSmall, { color: tokens.ink }]}>{revealed || index < 2 ? word : "•••••"}</Text></View>)}
      </View>
      <Pressable onPress={() => setRevealed((prev) => !prev)} accessibilityRole="button" accessibilityLabel={revealed ? "Hide words" : "Reveal words"}>
        <Text style={[typeScale.meta, { color: tokens.accent }]}>{revealed ? "Hide words" : "Reveal all words"}</Text>
      </Pressable>
      <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>no cloud backup · no screenshots · held on paper, not in this app</Text>
      <View style={styles.actionRow}>
        <Button label={Platform.OS === "web" ? "Save Recovery Kit" : "Share Recovery Kit"} variant="secondary" loading={saving} icon={<Download size={17} color={tokens.ink} />} onPress={() => void saveKit()} style={styles.flexAction} />
        <Button label="Copy words" variant="ghost" icon={<Copy size={17} color={tokens.ink2} />} onPress={() => void copyWords()} style={styles.flexAction} />
      </View>
      <View style={[styles.verifyBlock, { borderTopColor: tokens.border }]}>
        <Text style={[typeScale.bodyBold, { color: tokens.ink }]}>Verify your backup</Text>
        <Text style={[typeScale.sub, { color: tokens.ink3 }]}>Enter three words from the copy you saved. Your first device activates only after this check.</Text>
        <View style={styles.answerGrid}>
          {anywhere.recoverySample.map((index) => <Input key={index} label={`Word ${index + 1}`} value={answers[index] ?? ""} autoCapitalize="none" autoCorrect={false} onChangeText={(value) => setAnswers((current) => ({ ...current, [index]: value }))} containerStyle={styles.answer} />)}
        </View>
        <Button label="Verify Recovery Kit" onPress={() => void anywhere.confirmNewRecovery(answers)} fullWidth disabled={anywhere.recoverySample.some((index) => !answers[index]?.trim())} />
      </View>
    </View>
  );
}

function RecoveryStep() {
  const anywhere = useAnywhere();
  const tokens = useTokens();
  const toast = useToast();
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [showReset, setShowReset] = useState(false);
  const [resetConfirmation, setResetConfirmation] = useState("");
  const [resetAt, setResetAt] = useState<number | null>(null);

  const importKit = useCallback(async () => {
    const result = await DocumentPicker.getDocumentAsync({ type: ["application/json", "application/octet-stream", "text/plain"], multiple: false, copyToCacheDirectory: true });
    if (result.canceled) return;
    try {
      const contents = await new ExpoFile(result.assets[0].uri).text();
      setValue(contents);
      toast.show("Recovery Kit loaded. Review the account, then recover this device.", { tone: "neutral" });
    } catch {
      toast.show("That Recovery Kit could not be read.", { tone: "danger" });
    }
  }, [toast]);

  const recover = useCallback(async () => {
    if (!value.trim()) return;
    setBusy(true);
    try { await anywhere.recoverExisting(value); } finally { setBusy(false); }
  }, [anywhere, value]);

  const scheduleReset = useCallback(async () => {
    setBusy(true);
    try {
      const executeAt = await anywhere.scheduleCleanReset(resetConfirmation);
      setResetAt(executeAt);
      toast.show("Clean reset scheduled. Any enrolled device can cancel it for seven days.", { tone: "neutral" });
    } catch (reason) {
      toast.show(reason instanceof Error ? reason.message : "Clean reset could not be scheduled.", { tone: "danger" });
    } finally { setBusy(false); }
  }, [anywhere, resetConfirmation, toast]);

  return (
    <View style={styles.step}>
      <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Recover this device</Text>
      <Text style={[typeScale.body, styles.measure, { color: tokens.ink2 }]}>Choose the easiest recovery method available. Recovery data stays encrypted and GitHub sign-in alone cannot unlock it.</Text>
      {anywhere.passkeys.length ? <View style={[styles.safetyPanel, { borderColor: tokens.borderStrong, backgroundColor: tokens.bg2 }]}>
        <View style={styles.safetyHeader}><KeyRound size={18} color={tokens.accent} /><Text style={[typeScale.bodyBold, { color: tokens.ink }]}>Recover with a passkey</Text></View>
        {anywhere.passkeys.map((passkey) => <Button key={passkey.id} label={passkey.name} variant="secondary" loading={busy} onPress={() => { setBusy(true); void anywhere.recoverWithPasskey(passkey.id).catch((reason) => toast.show(reason instanceof Error ? reason.message : "Passkey recovery failed.", { tone: "danger" })).finally(() => setBusy(false)); }} fullWidth />)}
      </View> : null}
      <Text style={[typeScale.sub, { color: tokens.ink3 }]}>Recovery Kit file, QR, or words</Text>
      <Button label="Choose Recovery Kit file" variant="secondary" icon={<Download size={17} color={tokens.ink} />} onPress={() => void importKit()} fullWidth />
      <View style={styles.orRow}><View style={[styles.rule, { backgroundColor: tokens.border }]} /><Text style={[typeScale.meta, { color: tokens.ink3 }]}>or enter words</Text><View style={[styles.rule, { backgroundColor: tokens.border }]} /></View>
      <Input label="Recovery words" value={value.startsWith("{") ? "Recovery Kit file loaded" : value} onChangeText={setValue} multiline autoCapitalize="none" autoCorrect={false} clearable containerStyle={styles.phraseInput} accessibilityHint="Enter 12 or 24 recovery words" />
      <Button label="Recover this device" onPress={() => void recover()} loading={busy} disabled={!value.trim()} fullWidth />
      <Button label="Try device approval again" variant="ghost" icon={<RefreshCw size={17} color={tokens.ink2} />} onPress={anywhere.restartSetup} fullWidth />
      {!showReset ? <Button label="I lost every recovery method" variant="ghost" onPress={() => setShowReset(true)} fullWidth /> : null}
      {showReset ? <View style={[styles.resetPanel, { borderColor: tokens.danger, backgroundColor: tokens.dangerBg }]}>
        <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Schedule a clean reset</Text>
        <Text style={[typeScale.sub, { color: tokens.ink2 }]}>This waits seven days, then permanently deletes hosted encrypted account data. It never deletes local Forge data. Enrolled devices are notified and can cancel.</Text>
        {resetAt ? <Banner tone="warn" message={`Reset scheduled for ${new Date(resetAt).toLocaleString()}.`} style={styles.flushBanner} /> : <>
          <Input label="Type DELETE MY FORGE ANYWHERE DATA" value={resetConfirmation} onChangeText={setResetConfirmation} autoCapitalize="characters" autoCorrect={false} accessibilityHint="Exact destructive confirmation phrase" />
          <View style={styles.actionRow}><Button label="Keep trying recovery" variant="ghost" disabled={busy} onPress={() => { setShowReset(false); setResetConfirmation(""); }} style={styles.flexAction} /><Button label="Schedule seven-day reset" variant="danger" loading={busy} disabled={resetConfirmation !== "DELETE MY FORGE ANYWHERE DATA"} onPress={() => void scheduleReset()} style={styles.flexAction} /></View>
        </>}
      </View> : null}
    </View>
  );
}

function ErrorStep() {
  const anywhere = useAnywhere();
  const tokens = useTokens();
  return (
    <View style={styles.step}>
      <View style={[styles.errorIcon, { backgroundColor: tokens.dangerBg }]}><X size={22} color={tokens.danger} /></View>
      <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Setup needs your attention</Text>
      <Text accessibilityRole="alert" style={[typeScale.body, styles.measure, { color: tokens.danger }]}>{anywhere.error ?? "Forge Anywhere could not continue."}</Text>
      {anywhere.claimantApproval ? <Button label="Use Recovery Kit" variant="secondary" icon={<KeyRound size={17} color={tokens.ink} />} onPress={anywhere.useRecoveryInstead} fullWidth /> : null}
      <Button label="Start again" icon={<RefreshCw size={17} color={tokens.onAccent} />} onPress={anywhere.restartSetup} fullWidth />
    </View>
  );
}

function ReadyCenter() {
  const anywhere = useAnywhere();
  const tokens = useTokens();
  const toast = useToast();
  const [actingId, setActingId] = useState<string | null>(null);

  const decide = useCallback(async (request: AnywherePendingApproval, approve: boolean) => {
    setActingId(request.id);
    try {
      if (approve) {
        await anywhere.approvePendingDevice(request.id);
        toast.show(`${request.deviceName} approved.`, { tone: "neutral" });
      } else {
        await anywhere.denyPendingDevice(request.id);
        toast.show(`${request.deviceName} denied. No keys were shared.`, { tone: "neutral" });
      }
    } catch (reason) {
      toast.show(reason instanceof Error ? reason.message : "Approval could not be completed.", { tone: "danger" });
    } finally { setActingId(null); }
  }, [anywhere, toast]);

  const trialBadge = trialBadgeLabel(anywhere.account?.entitlement, anywhere.account?.trial_ends_at ?? null);
  const login = anywhere.credentials?.githubLogin;

  return (
    <View style={styles.ready}>
      <View style={styles.readyHeader}>
        <View style={styles.readyHeaderCopy}>
          <View style={styles.titleRow}>
            <Text accessibilityRole="header" style={[typeScale.title, { color: tokens.ink }]}>Forge Anywhere</Text>
            {trialBadge ? (
              <View style={[styles.trialBadge, { borderColor: hexToRgba(tokens.warn, 0.35) }]}>
                <Text style={[typeScale.monoMeta, styles.trialBadgeText, { color: tokens.warn }]}>{trialBadge}</Text>
              </View>
            ) : null}
          </View>
          <Text style={[typeScale.monoMeta, styles.syncCaption, { color: tokens.ink3 }]}>
            {`${login ? `@${login}` : "signed in"} · relay connected`}
          </Text>
        </View>
        <Button label="Refresh" variant="ghost" icon={<RefreshCw size={16} color={tokens.ink2} />} onPress={() => void Promise.all([anywhere.refresh(), anywhere.refreshPendingApprovals(true)])} />
      </View>

      {anywhere.account?.pending_reset ? <Banner
        tone="danger"
        message={`A clean reset is scheduled for ${new Date(anywhere.account.pending_reset.executes_at_ms).toLocaleString()}. Hosted encrypted data will be deleted unless an enrolled device cancels it.`}
        actionLabel="Cancel reset"
        onAction={() => void anywhere.cancelCleanReset().catch((reason) => toast.show(reason instanceof Error ? reason.message : "Reset could not be canceled.", { tone: "danger" }))}
        style={styles.flushBanner}
      /> : null}
      {!anywhere.passkeys.length ? <Banner tone="neutral" message="Add a recovery passkey before connecting your first host for the quickest zero-knowledge recovery." actionLabel="Open Recovery Center" onAction={() => router.push("/anywhere/recovery-phrase")} style={styles.flushBanner} /> : null}

      <Section title="Approval inbox" meta={anywhere.pendingApprovals.length ? `${anywhere.pendingApprovals.length} pending` : "No pending requests"}>
        {anywhere.approvalError ? <Banner tone="warn" message={anywhere.approvalError} actionLabel="Retry" onAction={() => void anywhere.refreshPendingApprovals(true)} style={styles.flushBanner} /> : null}
        {anywhere.pendingApprovals.length ? anywhere.pendingApprovals.map((request) => <ApprovalRequest key={request.id} request={request} busy={actingId === request.id} onApprove={() => void decide(request, true)} onDeny={() => void decide(request, false)} />) : !anywhere.approvalError ? <EmptyLine icon={<ShieldCheck size={18} color={tokens.ink3} />} text="New device requests appear here automatically." /> : null}
      </Section>

      <HubOverview anywhere={anywhere} trialBadge={trialBadge} login={login} />

      <Section title="Recovery" meta="Protected">
        <Pressable onPress={() => router.push("/anywhere/recovery-phrase")} accessibilityRole="button" style={({ pressed }) => [styles.resourceRow, { borderBottomColor: tokens.hairline, opacity: pressed ? 0.7 : 1 }]}><KeyRound size={18} color={tokens.success} /><View style={styles.resourceCopy}><Text style={[typeScale.bodyBold, { color: tokens.ink }]}>Recovery Center</Text><Text style={[typeScale.meta, { color: tokens.ink3 }]}>Recovery Kit, passkeys, and device access</Text></View></Pressable>
      </Section>
    </View>
  );
}

/** M AW Hub (nav rows to sub-screens) on compact/medium; D AW Settings (combined
 * inline pane — hosts list + two-column devices/storage-billing-account) once the
 * window is wide enough for it, per the desktop comp. */
function HubOverview({ anywhere, trialBadge, login }: { anywhere: ReturnType<typeof useAnywhere>; trialBadge: string | null; login: string | undefined }) {
  const tokens = useTokens();
  const { isExpanded } = useBreakpoint();

  if (!isExpanded) {
    return (
      <>
        {!anywhere.hosts.length ? (
          <Section title="Hosts" meta={hostFleetSummary(anywhere.hosts)}>
            <HostEmpty />
          </Section>
        ) : null}
        <View style={[styles.hubCard, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}>
          <HubRow
            label="Hosts"
            value={`${anywhere.hosts.length} of ${MAX_ACTIVE_HOSTS} · ${hostFleetSummary(anywhere.hosts)}`}
            onPress={() => router.push("/anywhere/hosts")}
          />
          <HubRow label="Devices" value={`${anywhere.devices.length} paired`} onPress={() => router.push("/anywhere/devices")} />
          <HubRow label="Remote jobs" value={`${anywhere.remoteJobs.length} queued`} onPress={() => router.push("/anywhere/jobs")} />
          <HubRow label="Notifications" value={pushStatusLabel(anywhere.pushStatus)} onPress={() => router.push("/anywhere/notifications")} />
          <HubRow
            label="Encrypted storage"
            value={anywhere.account ? `${formatBytes(anywhere.account.storage_used_bytes)} of ${formatBytes(anywhere.account.storage_limit_bytes)}` : "—"}
            onPress={() => router.push("/anywhere/storage")}
          />
          <HubRow
            label="Billing"
            value={trialBadge ?? anywhere.account?.entitlement ?? "—"}
            valueColor={trialBadge ? tokens.warn : undefined}
            onPress={() => router.push("/anywhere/billing")}
          />
          <HubRow label="Account" value={login ? `@${login} · github` : "github"} onPress={() => router.push("/anywhere/account")} last />
        </View>
        <Text style={[typeScale.monoMeta, styles.sealedNote, { color: tokens.ink3 }]}>{SEALED_ENVELOPE_NOTE}</Text>
      </>
    );
  }

  const pct = anywhere.account && anywhere.account.storage_limit_bytes > 0
    ? Math.round((anywhere.account.storage_used_bytes / anywhere.account.storage_limit_bytes) * 100)
    : 0;

  return (
    <View style={styles.desktopPane}>
      <Text style={[typeScale.section, { color: tokens.ink3 }]}>{`HOSTS · ${anywhere.hosts.length} OF ${MAX_ACTIVE_HOSTS}`}</Text>
      <View style={styles.desktopHostList}>
        {anywhere.hosts.map((host) => (
          <Pressable
            key={host.id}
            onPress={() => router.push({ pathname: "/anywhere/host/[id]", params: { id: host.id } })}
            accessibilityRole="button"
            style={({ pressed }) => [styles.desktopRow, { borderColor: tokens.border, backgroundColor: tokens.bg2, opacity: pressed ? 0.7 : 1 }]}
          >
            <View style={[styles.desktopDot, { backgroundColor: host.online === true ? tokens.success : tokens.ink4 }]} />
            <Text style={[typeScale.bodyBold, styles.desktopRowLabel, { color: tokens.ink }]} numberOfLines={1}>{host.name}</Text>
            <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]} numberOfLines={1}>{hostStatusText(host)}</Text>
            <Text style={[typeScale.meta, { color: tokens.ink2 }]}>Details</Text>
          </Pressable>
        ))}
        {!anywhere.hosts.length ? <HostEmpty /> : null}
      </View>

      <View style={styles.desktopColumns}>
        <View style={styles.desktopColumn}>
          <Text style={[typeScale.section, { color: tokens.ink3 }]}>{`DEVICES · ${anywhere.devices.length}`}</Text>
          {anywhere.devices.map((device) => (
            <View key={device.id} style={[styles.desktopRow, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}>
              <Text style={[typeScale.sub, styles.desktopRowLabel, { color: tokens.ink }]} numberOfLines={1}>{device.name}</Text>
              <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]} numberOfLines={1}>
                {device.last_seen_at ? new Date(device.last_seen_at).toLocaleDateString() : "enrolled"}
              </Text>
            </View>
          ))}
          <View style={styles.desktopActionRow}>
            <Button label="Pair device" variant="secondary" onPress={() => router.push("/anywhere/pair")} />
            <Button label="Devices…" variant="ghost" onPress={() => router.push("/anywhere/devices")} />
          </View>
        </View>

        <View style={styles.desktopColumn}>
          <Text style={[typeScale.section, { color: tokens.ink3 }]}>STORAGE · BILLING · ACCOUNT</Text>
          <Pressable onPress={() => router.push("/anywhere/storage")} style={[styles.desktopCard, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}>
            <View style={styles.desktopCardRow}>
              <Text style={[typeScale.sub, { color: tokens.ink2 }]}>Encrypted storage</Text>
              <Text style={[typeScale.monoMeta, { color: tokens.ink }]}>
                {anywhere.account ? `${formatBytes(anywhere.account.storage_used_bytes)} / ${formatBytes(anywhere.account.storage_limit_bytes)}` : "—"}
              </Text>
            </View>
            <View style={[styles.desktopGaugeTrack, { backgroundColor: tokens.border }]}>
              <View style={[styles.desktopGaugeFill, { width: `${Math.min(100, Math.max(0, pct))}%`, backgroundColor: tokens.ink2 }]} />
            </View>
          </Pressable>
          <Pressable onPress={() => router.push("/anywhere/billing")} style={[styles.desktopCard, trialBadge ? { borderColor: hexToRgba(tokens.warn, 0.3) } : { borderColor: tokens.border }, { backgroundColor: tokens.bg2 }]}>
            <View style={styles.desktopCardRow}>
              <Text style={[typeScale.bodyBold, { color: tokens.ink }]}>{`Billing — ${trialBadge ?? anywhere.account?.entitlement ?? "—"}`}</Text>
            </View>
          </Pressable>
          <View style={[styles.desktopCard, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}>
            <Text style={[typeScale.sub, { color: tokens.ink2 }]}>{login ? `@${login} · GitHub` : "GitHub"}</Text>
            <View style={styles.desktopActionRow}>
              <Pressable onPress={() => router.push("/anywhere/account")}><Text style={[typeScale.meta, { color: tokens.ink2 }]}>Account…</Text></Pressable>
              <Pressable onPress={() => router.push("/anywhere/account")}><Text style={[typeScale.meta, { color: tokens.danger }]}>Delete…</Text></Pressable>
            </View>
          </View>
        </View>
      </View>
      <Text style={[typeScale.monoMeta, styles.sealedNote, { color: tokens.ink3 }]}>{SEALED_ENVELOPE_NOTE}</Text>
    </View>
  );
}

function HubRow({ label, value, valueColor, onPress, last }: { label: string; value: string; valueColor?: string; onPress: () => void; last?: boolean }) {
  const tokens = useTokens();
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={label}
      style={({ pressed }) => [
        styles.hubRow,
        !last ? { borderBottomColor: tokens.hairline, borderBottomWidth: StyleSheet.hairlineWidth } : null,
        pressed ? { opacity: 0.7 } : null,
      ]}
    >
      <Text style={[typeScale.body, styles.hubRowLabel, { color: tokens.ink }]}>{label}</Text>
      <Text style={[typeScale.monoMeta, { color: valueColor ?? tokens.ink3 }]} numberOfLines={1}>{value}</Text>
      <ChevronRight size={14} strokeWidth={1.75} color={tokens.ink4} />
    </Pressable>
  );
}

function ApprovalRequest({ request, busy, onApprove, onDeny }: { request: AnywherePendingApproval; busy: boolean; onApprove(): void; onDeny(): void }) {
  const tokens = useTokens();
  const remaining = useCountdown(request.expiresAtMs);
  return (
    <View style={[styles.approval, { borderColor: tokens.borderStrong, backgroundColor: tokens.bg2 }]}>
      <View style={styles.approvalTop}><View style={[styles.pendingDot, { backgroundColor: tokens.warn }]} /><View style={styles.resourceCopy}><Text style={[typeScale.bodyBold, { color: tokens.ink }]}>{request.deviceName}</Text><Text style={[typeScale.meta, { color: tokens.ink3 }]}>New device · expires {remaining}</Text></View></View>
      <Text style={[typeScale.sub, { color: tokens.ink2 }]}>Compare this safety code with the new device:</Text>
      <Text selectable style={[styles.inboxCode, tabularNums, { color: tokens.ink }]}>{request.safetyCode}</Text>
      <View style={styles.actionRow}><Button label="Approve device" variant="allow" loading={busy} onPress={onApprove} style={styles.flexAction} /><Button label="Deny" variant="danger" disabled={busy} onPress={onDeny} style={styles.flexAction} /></View>
    </View>
  );
}

function HostEmpty() {
  const tokens = useTokens();
  if (isTauri) return <View style={styles.hostEmpty}><EmptyLine icon={<Laptop size={18} color={tokens.ink3} />} text="This computer can be your first host." /><Button label="Activate this computer" onPress={() => router.push("/anywhere/first-host")} fullWidth /></View>;
  if (Platform.OS === "web") return <View style={styles.hostEmpty}><EmptyLine icon={<Laptop size={18} color={tokens.ink3} />} text="A host runs in Forge Desktop while this browser stays your controller." /><Button label="Get Forge Desktop" variant="secondary" onPress={() => void openDesktop()} fullWidth /></View>;
  return <EmptyLine icon={<Laptop size={18} color={tokens.ink3} />} text="Use Forge Desktop on a computer to activate your first host. Mobile remains a secure controller." />;
}

function Section({ title, meta, children }: { title: string; meta: string; children: React.ReactNode }) {
  const tokens = useTokens();
  return <View style={styles.section}><View style={styles.sectionHeader}><Text style={[typeScale.headingBold, { color: tokens.ink }]}>{title}</Text><Text style={[typeScale.meta, { color: tokens.ink3 }]}>{meta}</Text></View><View style={[styles.sectionBody, { borderTopColor: tokens.border }]}>{children}</View></View>;
}

function PromiseLine({ icon, text }: { icon: React.ReactNode; text: string }) { const tokens = useTokens(); return <View style={styles.promise}>{icon}<Text style={[typeScale.sub, styles.promiseText, { color: tokens.ink2 }]}>{text}</Text></View>; }
function DetailRow({ label, value, icon }: { label: string; value: string; icon?: React.ReactNode }) { const tokens = useTokens(); return <View style={styles.detailRow}><Text style={[typeScale.meta, { color: tokens.ink3 }]}>{label}</Text><View style={styles.valueWithIcon}>{icon}<Text style={[typeScale.body, { color: tokens.ink }]}>{value}</Text></View></View>; }
function EmptyLine({ icon, text }: { icon: React.ReactNode; text: string }) { const tokens = useTokens(); return <View style={styles.emptyLine}>{icon}<Text style={[typeScale.sub, styles.promiseText, { color: tokens.ink3 }]}>{text}</Text></View>; }

function useCountdown(expiresAtMs: number): string {
  const [now, setNow] = useState(0);
  useEffect(() => {
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1_000);
    return () => clearInterval(timer);
  }, []);
  const seconds = Math.max(0, Math.ceil((expiresAtMs - now) / 1_000));
  return `${Math.floor(seconds / 60)}:${(seconds % 60).toString().padStart(2, "0")}`;
}

async function openDesktop(): Promise<void> {
  if (Platform.OS !== "web") return;
  window.open("https://github.com/adulari/forge/releases/latest", "_blank", "noopener,noreferrer");
}

function GithubMark({ size, color }: { size: number; color: string }) {
  return <Svg width={size} height={size} viewBox="0 0 24 24" fill={color}><Path d="M12 2C6.48 2 2 6.58 2 12.25c0 4.53 2.87 8.37 6.84 9.73.5.09.68-.22.68-.49v-1.7c-2.78.62-3.37-1.37-3.37-1.37-.45-1.18-1.11-1.5-1.11-1.5-.9-.63.07-.62.07-.62 1 .07 1.53 1.05 1.53 1.05.89 1.56 2.34 1.11 2.91.85.09-.66.35-1.11.63-1.37-2.22-.26-4.56-1.14-4.56-5.07 0-1.12.39-2.03 1.03-2.75-.1-.26-.45-1.3.1-2.7 0 0 .84-.28 2.75 1.05a9.4 9.4 0 0 1 5 0c1.91-1.33 2.75-1.05 2.75-1.05.55 1.4.2 2.44.1 2.7.64.72 1.03 1.63 1.03 2.75 0 3.94-2.34 4.8-4.57 5.06.36.32.68.94.68 1.9v2.82c0 .27.18.59.69.49A10.06 10.06 0 0 0 22 12.25C22 6.58 17.52 2 12 2z" /></Svg>;
}

const styles = StyleSheet.create({
  screen: { paddingTop: space.space12, paddingBottom: space.space48 },
  shell: { width: "100%", maxWidth: 760, alignSelf: "center" },
  flow: { marginTop: space.space12 },
  titleBlock: { flexDirection: "row", gap: space.space12, alignItems: "flex-start" },
  brandMark: { width: 44, height: 44, borderRadius: radii.radius12, alignItems: "center", justifyContent: "center" },
  titleCopy: { flex: 1, gap: space.space4 },
  measure: { maxWidth: 620 },
  progress: { flexDirection: "row", marginTop: space.space24, marginBottom: space.space20, justifyContent: "space-between" },
  progressItem: { flex: 1, alignItems: "center", gap: space.space4 },
  progressDot: { width: 26, height: 26, borderRadius: 13, borderWidth: 1, alignItems: "center", justifyContent: "center" },
  step: { gap: space.space12 },
  centerStep: { minHeight: 180, alignItems: "center", justifyContent: "center", gap: space.space12 },
  primaryAction: { marginTop: space.space4 },
  promiseList: { gap: space.space8, marginTop: space.space4 },
  promise: { flexDirection: "row", gap: space.space8, alignItems: "flex-start" },
  promiseText: { flex: 1 },
  codeBox: { minHeight: 76, borderWidth: 1, borderRadius: radii.radius12, alignItems: "center", justifyContent: "center" },
  deviceCode: { fontFamily: monoFamily.bold, fontSize: 25, lineHeight: 32, letterSpacing: 2.2 },
  waitingRow: { flexDirection: "row", alignItems: "center", justifyContent: "center", gap: space.space8, marginTop: space.space4 },
  safetyPanel: { borderWidth: 1, borderRadius: radii.radius12, padding: space.space16, gap: space.space8 },
  safetyHeader: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  safetyCode: { fontFamily: monoFamily.bold, fontSize: 28, lineHeight: 36, letterSpacing: 2.4 },
  detailRows: { gap: space.space8 },
  detailRow: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: space.space12 },
  valueWithIcon: { flexDirection: "row", alignItems: "center", gap: space.space4 },
  wordGrid: { flexDirection: "row", flexWrap: "wrap", borderWidth: 1, borderRadius: radii.radius12, padding: space.space12, gap: space.space8 },
  word: { width: "30%", minWidth: 130, flexGrow: 1, flexDirection: "row", gap: space.space8, alignItems: "center" },
  actionRow: { flexDirection: "row", flexWrap: "wrap", gap: space.space8 },
  flexAction: { flexGrow: 1, flexBasis: 180 },
  verifyBlock: { borderTopWidth: 1, paddingTop: space.space16, marginTop: space.space4, gap: space.space12 },
  answerGrid: { flexDirection: "row", flexWrap: "wrap", gap: space.space8 },
  answer: { flexGrow: 1, flexBasis: 160 },
  orRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  rule: { height: StyleSheet.hairlineWidth, flex: 1 },
  phraseInput: { minHeight: 72 },
  resetPanel: { borderWidth: 1, borderRadius: radii.radius12, padding: space.space16, gap: space.space12, marginTop: space.space8 },
  errorIcon: { width: 44, height: 44, borderRadius: 22, alignItems: "center", justifyContent: "center" },
  ready: { marginTop: space.space12, gap: space.space24 },
  readyHeader: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: space.space12 },
  readyHeaderCopy: { flex: 1, gap: space.space4 },
  titleRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  trialBadge: { borderWidth: 1, borderRadius: radii.radius4, paddingHorizontal: space.space8, paddingVertical: 2 },
  trialBadgeText: { fontWeight: "600" },
  syncCaption: {},
  hubCard: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius8, overflow: "hidden" },
  hubRow: { minHeight: 50, flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space12 },
  hubRowLabel: { flex: 1 },
  sealedNote: { lineHeight: 17, fontFamily: monoFamily.regular },
  desktopPane: { gap: space.space12 },
  desktopHostList: { gap: space.space4 },
  desktopColumns: { flexDirection: "row", gap: space.space20 },
  desktopColumn: { flex: 1, gap: space.space4 },
  desktopRow: { minHeight: 40, flexDirection: "row", alignItems: "center", gap: space.space8, borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4, paddingHorizontal: space.space12 },
  desktopRowLabel: { flex: 1 },
  desktopDot: { width: 5, height: 5, borderRadius: 3 },
  desktopActionRow: { flexDirection: "row", gap: space.space8, marginTop: space.space4 },
  desktopCard: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4, padding: space.space12, gap: space.space4 },
  desktopCardRow: { flexDirection: "row", justifyContent: "space-between", alignItems: "center", gap: space.space8 },
  desktopGaugeTrack: { height: 3, borderRadius: 2, overflow: "hidden" },
  desktopGaugeFill: { height: "100%", borderRadius: 2 },
  section: { gap: space.space8 },
  sectionHeader: { flexDirection: "row", alignItems: "baseline", justifyContent: "space-between", gap: space.space12 },
  sectionBody: { borderTopWidth: 1, paddingTop: space.space4 },
  resourceRow: { minHeight: 58, flexDirection: "row", alignItems: "center", gap: space.space12, borderBottomWidth: StyleSheet.hairlineWidth, paddingVertical: space.space8 },
  resourceCopy: { flex: 1, gap: 2 },
  emptyLine: { minHeight: 58, flexDirection: "row", alignItems: "center", gap: space.space12, paddingVertical: space.space8 },
  hostEmpty: { gap: space.space8, paddingBottom: space.space8 },
  approval: { borderWidth: 1, borderRadius: radii.radius12, padding: space.space16, marginTop: space.space8, gap: space.space12 },
  approvalTop: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  pendingDot: { width: 8, height: 8, borderRadius: 4 },
  inboxCode: { fontFamily: monoFamily.bold, fontSize: 23, lineHeight: 30, letterSpacing: 2 },
  flushBanner: { marginHorizontal: 0 },
  localNote: { textAlign: "center", marginTop: space.space32 },
});
