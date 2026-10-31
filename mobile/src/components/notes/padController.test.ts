import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "../../lib/api";
import { scratchpadConflict } from "../../lib/apiNotes";
import { PAD_SAVE_DELAY_MS, PadController } from "./padController";
import type { PadState } from "./notesModel";

vi.mock("../../lib/transport", () => ({ tFetch: vi.fn(), TWebSocket: class {} }));

interface Sent {
  body: string;
  base: number;
}

function harness(send?: (sent: Sent) => Promise<{ updated_at: number }>) {
  const sent: Sent[] = [];
  let revision = 0;
  const states: PadState[] = [];
  const controller = new PadController({
    send: (body, base) => {
      sent.push({ body, base });
      if (send) return send({ body, base });
      revision += 10;
      return Promise.resolve({ updated_at: revision });
    },
    asConflict: scratchpadConflict,
    onChange: (s) => states.push(s),
  });
  return { controller, sent, states };
}

describe("PadController", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("sends one save after typing stops, not one per keystroke", async () => {
    const { controller, sent } = harness();
    controller.dispatch({ type: "server", body: "", updatedAt: 0 });
    for (const text of ["a", "ab", "abc"]) {
      controller.dispatch({ type: "edit", text });
      await vi.advanceTimersByTimeAsync(100);
    }
    expect(sent).toEqual([]);
    await vi.advanceTimersByTimeAsync(PAD_SAVE_DELAY_MS);
    expect(sent).toEqual([{ body: "abc", base: 0 }]);
    expect(controller.state).toMatchObject({ status: "clean", baseUpdatedAt: 10, conflict: null });
  });

  it("types continuously for ten seconds without ever raising a conflict", async () => {
    // A stand-in server that applies the same rule as the daemon.
    let stored = { body: "", updated_at: 0 };
    const { controller, sent } = harness(async ({ body, base }) => {
      if (stored.updated_at !== base && stored.body !== body) {
        throw new ApiError(409, "conflict", { body: stored.body, updated_at: stored.updated_at });
      }
      stored = { body, updated_at: stored.updated_at + 1 };
      return { updated_at: stored.updated_at };
    });
    controller.dispatch({ type: "server", body: "", updatedAt: 0 });
    let text = "";
    for (let t = 0; t < 10_000; t += 120) {
      text += "x";
      controller.dispatch({ type: "edit", text });
      await vi.advanceTimersByTimeAsync(120);
      // Our own save makes the app refetch; that echo must never read as another device.
      controller.dispatch({ type: "server", body: stored.body, updatedAt: stored.updated_at });
    }
    await vi.advanceTimersByTimeAsync(2 * PAD_SAVE_DELAY_MS);
    expect(controller.state.conflict).toBeNull();
    expect(stored.body).toBe(text);
    expect(sent.length).toBeGreaterThan(1);
    expect(controller.state.status).toBe("clean");
  });

  it("queues typing made during a save and sends it on the revision the save returned", async () => {
    let release: (() => void) | undefined;
    const { controller, sent } = harness(
      ({ body }) =>
        new Promise((resolve) => {
          release = () => resolve({ updated_at: body.length * 100 });
        }),
    );
    controller.dispatch({ type: "server", body: "", updatedAt: 0 });
    controller.dispatch({ type: "edit", text: "a" });
    await vi.advanceTimersByTimeAsync(PAD_SAVE_DELAY_MS);
    expect(sent).toEqual([{ body: "a", base: 0 }]);

    controller.dispatch({ type: "edit", text: "ab" });
    await vi.advanceTimersByTimeAsync(PAD_SAVE_DELAY_MS * 2);
    expect(sent).toHaveLength(1);
    release?.();
    await vi.advanceTimersByTimeAsync(0);
    await vi.advanceTimersByTimeAsync(PAD_SAVE_DELAY_MS);
    expect(sent[1]).toEqual({ body: "ab", base: 100 });
  });

  it("flush is idempotent and repeated flushes do not duplicate a save", async () => {
    const { controller, sent } = harness();
    controller.dispatch({ type: "server", body: "", updatedAt: 0 });
    controller.dispatch({ type: "edit", text: "hello" });
    await Promise.all([controller.flush(), controller.flush(), controller.flush()]);
    await vi.advanceTimersByTimeAsync(0);
    expect(sent).toEqual([{ body: "hello", base: 0 }]);
  });

  it("a real conflict from another device raises the banner and stops saving", async () => {
    const { controller, sent } = harness(() =>
      Promise.reject(new ApiError(409, "conflict", { body: "theirs", updated_at: 77 })),
    );
    controller.dispatch({ type: "server", body: "", updatedAt: 0 });
    controller.dispatch({ type: "edit", text: "mine" });
    await vi.advanceTimersByTimeAsync(PAD_SAVE_DELAY_MS);
    expect(controller.state.conflict).toEqual({ body: "theirs", updated_at: 77 });
    controller.dispatch({ type: "edit", text: "mine!" });
    await vi.advanceTimersByTimeAsync(PAD_SAVE_DELAY_MS * 3);
    expect(sent).toHaveLength(1);
  });

  it("a controller per project keeps one project's text out of another", async () => {
    const a = harness();
    const b = harness();
    a.controller.dispatch({ type: "server", body: "", updatedAt: 0 });
    a.controller.dispatch({ type: "edit", text: "project A text" });
    b.controller.dispatch({ type: "server", body: "B original", updatedAt: 5 });
    await vi.advanceTimersByTimeAsync(PAD_SAVE_DELAY_MS);
    expect(b.sent).toEqual([]);
    expect(b.controller.state.draft).toBe("B original");
    expect(b.controller.state.conflict).toBeNull();
  });
});
