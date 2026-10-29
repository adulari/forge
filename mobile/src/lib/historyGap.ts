export interface GapMarker {
  count: number;
  reason: string;
}

export interface GapNotice {
  /** Stable for one gap, so acknowledging it does not hide a larger one later. */
  id: string;
  title: string;
  detail: string;
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** A reconnect the host could not replay: the screen was rebuilt from the current state. */
export function liveGapNotice(gap: GapMarker, revision: number): GapNotice {
  return {
    id: `live:${revision}:${gap.count}`,
    title: "History incomplete",
    detail: `${plural(gap.count, "live update", "live updates")} passed while this device was disconnected. The transcript was reloaded from the host, but streamed output and notes from that time may be missing.`,
  };
}

/** Rows the host's store no longer has. */
export function missingRowsNotice(gap: GapMarker): GapNotice {
  return {
    id: `rows:${gap.count}`,
    title: "History incomplete",
    detail: `${plural(gap.count, "message is", "messages are")} missing from this session's stored history, so the transcript is shorter than the conversation was.`,
  };
}

export function gapAckKey(baseUrl: string, sessionId: string): string {
  return `forge.historyGap.ack.v1:${baseUrl}|${sessionId}`;
}

/** Whether a stored-history gap still needs showing: only a gap larger than the one already
 * acknowledged does. */
export function needsAck(gap: GapMarker | null, acknowledgedCount: number): boolean {
  return gap != null && gap.count > acknowledgedCount;
}
