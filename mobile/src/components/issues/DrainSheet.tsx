// The Drain sheet: how many of the selected issues run at once, the spend envelope, and PR Watch.
import React, { useState } from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";

import type { PrWatchMethod, PrWatchMode } from "../../lib/apiPr";
import { useStartDrain } from "../../lib/queriesIssues";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Chip } from "../ds/Chip";
import { Segmented } from "../ds/Segmented";
import { Switch } from "../ds/Switch";
import { useToast } from "../ds/ToastHost";
import { WATCH_METHODS, WATCH_MODES } from "../pr/prWatchModel";
import { DRAIN_BUDGETS, drainHint, drainRequest, type IssueEntry } from "./issuesModel";

const PARALLEL = [1, 2, 3, 4].map((n) => ({ value: String(n), label: String(n) }));

export function DrainSheet({ selected, onStarted }: { selected: readonly IssueEntry[]; onStarted: () => void }) {
  const tokens = useTokens();
  const toast = useToast();
  const start = useStartDrain();
  const [parallel, setParallel] = useState("2");
  const [budget, setBudget] = useState("");
  const [worktree, setWorktree] = useState(true);
  const [mode, setMode] = useState<PrWatchMode>("fix");
  const [method, setMethod] = useState<PrWatchMethod>("squash");

  const begin = () =>
    start.mutate(drainRequest(selected, { parallel: Number(parallel), budget, worktree, mode, method }), {
      onSuccess: () => {
        toast.show(`Draining ${selected.length} ${selected.length === 1 ? "issue" : "issues"}.`);
        onStarted();
      },
      onError: (e) => toast.show(e.message, { tone: "danger" }),
    });

  return (
    <ScrollView keyboardShouldPersistTaps="handled" contentContainerStyle={styles.scroll}>
      <Text style={[typeScale.heading, { color: tokens.ink }]}>
        Drain {selected.length} {selected.length === 1 ? "issue" : "issues"}
      </Text>
      <Text style={[typeScale.meta, { color: tokens.ink2 }]}>At once</Text>
      <Segmented options={PARALLEL} value={parallel} onChange={setParallel} />
      <Text style={[typeScale.meta, { color: tokens.ink2 }]}>Spend limit</Text>
      <View style={styles.chips}>
        {DRAIN_BUDGETS.map((b) => (
          <Chip
            key={b.value || "none"}
            label={b.label}
            size="compact"
            selected={budget === b.value}
            onPress={() => setBudget(b.value)}
          />
        ))}
      </View>
      <View style={styles.switchRow}>
        <Text style={[typeScale.sub, { color: tokens.ink2 }]}>Work in new worktrees</Text>
        <Switch value={worktree} onValueChange={setWorktree} accessibilityLabel="Work in new worktrees" />
      </View>
      <Text style={[typeScale.meta, { color: tokens.ink2 }]}>PR Watch</Text>
      <Segmented options={WATCH_MODES} value={mode} onChange={setMode} />
      {mode === "merge" ? <Segmented options={WATCH_METHODS} value={method} onChange={setMethod} /> : null}
      <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>
        {drainHint(selected.length, Number(parallel), budget)}
      </Text>
      <Button label="Start drain" onPress={begin} loading={start.isPending} fullWidth />
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  scroll: { paddingHorizontal: space.space16, paddingBottom: space.space24, gap: space.space8 },
  chips: { flexDirection: "row", gap: space.space8, flexWrap: "wrap" },
  switchRow: { flexDirection: "row", alignItems: "center", justifyContent: "space-between" },
});
