import { describe, expect, it } from "vitest";

import type { OverlayRow } from "../../lib/ws";
import { badgeTone, modelIdFrom, parseCandidate, workflowState } from "./meshParse";

function row(partial: Partial<OverlayRow>): OverlayRow {
  return { id: "r", label: "", detail: "", selected: false, group: null, ...partial };
}

describe("workflowState (overlay:workflow glyph → state)", () => {
  it("maps the leading glyph to a phase/agent state", () => {
    expect(workflowState("✓ done thing")).toBe("done");
    expect(workflowState("✗ failed thing")).toBe("failed");
    expect(workflowState("◐ running thing")).toBe("running");
    expect(workflowState("  queued thing")).toBe("pending");
  });
});

describe("modelIdFrom (mesh candidate label)", () => {
  it("strips a rank prefix, keeping the model id", () => {
    expect(modelIdFrom("#1 claude-opus-4-8")).toBe("claude-opus-4-8");
    expect(modelIdFrom("codex-cli::gpt-5.5")).toBe("codex-cli::gpt-5.5");
  });
});

describe("badgeTone (mesh candidate badge → semantic tone)", () => {
  it("maps the real wire badges to tones (unusable/benched = danger)", () => {
    expect(badgeTone("benched")).toBe("danger");
    expect(badgeTone("unusable")).toBe("danger");
    expect(badgeTone("frontier")).toBe("accent");
    expect(badgeTone("complex")).toBe("warn");
    expect(badgeTone("subscription")).toBe("accent");
    expect(badgeTone("free")).toBe("success");
    expect(badgeTone("whatever")).toBe("neutral");
  });
});

describe("parseCandidate (real overlay:mesh detail: `score N.NN · <cost> · frontier · …`)", () => {
  it("splits the score bar and capability badges out of a winner row", () => {
    const c = parseCandidate(row({ label: "#1 claude-opus-4-8", detail: "score 0.82 · free · frontier" }));
    expect(c.id).toBe("claude-opus-4-8");
    expect(c.scores.map((s) => s.value)).toEqual([0.82]);
    expect(c.badges).toEqual(expect.arrayContaining(["free", "frontier"]));
    expect(c.benched).toBe(false);
  });
  it("flags an `unusable` candidate as benched (danger reject styling)", () => {
    const c = parseCandidate(row({ label: "#7 groq::llama-3.3-70b", detail: "score 0.40 · paid · unusable" }));
    expect(c.badges).toContain("unusable");
    expect(c.benched).toBe(true);
  });
  it("keeps a non-badge token (`← routed pick`) as free-form reason text", () => {
    const c = parseCandidate(row({ label: "#1 m", detail: "score 0.90 · free · ← routed pick" }));
    expect(c.scores.map((s) => s.value)).toEqual([0.9]);
    expect(c.reason).toContain("routed pick");
  });
});
