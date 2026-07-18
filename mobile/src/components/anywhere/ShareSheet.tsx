// Forge Anywhere — read-only, end-to-end encrypted replay links. Design comp "AW Replay
// Share" (mobile.dc.html lines 738-794). The decryption key lives in the URL fragment
// (ReplayShare.url) and never touches this component beyond displaying/copying it whole.
import * as Clipboard from "expo-clipboard";
import { router } from "expo-router";
import { ChevronRight } from "lucide-react-native";
import React, { useCallback, useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { useAnywhere } from "../../lib/anywhere/store";
import type { ReplayShare, ShareExpiry, ShareState } from "../../lib/anywhere/types";
import { useTokens } from "../../theme/ThemeProvider";
import { space, tapTarget, type ColorTokens } from "../../theme/tokens";
import { tabularNums, type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { ConfirmDialog } from "../ds/ConfirmDialog";
import { Segmented } from "../ds/Segmented";
import { SectionHeader } from "../ds/SectionHeader";
import { Sheet } from "../ds/Sheet";
import { useToast } from "../ds/ToastHost";

export interface ShareSheetProps {
  visible: boolean;
  onClose: () => void;
  sessionId: string;
}

const EXPIRY_OPTIONS: { value: ShareExpiry; label: string }[] = [
  { value: "24h", label: "24 hours" },
  { value: "7d", label: "7 days" },
  { value: "30d", label: "30 days" },
];

function formatShareTimestamp(ms: number): string {
  const date = new Date(ms);
  const month = date.toLocaleDateString(undefined, { month: "short" });
  const hh = String(date.getHours()).padStart(2, "0");
  const mm = String(date.getMinutes()).padStart(2, "0");
  return `${month} ${date.getDate()}, ${hh}:${mm}`;
}

function formatShareDate(ms: number): string {
  const date = new Date(ms);
  return `${date.toLocaleDateString(undefined, { month: "short" })} ${date.getDate()}`;
}

/** Strips the scheme for a slightly shorter display string — Text truncation (numberOfLines)
 * handles the rest, so this doesn't try to match the comp's hand-shortened `r/8f3a…` form. */
function displayUrl(url: string): string {
  return url.replace(/^https?:\/\//, "");
}

function shareRowMeta(share: ReplayShare, tokens: ColorTokens, nowMs: number = Date.now()): { text: string; color: string } {
  switch (share.state) {
    case "active": {
      const daysLeft = Math.max(0, Math.ceil((share.expiresAt - nowMs) / (24 * 60 * 60_000)));
      return { text: `active · ${daysLeft}d left`, color: tokens.ink3 };
    }
    case "expired":
      return { text: `expired ${formatShareDate(share.expiresAt)}`, color: tokens.ink4 };
    case "revoked":
      return { text: "revoked", color: tokens.danger };
    default: {
      const _exhaustive: never = share.state;
      return _exhaustive;
    }
  }
}

function dotColorFor(state: ShareState, tokens: ColorTokens): string {
  return state === "active" ? tokens.success : tokens.ink4;
}

export function ShareSheet({ visible, onClose, sessionId }: ShareSheetProps) {
  const tokens = useTokens();
  const toast = useToast();
  const { client } = useAnywhere();

  const [expiry, setExpiry] = useState<ShareExpiry>("7d");
  const [creating, setCreating] = useState(false);
  const [lastCreated, setLastCreated] = useState<ReplayShare | null>(null);
  const [shares, setShares] = useState<ReplayShare[]>([]);
  const [revokeTarget, setRevokeTarget] = useState<ReplayShare | null>(null);

  const refreshShares = useCallback(async () => {
    setShares(await client.listShares(sessionId));
  }, [client, sessionId]);

  useEffect(() => {
    if (!visible) return;
    setLastCreated(null);
    void refreshShares();
  }, [visible, refreshShares]);

  const onCreate = useCallback(async () => {
    setCreating(true);
    try {
      const share = await client.createShare(sessionId, expiry);
      setLastCreated(share);
      await refreshShares();
    } finally {
      setCreating(false);
    }
  }, [client, sessionId, expiry, refreshShares]);

  const onCopy = useCallback(
    async (url: string) => {
      await Clipboard.setStringAsync(url);
      toast.show("share link copied");
    },
    [toast],
  );

  const onConfirmRevoke = useCallback(async () => {
    if (!revokeTarget) return;
    await client.revokeShare(revokeTarget.id);
    if (lastCreated?.id === revokeTarget.id) setLastCreated(null);
    setRevokeTarget(null);
    await refreshShares();
  }, [revokeTarget, client, lastCreated, refreshShares]);

  const previewId = lastCreated?.id ?? shares.find((s) => s.state === "active")?.id ?? null;

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Share replay" snapPoints={[0.9]}>
      <View style={styles.content}>
        <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Share replay</Text>
        <Text style={[typeScale.sub, styles.privacyNote, { color: tokens.ink2 }]}>
          Read-only, end-to-end encrypted. The key travels in the link fragment — the relay never sees it.
        </Text>

        <SectionHeader>expires after</SectionHeader>
        <Segmented options={EXPIRY_OPTIONS} value={expiry} onChange={setExpiry} testID="share-expiry-segmented" />
        <Button label="Create share link" onPress={onCreate} loading={creating} fullWidth style={styles.createButton} />

        {lastCreated ? (
          <View>
            <SectionHeader>created</SectionHeader>
            <View style={styles.createdRow}>
              <Text style={[typeScale.monoMeta, styles.flex1, { color: tokens.ink }]} numberOfLines={1}>
                {displayUrl(lastCreated.url)}
              </Text>
              <Pressable
                onPress={() => onCopy(lastCreated.url)}
                accessibilityRole="button"
                accessibilityLabel="Copy share link"
                hitSlop={8}
              >
                <Text style={[typeScale.meta, styles.actionLabel, { color: tokens.accent }]}>Copy</Text>
              </Pressable>
            </View>
            <View style={styles.createdMetaRow}>
              <Text style={[typeScale.monoMeta, tabularNums, styles.flex1, { color: tokens.ink3 }]} numberOfLines={1}>
                {`expires ${formatShareTimestamp(lastCreated.expiresAt)} · viewed 0 times`}
              </Text>
              <Pressable
                onPress={() => setRevokeTarget(lastCreated)}
                accessibilityRole="button"
                accessibilityLabel="Revoke share link"
                hitSlop={8}
              >
                <Text style={[typeScale.meta, styles.actionLabel, { color: tokens.danger }]}>Revoke</Text>
              </Pressable>
            </View>
          </View>
        ) : null}

        <SectionHeader>shares for this session</SectionHeader>
        {shares.length === 0 ? (
          <Text style={[typeScale.meta, styles.emptyNote, { color: tokens.ink4 }]}>No shares yet.</Text>
        ) : (
          shares.map((share) => {
            const meta = shareRowMeta(share, tokens);
            return (
              <View key={share.id} style={styles.shareRow}>
                <View style={[styles.shareDot, { backgroundColor: dotColorFor(share.state, tokens) }]} />
                <Text style={[typeScale.monoMeta, styles.flex1, { color: tokens.ink }]} numberOfLines={1}>
                  {displayUrl(share.url)}
                </Text>
                <Text style={[typeScale.monoMeta, tabularNums, { color: meta.color }]} numberOfLines={1}>
                  {meta.text}
                </Text>
                {share.state === "active" ? (
                  <Pressable
                    onPress={() => setRevokeTarget(share)}
                    accessibilityRole="button"
                    accessibilityLabel="Revoke share link"
                    hitSlop={8}
                  >
                    <Text style={[typeScale.meta, styles.actionLabel, { color: tokens.danger }]}>Revoke</Text>
                  </Pressable>
                ) : null}
              </View>
            );
          })
        )}

        {previewId ? (
          <Pressable
            onPress={() => router.push(`/shares/${previewId}` as never)}
            accessibilityRole="button"
            accessibilityLabel="Preview what recipients see"
            style={styles.previewRow}
          >
            <Text style={[typeScale.bodyBold, styles.previewLabel, { color: tokens.accent }]}>Preview what recipients see</Text>
            <ChevronRight size={13} color={tokens.accent} strokeWidth={2} />
          </Pressable>
        ) : null}
      </View>

      <ConfirmDialog
        visible={revokeTarget != null}
        title="Revoke share link"
        message="Anyone with this link will immediately lose access. This can't be undone."
        confirmLabel="Revoke"
        destructive
        onConfirm={onConfirmRevoke}
        onCancel={() => setRevokeTarget(null)}
      />
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space20, paddingBottom: space.space24, gap: space.space4 },
  flex1: { flex: 1, minWidth: 0 },
  privacyNote: { marginBottom: space.space8, lineHeight: 19 },
  createButton: { marginTop: space.space12 },
  createdRow: { flexDirection: "row", alignItems: "center", gap: space.space12, minHeight: tapTarget },
  createdMetaRow: { flexDirection: "row", alignItems: "center", gap: space.space12, marginTop: -4 },
  actionLabel: { fontWeight: "600" },
  emptyNote: { paddingVertical: space.space8 },
  shareRow: { flexDirection: "row", alignItems: "center", gap: space.space12, minHeight: tapTarget },
  shareDot: { width: 7, height: 7, borderRadius: 3.5, flexShrink: 0 },
  previewRow: { flexDirection: "row", alignItems: "center", gap: space.space8, minHeight: tapTarget, marginTop: space.space8 },
  previewLabel: { flex: 1 },
});
