// Live view of one onboarding job: progress while it runs, the sign-in prompt it is waiting on
// (page to open, device code, QR for another device, or a code to paste), and how it ended.
// The daemon scrubs secrets from every line before they reach this component.
import * as Clipboard from "expo-clipboard";
import { Copy, ExternalLink } from "lucide-react-native";
import React, { useMemo } from "react";
import { ActivityIndicator, StyleSheet, Text, View } from "react-native";

import { failureSummary, jobNeedsAttention, type JobView } from "../../lib/onboarding/flow";
import { openExternal } from "../../lib/onboarding/open";
import type { PromptKind, SecretSlot } from "../../lib/onboarding/types";
import { qrMatrix } from "../../lib/qr";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";
import { QrCode } from "../QrCode";
import { Button } from "../ds/Button";
import { SecretField } from "./parts";

export interface JobPanelProps {
  view: JobView | null;
  onInput?: (job: number, text: string) => void;
  onCancel?: (job: number) => void;
  /** Called with the token the person pasted when the flow could not capture it. */
  onPaste?: (slot: SecretSlot, value: string) => void;
  /** Hide the finished-state line (the caller shows its own). */
  quietWhenDone?: boolean;
}

function canEncode(value: string): boolean {
  try {
    qrMatrix(value);
    return true;
  } catch {
    return false;
  }
}

export function JobPanel({ view, onInput, onCancel, onPaste, quietWhenDone }: JobPanelProps) {
  const tokens = useTokens();
  if (!view) return null;
  const lastLine = view.lines.at(-1);
  if (view.state === "running") {
    return (
      <View style={[styles.panel, { borderColor: tokens.border, backgroundColor: tokens.bg1 }]} testID="onboarding-job">
        {view.prompt ? (
          <SignInPrompt prompt={view.prompt.kind} message={view.prompt.message} job={view.id} onInput={onInput} />
        ) : (
          <View style={styles.row}>
            <ActivityIndicator size="small" color={tokens.accent} />
            <Text style={[type.body, { color: tokens.ink2 }]} numberOfLines={2}>
              {view.progress?.label ?? lastLine ?? view.label}
            </Text>
          </View>
        )}
        {view.prompt ? (
          <View style={styles.row}>
            <ActivityIndicator size="small" color={tokens.ink3} />
            <Text style={[type.sub, { color: tokens.ink3 }]}>Waiting for you to finish in the browser…</Text>
            {onCancel ? <Button label="Cancel" variant="ghost" onPress={() => onCancel(view.id)} /> : null}
          </View>
        ) : null}
        {view.progress?.fraction != null ? <ProgressBar fraction={view.progress.fraction} /> : null}
      </View>
    );
  }
  if (view.result?.kind === "paste_needed" && onPaste) {
    const slot = view.result.slot;
    return (
      <View style={[styles.panel, { borderColor: tokens.border, backgroundColor: tokens.bg1 }]} testID="onboarding-job">
        <Text style={[type.body, { color: tokens.ink }]}>
          The sign-in finished, but Forge could not read the token it printed. Paste it here and Forge keeps it
          safe.
        </Text>
        <SecretField label="Token" placeholder="sk-ant-oat01-…" actionLabel="Save token" onSubmit={(value) => onPaste(slot, value)} />
      </View>
    );
  }
  if (quietWhenDone && !jobNeedsAttention(view)) return null;
  const attention = jobNeedsAttention(view);
  const text = attention
    ? failureSummary(view.error ?? (view.result?.kind === "verified" ? (view.result.detail ?? "The check did not pass.") : "That did not finish."))
    : view.summary;
  if (!text) return null;
  return (
    <View style={styles.row} testID="onboarding-job-result">
      <Text style={[type.body, { color: attention ? tokens.danger : tokens.success }]}>{text}</Text>
    </View>
  );
}

function ProgressBar({ fraction }: { fraction: number }) {
  const tokens = useTokens();
  const pct = Math.max(0, Math.min(1, fraction)) * 100;
  return (
    <View style={[styles.track, { backgroundColor: tokens.bg3 }]} accessibilityRole="progressbar" accessibilityValue={{ min: 0, max: 100, now: Math.round(pct) }}>
      <View style={[styles.fill, { width: `${pct}%`, backgroundColor: tokens.accent }]} />
    </View>
  );
}

