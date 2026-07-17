// Hearth Fork sheet — a parallel line from any point; the original keeps running.
// The wire (`ForkSessionRequest`) carries ONLY `at_seq`, so the base point is the one
// real, functional control here. Name is a spec/design affordance the server does not
// persist (noted); worktree is shown READ-ONLY from the base session's real
// `SessionRow.worktree` rather than faked as a toggle the API can't honour.
import { router } from "expo-router";
import { ChevronDown, GitBranch } from "lucide-react-native";
import React, { useMemo, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import type { HistoryRow } from "../../lib/api";
import { useForkSession, useHistory, useSessions } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";

function preview(row: HistoryRow) {
  return row.content.replace(/\s+/g, " ").trim();
}

function worktreeLabel(path: string): string {
  const seg = path.replace(/\/+$/, "").split("/").pop() ?? path;
  return seg.length > 16 ? `${seg.slice(0, 15)}…` : seg;
}

export function ForkSheet({ visible, onClose, sessionId }: { visible: boolean; onClose: () => void; sessionId: string }) {
  const tokens = useTokens();
  const history = useHistory(sessionId);
  const sessions = useSessions();
  const fork = useForkSession();
  const [name, setName] = useState("");
  const [selectedSeq, setSelectedSeq] = useState<number | null>(null);
  const [pickerOpen, setPickerOpen] = useState(false);

  const prompts = useMemo(
    () => history.data?.pages.flat().filter((row) => row.role === "user").slice().reverse() ?? [],
    [history.data?.pages],
  );
  const base = useMemo(() => {
    if (prompts.length === 0) return null;
    return prompts.find((row) => row.seq === selectedSeq) ?? prompts[0];
  }, [prompts, selectedSeq]);
  const baseSession = useMemo(() => sessions.data?.find((s) => s.id === sessionId) ?? null, [sessions.data, sessionId]);

  const close = () => {
    setPickerOpen(false);
    onClose();
  };
  const startFork = () => {
    if (!base) return;
    fork.mutate(
      { id: sessionId, body: { at_seq: base.seq } },
      {
        onSuccess: (session) => {
          setName("");
          setSelectedSeq(null);
          close();
          router.push(`/session/${session.id}`);
        },
      },
    );
  };

  return (
    <Sheet visible={visible} onClose={close} accessibilityLabel="Fork session" snapPoints={[0.82]}>
      <View style={styles.content}>
        <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Fork session</Text>
        <Text style={[typeScale.sub, styles.subtitle, { color: tokens.ink3 }]}>
          A parallel line from any point — the original keeps running.
        </Text>

        <Text style={[typeScale.meta, styles.fieldLabel, { color: tokens.ink3 }]}>Name</Text>
        <Input
          value={name}
          onChangeText={setName}
          placeholder="retry-loop experiment"
          autoCapitalize="sentences"
          numberOfLines={1}
          accessibilityLabel="Fork name"
          returnKeyType="done"
        />

        <Text style={[typeScale.meta, styles.fieldLabel, { color: tokens.ink3 }]}>Fork from</Text>
        {base ? (
          <Pressable
            onPress={() => setPickerOpen((open) => !open)}
            accessibilityRole="button"
            accessibilityState={{ expanded: pickerOpen }}
            accessibilityLabel={`Base point: ${preview(base)}`}
            style={[styles.baseRow, { backgroundColor: tokens.bg0, borderColor: pickerOpen ? tokens.accent : tokens.border }]}
          >
            <Text style={[typeScale.body, styles.baseText, { color: tokens.ink }]} numberOfLines={1}>{preview(base)}</Text>
            <Text style={[styles.mono, tabularNums, { color: tokens.ink4 }]}>msg {base.seq}</Text>
            <ChevronDown size={15} strokeWidth={1.75} color={tokens.ink3} style={pickerOpen ? styles.chevronOpen : undefined} />
          </Pressable>
        ) : (
          <Text style={[typeScale.sub, styles.helper, { color: tokens.ink3 }]}>
            No prompts to fork from yet — send a message first.
          </Text>
        )}

        {pickerOpen && base ? (
          <ScrollView style={styles.picker} keyboardShouldPersistTaps="handled" nestedScrollEnabled>
            {prompts.map((row, index) => {
              const active = row.seq === base.seq;
              return (
                <Pressable
                  key={row.seq}
                  onPress={() => {
                    setSelectedSeq(row.seq);
                    setPickerOpen(false);
                  }}
                  accessibilityRole="radio"
                  accessibilityState={{ selected: active }}
                  style={[
                    styles.pickerRow,
                    index > 0 ? { borderTopColor: tokens.hairline, borderTopWidth: StyleSheet.hairlineWidth } : null,
                    active ? { backgroundColor: tokens.selection } : null,
                  ]}
                >
                  <Text style={[typeScale.sub, styles.baseText, { color: active ? tokens.ink : tokens.ink2 }]} numberOfLines={1}>{preview(row)}</Text>
                  <Text style={[styles.mono, tabularNums, { color: tokens.ink4 }]}>msg {row.seq}</Text>
                </Pressable>
              );
            })}
          </ScrollView>
        ) : null}

        {baseSession?.worktree ? (
          <View style={styles.worktreeRow}>
            <GitBranch size={15} strokeWidth={1.75} color={tokens.ink3} />
            <Text style={[typeScale.sub, styles.baseText, { color: tokens.ink2 }]}>Isolated git worktree</Text>
            <Text style={[styles.mono, { color: tokens.ink4 }]} numberOfLines={1}>{worktreeLabel(baseSession.worktree)}</Text>
          </View>
        ) : null}

        {fork.isError ? (
          <Text style={[typeScale.sub, styles.helper, { color: tokens.danger }]} numberOfLines={2}>
            Could not fork this session. Try again.
          </Text>
        ) : null}

        <Button
          label="Forge fork"
          onPress={startFork}
          fullWidth
          disabled={!base}
          loading={fork.isPending}
          icon={<GitBranch size={16} strokeWidth={2} color={tokens.bg2} />}
        />
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space20, paddingBottom: space.space24, gap: space.space8 },
  subtitle: { marginTop: 2, marginBottom: space.space4 },
  fieldLabel: { marginTop: space.space8 },
  baseRow: { flexDirection: "row", alignItems: "center", gap: space.space8, minHeight: 48, paddingHorizontal: space.space12, borderWidth: 1, borderRadius: radii.radius12 },
  baseText: { flex: 1, minWidth: 0 },
  chevronOpen: { transform: [{ rotate: "180deg" }] },
  picker: { maxHeight: 168, borderWidth: 1, borderColor: "transparent", marginTop: space.space4 },
  pickerRow: { minHeight: 44, flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space12, paddingVertical: space.space8 },
  worktreeRow: { flexDirection: "row", alignItems: "center", gap: space.space8, minHeight: 44 },
  helper: { marginVertical: space.space4 },
  mono: { fontFamily: monoFamily.regular, fontSize: 11, lineHeight: 15 },
});
