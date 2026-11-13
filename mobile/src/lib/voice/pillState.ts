export const MAX_RECORDING_SECONDS = 15 * 60;

export type PillPhase = "recording" | "transcribing" | "error";

export interface PillState {
  phase: PillPhase;
  error: string | null;
  /** The recording is still held, so Retry can re-upload it. */
  retryable: boolean;
}

export type PillEvent =
  | { type: "accept" }
  | { type: "failed"; message: string; retryable: boolean }
  | { type: "retry" };

export const initialPillState: PillState = { phase: "recording", error: null, retryable: false };

export function pillReducer(state: PillState, event: PillEvent): PillState {
  switch (event.type) {
    case "accept":
      return state.phase === "recording" ? { phase: "transcribing", error: null, retryable: false } : state;
    case "failed":
      return { phase: "error", error: event.message, retryable: event.retryable };
    case "retry":
      return state.phase === "error" && state.retryable ? { phase: "transcribing", error: null, retryable: false } : state;
  }
}

export function recordingCapReached(elapsedSeconds: number): boolean {
  return elapsedSeconds >= MAX_RECORDING_SECONDS;
}