export function SignInPrompt({
  prompt,
  message,
  job,
  onInput,
}: {
  prompt: PromptKind;
  message: string;
  job: number;
  onInput?: (job: number, text: string) => void;
}) {
  const tokens = useTokens();
  const { isCompact } = useBreakpoint();
  const url = prompt.kind === "open_url" || prompt.kind === "device_code" ? prompt.url : null;
  const showQr = useMemo(() => url != null && canEncode(url), [url]);
  if (prompt.kind === "terminal") {
    return <Text style={[type.body, { color: tokens.ink }]}>{message || "This step needs a terminal."}</Text>;
  }
  if (prompt.kind === "code") {
    return (
      <View style={styles.stack}>
        <Text style={[type.body, { color: tokens.ink }]}>{message || prompt.label}</Text>
        <SecretField label={prompt.label} actionLabel="Send" onSubmit={(value) => onInput?.(job, value)} />
      </View>
    );
  }
  return (
    <View style={[styles.signIn, isCompact && styles.signInCompact]}>
      <View style={styles.signInText}>
        <Text style={[type.bodyBold, { color: tokens.ink }]}>
          {prompt.kind === "device_code" ? "Enter this code on the page" : "Sign in in your browser"}
        </Text>
        {message ? <Text style={[type.sub, { color: tokens.ink3 }]}>{message}</Text> : null}
        {prompt.kind === "device_code" ? (
          <View style={[styles.code, { borderColor: tokens.borderStrong, backgroundColor: tokens.bg2 }]} testID="onboarding-device-code">
            <Text selectable style={[styles.codeText, { color: tokens.ink }]}>
              {prompt.code}
            </Text>
          </View>
        ) : null}
        <Text selectable numberOfLines={2} style={[type.codeSmall, { color: tokens.ink2 }]} testID="onboarding-sign-in-url">
          {prompt.url}
        </Text>
        <View style={styles.actions}>
          <Button
            label={prompt.kind === "device_code" ? "Open the page" : "Open sign-in page"}
            icon={<ExternalLink size={16} strokeWidth={2} color={tokens.onAccent} />}
            onPress={() => void openExternal(prompt.url).catch(() => undefined)}
          />
          <Button
            label={prompt.kind === "device_code" ? "Copy code" : "Copy link"}
            variant="secondary"
            icon={<Copy size={16} strokeWidth={2} color={tokens.ink} />}
            onPress={() => void Clipboard.setStringAsync(prompt.kind === "device_code" ? prompt.code : prompt.url)}
          />
        </View>
      </View>
      {showQr ? (
        <View style={styles.qr}>
          <QrCode value={prompt.url} size={isCompact ? 148 : 156} accessibilityLabel="QR code for the sign-in page" />
          <Text style={[type.meta, { color: tokens.ink3 }]}>Or scan with your phone</Text>
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  panel: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4, padding: space.space12, gap: space.space12 },
  row: { flexDirection: "row", alignItems: "center", gap: space.space8, flexWrap: "wrap" },
  stack: { gap: space.space8 },
  track: { height: 4, borderRadius: 2, overflow: "hidden" },
  fill: { height: 4, borderRadius: 2 },
  signIn: { flexDirection: "row", gap: space.space16, alignItems: "flex-start" },
  signInCompact: { flexDirection: "column" },
  signInText: { flex: 1, gap: space.space8, minWidth: 0 },
  actions: { flexDirection: "row", gap: space.space8, flexWrap: "wrap" },
  qr: { alignItems: "center", gap: space.space4 },
  code: { borderWidth: 1, borderRadius: radii.radius12, paddingVertical: space.space12, paddingHorizontal: space.space16, alignSelf: "flex-start" },
  codeText: { fontFamily: monoFamily.bold, fontSize: 25, lineHeight: 32, letterSpacing: 2.2, ...tabularNums },
});
