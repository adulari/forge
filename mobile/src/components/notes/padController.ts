// Owns one scratchpad's editing state and its saves, outside React, so the rules are testable and
// a re-render can never trigger a save: a save runs only from the debounce timer, from `flush`
// (blur, preview, unmount), or as the follow-up to a save that was already in flight.
//
// One save is in flight at a time. Typing during a save is queued and sent afterwards on top of the
// revision the first save returned, so a person's own consecutive saves never conflict.
import type { ScratchpadConflict } from "../../lib/apiNotes";
import { EMPTY_PAD, padReducer, type PadAction, type PadState } from "./notesModel";

export const PAD_SAVE_DELAY_MS = 800;
/** Typing without a pause still saves this often, so a crash loses seconds, not a session. */
export const PAD_SAVE_MAX_WAIT_MS = 3000;

export interface PadControllerOptions {
  send: (body: string, expectedUpdatedAt: number) => Promise<{ updated_at: number }>;
  asConflict: (error: unknown) => ScratchpadConflict | null;
  onChange: (state: PadState) => void;
  delayMs?: number;
}

export class PadController {
  state: PadState = EMPTY_PAD;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private inflight = false;
  private again = false;
  private dirtySince: number | null = null;

  private readonly options: PadControllerOptions;

  constructor(options: PadControllerOptions) {
    this.options = options;
  }

  /** Point saves at the current mutation function without rebuilding the controller. */
  setSend(send: PadControllerOptions["send"]): void {
    this.options.send = send;
  }

  dispatch(action: PadAction): void {
    this.state = padReducer(this.state, action);
    this.options.onChange(this.state);
    if (action.type === "edit" || action.type === "keepMine" || action.type === "saved") this.schedule();
  }

  private schedule(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    if (this.state.status !== "dirty" || this.state.conflict) return;
    const now = Date.now();
    this.dirtySince ??= now;
    const wait = Math.max(0, Math.min(this.options.delayMs ?? PAD_SAVE_DELAY_MS, this.dirtySince + PAD_SAVE_MAX_WAIT_MS - now));
    this.timer = setTimeout(() => void this.flush(), wait);
  }

  /** Save now if there is something to save. Safe to call at any time and as often as you like. */
  async flush(): Promise<void> {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    if (this.inflight) {
      this.again = true;
      return;
    }
    const { draft, baseUpdatedAt, status, conflict } = this.state;
    if (status !== "dirty" || conflict) return;
    this.inflight = true;
    this.dirtySince = null;
    this.dispatch({ type: "saving" });
    try {
      const result = await this.options.send(draft, baseUpdatedAt);
      this.dispatch({ type: "saved", body: draft, updatedAt: result.updated_at });
    } catch (error) {
      const found = this.options.asConflict(error);
      this.dispatch(found ? { type: "conflict", conflict: found } : { type: "failed" });
    } finally {
      this.inflight = false;
      if (this.again) {
        this.again = false;
        void this.flush();
      }
    }
  }

  dispose(): void {
    void this.flush();
  }
}
