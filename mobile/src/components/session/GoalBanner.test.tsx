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
