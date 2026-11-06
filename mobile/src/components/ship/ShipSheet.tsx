import React, { useEffect, useState } from "react";
import { Linking, StyleSheet, Text, View } from "react-native";

import type { ShipResponse } from "../../lib/apiShip";
import { usePrAvailable, useShipSession } from "../../lib/queriesPr";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Badge, type BadgeTone } from "../ds/Badge";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";
import {
  shipFinished,
  shipHeadline,
  shipPlanView,
  shipRequestBody,
  shipRows,
  type ShipRowState,
} from "./shipModel";

const BADGE: Record<ShipRowState, { label: string; tone: BadgeTone }> = {
  pending: { label: "waiting", tone: "neutral" },
  active: { label: "working", tone: "accent" },
  done: { label: "done", tone: "success" },
  skipped: { label: "skipped", tone: "neutral" },
  refused: { label: "stopped", tone: "danger" },
  blocked: { label: "not run", tone: "neutral" },
};

interface Props {
  visible: boolean;
  onClose: () => void;
  sessionId: string | null;
}

/** One tap from a finished session to a merged PR: commits, pushes, opens the PR and arms PR Watch
 * in merge mode, stopping at the first refusal with its reason. */
export function ShipSheet({ visible, onClose, sessionId }: Props) {
  const tokens = useTokens();
  const direct = usePrAvailable();
  const ship = useShipSession();
  const preview = useShipSession();
  const [plan, setPlan] = useState<ShipResponse | null>(null);
  const [title, setTitle] = useState("");
  const [response, setResponse] = useState<ShipResponse | null>(null);
  const [failure, setFailure] = useState<string | null>(null);

  useEffect(() => {
    if (!visible) return;
    setTitle("");
    setResponse(null);
    setFailure(null);
    setPlan(null);
    ship.reset();
    preview.reset();
    if (sessionId && direct) {
      preview.mutate(
        { sessionId, request: { preview: true } },
        { onSuccess: setPlan, onError: (error) => setFailure(error.message) },
      );
    }
    // Reset only when the sheet opens for a session; `ship` is a fresh object on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible, sessionId]);

  const running = ship.isPending;
  const previewing = preview.isPending;
  const confirmable = plan?.ok === true && plan.plan != null && !response && !failure;
  const planView = confirmable && plan?.plan ? shipPlanView(plan.plan) : null;
  const rows = shipRows(response, running);
  const finished = shipFinished(response);
  const url = response?.pr?.url ?? null;

  const recheck = () => {
    if (!sessionId) return;
    setFailure(null);
    setResponse(null);
    setPlan(null);
    preview.mutate(
      { sessionId, request: { preview: true } },
      { onSuccess: setPlan, onError: (error) => setFailure(error.message) },
    );
  };

  const start = () => {
    if (!sessionId || !confirmable) return;
    setFailure(null);
    setResponse(null);
    ship.mutate(
      { sessionId, request: shipRequestBody(title) },
      {
        onSuccess: setResponse,
        onError: (error) => setFailure(error.message),
      },
    );
  };

  return (
    <Sheet visible={visible && sessionId != null} onClose={onClose} accessibilityLabel="Ship session" snapPoints={[0.85]}>
      <View style={styles.content}>
        <Text style={[typeScale.heading, { color: tokens.ink }]}>Ship to a merged PR</Text>
        {!direct ? (
          <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
            Shipping needs a direct connection to the host: it runs git and the GitHub CLI there.
          </Text>
        ) : (
          <>
            <Text style={[typeScale.sub, { color: failure || (response && !response.ok) ? tokens.danger : tokens.ink3 }]}>
              {failure ??
                (previewing
                  ? "Checking what would be committed…"
                  : plan && !plan.ok && !response
                    ? shipHeadline(plan, false)
                    : shipHeadline(response, running))}
            </Text>
            {planView ? (
              <View style={[styles.row, { borderColor: tokens.border }]} accessibilityLabel="Files to commit">
                <Text style={[typeScale.body, { color: tokens.ink }]}>{planView.heading}</Text>
                {planView.files.slice(0, 30).map((file) => (
                  <Text key={file.path} style={[typeScale.monoMeta, { color: tokens.ink3 }]} numberOfLines={1}>
                    {file.mark} {file.path}
                  </Text>
                ))}
                {planView.files.length > 30 ? (
                  <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>+{planView.files.length - 30} more</Text>
                ) : null}
                {planView.leftOut ? (
                  <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]} numberOfLines={2}>
                    {planView.leftOut}
                  </Text>
                ) : null}
              </View>
            ) : null}
            {!running && !finished && confirmable ? (
              <Input
                label="Pull request title (optional)"
                value={title}
                onChangeText={setTitle}
                placeholder="generated from the commits"
                autoCapitalize="sentences"
                accessibilityLabel="Pull request title"
                returnKeyType="done"
                onSubmitEditing={start}
              />
            ) : null}
            <View style={styles.rows}>
              {rows.map((row) => (
                <View key={row.key} style={[styles.row, { borderColor: tokens.border }]}>
                  <View style={styles.rowHead}>
                    <Text style={[typeScale.body, styles.flex, { color: tokens.ink }]}>{row.label}</Text>
                    <Badge label={BADGE[row.state].label} tone={BADGE[row.state].tone} />
                  </View>
                  {row.detail ? (
                    <Text style={[typeScale.monoMeta, { color: row.state === "refused" ? tokens.danger : tokens.ink3 }]} numberOfLines={3}>
                      {row.detail}
                    </Text>
                  ) : null}
                </View>
              ))}
            </View>
            <View style={styles.buttons}>
              {url ? (
                <Button label="Open pull request" variant="secondary" onPress={() => void Linking.openURL(url).catch(() => undefined)} />
              ) : null}
              {(finished && !response?.ok) || (!confirmable && !previewing && !finished) ? (
                <Button label={finished ? "Try again" : "Check again"} variant="secondary" onPress={recheck} />
              ) : null}
              {finished ? (
                <Button label="Done" variant="primary" onPress={onClose} />
              ) : (
                <Button
                  label="Ship"
                  variant="primary"
                  onPress={start}
                  loading={running}
                  disabled={running || previewing || !confirmable || sessionId == null}
                />
              )}
            </View>
          </>
        )}
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space16, paddingBottom: space.space32, gap: space.space12 },
  rows: { gap: space.space8 },
  row: { borderWidth: StyleSheet.hairlineWidth, borderRadius: 4, padding: space.space12, gap: space.space4 },
  rowHead: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  flex: { flex: 1 },
  buttons: { flexDirection: "row", justifyContent: "flex-end", gap: space.space8, flexWrap: "wrap" },
});
