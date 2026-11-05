// The newest slot of the chat while a turn runs. It reads the reply text from the stream store
// itself, so a streamed token re-renders this row and nothing else on the screen.
import React from "react";
import { StyleSheet, View } from "react-native";

import { parseReasoning } from "../../lib/reasoning";
import type { StreamStore } from "../../lib/streamStore";
import { useStreamText } from "../../lib/useStreamText";
import { space } from "../../theme/tokens";
import { StreamingAnswer, TurnActivity } from "./LiveTurn";
import { SpeakerTag } from "./MessageRow";
import { ReasoningDisclosure } from "./ReasoningDisclosure";

export interface StreamingRowProps {
  store: StreamStore;
  /** What to show while the store has no text: the reply kept across the end of a turn. */
  fallbackText: string;
  streaming: boolean;
  busy: boolean;
  activity: string | undefined;
  toolLine: string | null;
  startedAt: number;
}

function StreamingRowImpl({ store, fallbackText, streaming, busy, activity, toolLine, startedAt }: StreamingRowProps) {
  const live = useStreamText(store);
  const text = (busy ? live : "") || fallbackText;
  // Split inline `<think>…</think>` out of the live text: reasoning goes to the collapsed
  // disclosure, only the answer streams in the main slot (never the raw thinking log).
  const parsed = parseReasoning(text);
  const hasReasoning = parsed.reasoning.length > 0 || parsed.thinking;
  // What the turn is doing, while no answer text is arriving: before the first token,
  // and between steps (a tool running after some prose). Never a bare spinner.
  const showActivity = streaming && busy && (!parsed.answer || (activity != null && activity !== "streaming response"));
  if (!hasReasoning && !parsed.answer && !showActivity) return null;
  return (
    <View style={styles.row}>
      {parsed.answer ? <SpeakerTag speaker="forge" /> : null}
      {hasReasoning ? <ReasoningDisclosure reasoning={parsed.reasoning} active={parsed.thinking && streaming} /> : null}
      {parsed.answer ? <StreamingAnswer text={parsed.answer} streaming={streaming && !showActivity} /> : null}
      {showActivity ? <TurnActivity activity={activity} toolLine={toolLine} startedAt={startedAt} /> : null}
    </View>
  );
}

export const StreamingRow = React.memo(StreamingRowImpl);

const styles = StyleSheet.create({
  row: { paddingHorizontal: space.space16, paddingVertical: space.space8 },
});
