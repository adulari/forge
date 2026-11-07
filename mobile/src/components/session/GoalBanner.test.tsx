import React from "react";
// @ts-expect-error -- react-dom ships no type declarations in this app (react-native-web pulls it in)
import * as server from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { RemoteInput, Snapshot, SnapshotGoal } from "../../lib/ws";
import { GoalBanner } from "./GoalBanner";

const renderToStaticMarkup = (server as { renderToStaticMarkup: (el: React.ReactElement) => string }).renderToStaticMarkup;

type Props = Record<string, unknown> & { children?: React.ReactNode };
const buttons = vi.hoisted(() => new Map<string, () => void>());

vi.mock("react-native", () => {
  const box = (tag: string) => {
    const Box = (props: Props) => React.createElement(tag, null, props.children);
    Box.displayName = tag;
    return Box;
  };
  return {
    Platform: { OS: "web", select: (o: Record<string, unknown>) => o.web ?? o.default },
    StyleSheet: { create: (s: unknown) => s, hairlineWidth: 1 },
    Text: box("span"),
    View: box("div"),
  };
});
vi.mock("lucide-react-native", () => {
  const icon = (name: string) => {
    const Icon = () => React.createElement("i", null, name);
    Icon.displayName = name;
    return Icon;
  };
  return { Pause: icon("Pause"), Pencil: icon("Pencil"), Play: icon("Play"), Square: icon("Square"), X: icon("X") };
});
vi.mock("../../lib/haptics", () => ({ haptics: { select: vi.fn(), deny: vi.fn() } }));
vi.mock("../../theme/ThemeProvider", () => ({
  useTokens: () => ({ ink: "ink", ink2: "ink2", ink3: "ink3", accent: "accent", danger: "danger", success: "success", border: "border", bg3: "bg3" }),
}));
vi.mock("../chat/MiniPressable", () => ({
  MiniPressable: (props: Props & { testID?: string; onPress?: () => void }) => {
    if (props.testID && props.onPress) buttons.set(props.testID, props.onPress);
    return React.createElement("button", { id: props.testID }, props.children);
  },
}));
vi.mock("../chat/GoalSheet", () => ({ GoalSheet: () => null }));
vi.mock("../ds/Card", () => ({ Card: (props: Props) => React.createElement("section", null, props.children) }));
vi.mock("../ds/StatusDot", () => ({ StatusDot: () => null }));

const goal = (over: Partial<SnapshotGoal> = {}): SnapshotGoal => ({
  objective: "ship the parser",
  state: "active",
  iteration: 2,
  tokens_used: 4_000,
  max_tokens: 10_000,
  max_minutes: null,
  elapsed_secs: 65,
  synced_at: 0,
  gates: [],
  reason: null,
  pending: null,
  ...over,
});
const snapshot = (over: Partial<Snapshot>): Snapshot => ({ notes: [], ...over }) as Snapshot;

const render = (snap: Snapshot | null, send = vi.fn<(input: RemoteInput) => boolean>(() => true)) => ({
  html: renderToStaticMarkup(React.createElement(GoalBanner, { snapshot: snap, send })),
  send,
});

beforeEach(() => buttons.clear());

