// Small building blocks shared by the onboarding steps: the step frame, a details disclosure,
// the "what we found" list, a masked paste field and the source rows of the model step.
import { Check, ChevronDown, ChevronRight, CircleAlert, ExternalLink, Minus } from "lucide-react-native";
import React, { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { Badge, type BadgeTone } from "../ds/Badge";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { openExternal } from "../../lib/onboarding/open";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";

export function FactList({ lines }: { lines: string[] }) {
  const tokens = useTokens();
  if (lines.length === 0) return null;
  return (
    <View style={styles.facts} testID="onboarding-detected">
      <Text style={[type.section, { color: tokens.ink4, fontFamily: monoFamily.medium }]}>FOUND ON THIS COMPUTER</Text>
      {lines.map((line) => (
        <Text key={line} style={[type.body, { color: tokens.ink }]}>
          {line}
        </Text>
      ))}
    </View>
  );
}

export function Disclosure({ label, children }: { label: string; children: React.ReactNode }) {
  const tokens = useTokens();
  const [open, setOpen] = useState(false);
  const Icon = open ? ChevronDown : ChevronRight;
  return (
    <View style={styles.disclosure}>
      <Pressable
        accessibilityRole="button"
        accessibilityState={{ expanded: open }}
        onPress={() => setOpen((v) => !v)}
        style={styles.disclosureHead}
      >
        <Icon size={14} strokeWidth={2} color={tokens.ink3} />
        <Text style={[type.bodyBold, { color: tokens.ink2 }]}>{label}</Text>
      </Pressable>
      {open ? <View style={styles.disclosureBody}>{children}</View> : null}
    </View>
  );
}

export function Hint({ children }: { children: React.ReactNode }) {
  const tokens = useTokens();
  return <Text style={[type.sub, { color: tokens.ink3 }]}>{children}</Text>;
}

/** A pasted secret: masked, never echoed, cleared once it is sent. */
export function SecretField({
  label,
  placeholder,
  actionLabel = "Save",
  keyUrl,
  busy,
  onSubmit,
}: {
  label: string;
  placeholder?: string;
  actionLabel?: string;
  keyUrl?: string | null;
  busy?: boolean;
  onSubmit: (value: string) => void;
}) {
  const tokens = useTokens();
  const [value, setValue] = useState("");
  return (
    <View style={styles.secret}>
      <Input
        label={label}
        value={value}
        onChangeText={setValue}
        placeholder={placeholder}
        secureTextEntry
        autoCapitalize="none"
        autoCorrect={false}
        mono
        clearable={false}
        disabled={busy}
        onSubmitEditing={() => value.trim() && onSubmit(value.trim())}
      />
      <View style={styles.secretActions}>
        <Button
          label={actionLabel}
          variant="secondary"
          disabled={!value.trim()}
          loading={busy}
          onPress={() => {
            onSubmit(value.trim());
            setValue("");
          }}
        />
        {keyUrl ? (
          <Button
            label="Get a key"
            variant="ghost"
            icon={<ExternalLink size={14} strokeWidth={2} color={tokens.ink2} />}
            onPress={() => void openExternal(keyUrl).catch(() => undefined)}
          />
        ) : null}
      </View>
    </View>
  );
}

export type SourceTone = "ready" | "attention" | "idle";

export function toneBadge(tone: SourceTone): { label: string; tone: BadgeTone } {
  if (tone === "ready") return { label: "Ready", tone: "success" };
  if (tone === "attention") return { label: "Needs you", tone: "warn" };
  return { label: "Not set up", tone: "neutral" };
}

export function StatusGlyph({ status }: { status: "done" | "running" | "failed" | "pending" | "waiting" | "skipped" }) {
  const tokens = useTokens();
  const common = { size: 14, strokeWidth: 2.5 } as const;
  let glyph: React.ReactNode;
  if (status === "done") glyph = <Check {...common} color={tokens.success} />;
  else if (status === "failed") glyph = <CircleAlert {...common} color={tokens.danger} />;
  else if (status === "skipped") glyph = <Minus {...common} color={tokens.ink3} />;
  else if (status === "waiting") glyph = <View style={[styles.glyphDot, { backgroundColor: tokens.warn }]} />;
  else if (status === "running") glyph = <View style={[styles.glyphDot, { backgroundColor: tokens.accent }]} />;
  else glyph = <View style={[styles.glyphDot, { backgroundColor: "transparent", borderColor: tokens.borderStrong, borderWidth: 1 }]} />;
  // One fixed box, so rows line up whichever glyph a status draws.
  return <View style={styles.glyphBox}>{glyph}</View>;
}

export function SourceRow({
  name,
  detail,
  tone,
  badges,
  children,
  actions,
}: {
  name: string;
  detail: string;
  tone: SourceTone;
  badges?: { label: string; tone: BadgeTone }[];
  children?: React.ReactNode;
  actions?: React.ReactNode;
}) {
  const tokens = useTokens();
  const badge = toneBadge(tone);
  return (
    <View style={[styles.source, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}>
      <View style={styles.sourceHead}>
        <View style={styles.sourceText}>
          <View style={styles.sourceTitleRow}>
            <Text style={[type.heading, { color: tokens.ink }]} numberOfLines={1}>
              {name}
            </Text>
            <Badge label={badge.label} tone={badge.tone} />
            {badges?.map((b) => <Badge key={b.label} label={b.label} tone={b.tone} />)}
          </View>
          <Text style={[type.sub, { color: tokens.ink3 }]}>{detail}</Text>
        </View>
        {actions ? <View style={styles.sourceActions}>{actions}</View> : null}
      </View>
      {children}
    </View>
  );
}

const styles = StyleSheet.create({
  facts: { gap: space.space4 },
  disclosure: { gap: space.space8 },
  disclosureHead: { flexDirection: "row", alignItems: "center", gap: space.space4, minHeight: 28 },
  disclosureBody: { gap: space.space8, paddingLeft: space.space16 },
  secret: { gap: space.space8 },
  secretActions: { flexDirection: "row", gap: space.space8, alignItems: "center" },
  glyphBox: { width: 14, height: 14, alignItems: "center", justifyContent: "center" },
  glyphDot: { width: 10, height: 10, borderRadius: 5 },
  source: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4, padding: space.space12, gap: space.space12 },
  sourceHead: { flexDirection: "row", alignItems: "flex-start", gap: space.space12, flexWrap: "wrap" },
  sourceText: { flex: 1, minWidth: 220, gap: space.space4 },
  sourceTitleRow: { flexDirection: "row", alignItems: "center", gap: space.space8, flexWrap: "wrap" },
  sourceActions: { flexDirection: "row", gap: space.space8, alignItems: "center", flexWrap: "wrap" },
});
