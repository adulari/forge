// Review segment: turn artifacts and the session's live working tree share one route. Expanded
// layouts can also retain Git as a workbench surface; compact iOS/Android gets the same component
// here instead of a reduced second implementation.
import { FileDiff, WifiOff } from "lucide-react-native";
import React, { useEffect, useRef, useState } from "react";
import { ScrollView, StyleSheet, View } from "react-native";

import { EmptyState } from "../../../components/ds/EmptyState";
import { Screen } from "../../../components/ds/Screen";
import { TabStrip } from "../../../components/ds/TabStrip";
import { GitReviewDock } from "../../../components/git/GitReviewDock";
import { DiffCard } from "../../../components/review/DiffCard";
import { PlanCard } from "../../../components/review/PlanCard";
import { useSessionCtx } from "../../../lib/sessionContext";
import { space } from "../../../theme/tokens";

type ReviewSurface = "turn" | "working-tree";

export default function Review() {
  const { sessionId, snapshot, snapshotTimedOut, send, setPendingAnswer } = useSessionCtx();
  const plan = snapshot?.plan ?? null;
  const diff = snapshot?.diff ?? null;
  const hasContent = plan != null || diff != null;
  const [surface, setSurface] = useState<ReviewSurface>("turn");
  const choseInitialSurface = useRef(false);

  // An idle session with no turn artifact should open useful repository state instead of an empty
  // card. Never steal the user's choice when a later diff arrives.
  useEffect(() => {
    if (snapshot == null || choseInitialSurface.current) return;
    choseInitialSurface.current = true;
    if (!hasContent) setSurface("working-tree");
  }, [hasContent, snapshot]);

  return (
    <Screen edges={["left", "right", "bottom"]} contentContainerStyle={styles.screen}>
      <TabStrip<ReviewSurface>
        options={[
          { value: "turn", label: "Turn", dot: hasContent },
          { value: "working-tree", label: "Working tree" },
        ]}
        value={surface}
        onChange={setSurface}
        testID="review-surface-tabs"
      />

      {surface === "working-tree" ? (
        <View style={styles.git}>
          <GitReviewDock sessionId={sessionId} />
        </View>
      ) : (
        <ScrollView
          style={styles.activity}
          contentContainerStyle={styles.content}
          showsVerticalScrollIndicator={false}
        >
          {snapshot == null ? (
            snapshotTimedOut ? (
              <EmptyState
                icon={WifiOff}
                message="Could not load this session for review. Check the server connection."
              />
            ) : (
              <View style={styles.loading} />
            )
          ) : !hasContent ? (
            <EmptyState icon={FileDiff} message="nothing from the current turn to review yet" />
          ) : (
            <>
              {plan ? (
                <PlanCard
                  plan={plan}
                  question={snapshot?.question ?? null}
                  questionOptions={snapshot?.question_options ?? []}
                  promptSeq={snapshot?.prompt_seq ?? 0}
                  send={send}
                  onQueueAnswer={setPendingAnswer}
                />
              ) : null}
              {diff ? <DiffCard diff={diff} /> : null}
            </>
          )}
        </ScrollView>
      )}
    </Screen>
  );
}

const styles = StyleSheet.create({
  screen: { paddingTop: space.space12, gap: space.space12 },
  activity: { flex: 1 },
  content: {
    paddingBottom: space.space16,
    gap: space.space16,
    width: "100%",
    maxWidth: 760,
    alignSelf: "center",
  },
  git: { flex: 1, minHeight: 0 },
  loading: { minHeight: 96 },
});