describe("GoalBanner", () => {
  it("renders nothing without a goal", () => {
    expect(render(snapshot({})).html).toBe("");
    expect(render(null).html).toBe("");
  });

  it("shows a running goal with its budget and offers pause, edit and stop", () => {
    const { html } = render(snapshot({ goal: goal() }));
    expect(html).toContain("Goal · active");
    expect(html).toContain("ship the parser");
    expect(html).toContain("turn 2 · 4k / 10k tok");
    expect(buttons.has("goal-pause") && buttons.has("goal-edit") && buttons.has("goal-stop")).toBe(true);
    expect(buttons.has("goal-resume")).toBe(false);
  });

  it("offers resume, edit and clear when paused, and sends the verbs the host understands", () => {
    const { html, send } = render(snapshot({ goal: goal({ state: "paused" }) }));
    expect(html).toContain("Goal · paused");
    expect([...buttons.keys()]).toEqual(["goal-resume", "goal-edit", "goal-clear"]);
    buttons.get("goal-resume")?.();
    buttons.get("goal-clear")?.();
    expect(send.mock.calls.map(([input]) => input)).toEqual([
      { kind: "prompt", text: "/goal resume" },
      { kind: "prompt", text: "/goal clear" },
    ]);
  });

  it("draws the spend envelope with its meter line", () => {
    const envelope = { unit: "percent" as const, used: 12, slice: 20, label: "claude weekly", meter: "envelope 12/20% claude weekly", spent: false, unverified: false, resets_at: null };
    const { html } = render(snapshot({ goal: goal({ envelope }) }));
    expect(html).toContain("envelope 12/20% claude weekly");
    expect(html).not.toContain("unverified");
  });

  it("shows why a goal paused at its envelope cap and offers resume", () => {
    const envelope = { unit: "percent" as const, used: 20, slice: 20, label: "claude weekly", meter: "envelope 20/20% claude weekly", spent: true, unverified: false, resets_at: 1_800_000_000 };
    const reason = "Envelope spent (20% of Claude weekly). The window resets at Thu 09:00, then /goal resume";
    const { html } = render(snapshot({ goal: goal({ state: "paused", reason, envelope }) }));
    expect(html).toContain("Envelope spent (20% of Claude weekly)");
    expect(html).toContain("envelope 20/20% claude weekly");
    expect(buttons.has("goal-resume")).toBe(true);
  });

  it("draws the contract table with a mark, the check and the proof per assertion", () => {
    const contract = {
      headline: "1/2 pass",
      approved: true,
      rounds: 1,
      passed: 1,
      total: 2,
      checked_at: 1_700_000_000,
      assertions: [
        { id: 1, text: "a exists", command: "test -f a.txt", expect: "", state: "pass" as const, reason: "", proof: "" },
        { id: 2, text: "b says ready", command: "cat b.txt", expect: "", state: "fail" as const, reason: "exit 1", proof: "cat: b.txt: No such file" },
      ],
    };
    const { html } = render(snapshot({ goal: goal({ contract }) }));
    expect(html).toContain("Contract · 1/2 pass");
    expect(html).toContain("1. a exists");
    expect(html).toContain("2. b says ready");
    expect(html).toContain("cat b.txt");
    expect(html).toContain("exit 1 · cat: b.txt: No such file");
    expect(html).toContain("✓");
    expect(html).toContain("✗");
  });

  it("groups a mission's assertions under their milestone with the progress line", () => {
    const row = (id: number, state: "pass" | "fail" | "pending") => ({
      id,
      text: `fact ${id}`,
      command: `test -f f${id}`,
      expect: "",
      state,
      reason: state === "fail" ? "exit 1" : "",
      proof: "",
    });
    const contract = {
      headline: "milestone 2/3: 1/2 pass",
      approved: true,
      rounds: 2,
      passed: 3,
      total: 6,
      checked_at: 1_700_000_000,
      assertions: [row(1, "pass"), row(2, "pass"), row(3, "pass"), row(4, "fail"), row(5, "pending"), row(6, "pending")],
      mission: {
        current: 2,
        total: 3,
        progress: "milestone 2/3: 1/2 pass",
        milestones: [
          { index: 1, title: "lexer", state: "done" as const, passed: 2, total: 2, assertion_ids: [1, 2] },
          { index: 2, title: "parser", state: "active" as const, passed: 1, total: 2, assertion_ids: [3, 4] },
          { index: 3, title: "cli", state: "upcoming" as const, passed: 0, total: 2, assertion_ids: [5, 6] },
        ],
      },
    };
    const { html } = render(snapshot({ goal: goal({ contract }) }));
    expect(html).toContain("Mission · milestone 2/3: 1/2 pass");
    expect(html).toContain("Milestone 1 · lexer · done 2/2");
    expect(html).toContain("Milestone 2 · parser · current 1/2 pass");
    expect(html).toContain("Milestone 3 · cli · upcoming, 2 assertions");
    expect(html.indexOf("Milestone 2 ·")).toBeLessThan(html.indexOf("4. fact 4"));
    expect(html.indexOf("4. fact 4")).toBeLessThan(html.indexOf("Milestone 3 ·"));
  });

  it("omits the contract table for a goal without one", () => {
    expect(render(snapshot({ goal: goal() })).html).not.toContain("Contract");
  });

  it("omits the envelope row for a goal without one", () => {
    expect(render(snapshot({ goal: goal() })).html).not.toContain("envelope");
  });

  it("says a pause is pending and leaves only stop", () => {
    const { html } = render(snapshot({ goal: goal({ pending: "pause" }) }));
    expect(html).toContain("pausing after this turn");
    expect([...buttons.keys()]).toEqual(["goal-stop"]);
  });

  it("keeps an ended goal visible with why it stopped, and lets it be cleared", () => {
    const { html } = render(snapshot({ goal: goal({ state: "budget_limit", reason: "🎯 goal stopped — token budget exhausted" }) }));
    expect(html).toContain("Goal · budget limit");
    expect(html).toContain("token budget exhausted");
    expect([...buttons.keys()]).toEqual(["goal-clear"]);
  });

  it("falls back to the notes of an older host and never sends it a /goal verb", () => {
    const { html, send } = render(snapshot({ notes: ["🎯 goal set — fix the build"] }));
    expect(html).toContain("fix the build");
    expect([...buttons.keys()]).toEqual(["goal-stop"]);
    buttons.get("goal-stop")?.();
    expect(send).toHaveBeenCalledWith({ kind: "interrupt" });
  });
});
