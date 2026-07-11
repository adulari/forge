// T3.3 — Review segment: renders PlanCard (when `snapshot.plan`) + DiffCard
// (when `snapshot.diff`, pending or landed — DiffCard itself renders the
// pending warn banner), EmptyState otherwise. Mirrors the session shell
// contract (BUILD_ORDER.md T3.1 HANDOFF): this segment owns its own
// `<Screen edges={["left","right","bottom"]}>` — the shell above already
// applied the top safe-area + gutter for the header/status-strip/Segmented.
import { FileDiff } from "lucide-react-native";
import React from "react";
import { StyleSheet } from "react-native";

import { DiffCard } from "../../../components/review/DiffCard";
import { PlanCard } from "../../../components/review/PlanCard";
import { EmptyState } from "../../../components/ds/EmptyState";
import { Screen } from "../../../components/ds/Screen";
import { useSessionCtx } from "../../../lib/sessionContext";
import { space } from "../../../theme/tokens";

export default function Review() {
  const { snapshot, send, setPendingAnswer } = useSessionCtx();

  const plan = snapshot?.plan ?? null;
  const diff = snapshot?.diff ?? null;
  const hasContent = plan != null || diff != null;

  return (
    <Screen edges={["left", "right", "bottom"]} scroll contentContainerStyle={styles.content}>
      {!hasContent ? (
        <EmptyState icon={FileDiff} message="nothing to review yet" />
      ) : (
        <>
          {plan ? (
            <PlanCard
              plan={plan}
              questionOptions={snapshot?.question_options ?? []}
              promptSeq={snapshot?.prompt_seq ?? 0}
              send={send}
              onQueueAnswer={setPendingAnswer}
            />
          ) : null}
          {diff ? <DiffCard diff={diff} /> : null}
        </>
      )}
    </Screen>
  );
}

const styles = StyleSheet.create({
  content: { paddingVertical: space.space16, gap: space.space16 },
});
