import React from "react";
// @ts-expect-error -- react-dom ships no type declarations in this app (react-native-web pulls it in)
import * as server from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { Snapshot, SnapshotSubagent } from "../../lib/ws";
import SessionAgents from "../../app/session/[id]/agents";

const renderToStaticMarkup = (server as { renderToStaticMarkup: (el: React.ReactElement) => string }).renderToStaticMarkup;

type Props = Record<string, unknown> & { children?: React.ReactNode };
const ctx = vi.hoisted(() => ({ snapshot: null as Partial<Snapshot> | null }));

vi.mock("react-native", () => {
  const box = (tag: string) => {
    const Box = (props: Props) => React.createElement(tag, null, props.children);
    Box.displayName = tag;
    return Box;
  };
  return {
    Platform: { OS: "web", select: (o: Record<string, unknown>) => o.web ?? o.default },
    Pressable: box("button"),
    StyleSheet: { create: (s: unknown) => s, hairlineWidth: 1 },
    Text: box("span"),
    View: box("div"),
  };
});
vi.mock("lucide-react-native", () => ({
  Bot: () => null,
  Check: () => React.createElement("i", null, "check"),
  WifiOff: () => null,
}));
vi.mock("../../theme/ThemeProvider", () => ({ useTokens: () => ({}) }));
vi.mock("../ds/StatusDot", () => ({
  StatusDot: ({ accessibilityLabel }: { accessibilityLabel: string }) => React.createElement("i", null, accessibilityLabel),
}));
vi.mock("../ds/Button", () => ({ Button: () => null }));
vi.mock("../ds/Screen", () => ({ Screen: (props: Props) => React.createElement("main", null, props.children) }));
vi.mock("../ds/Skeleton", () => ({ Skeleton: () => React.createElement("i", null, "loading") }));
vi.mock("../ds/EmptyState", () => ({
  EmptyState: ({ message }: { message: string }) => React.createElement("p", null, message),
}));
vi.mock("../ds/BoundedList", () => ({
  BoundedList: ({
    data,
    renderItem,
    ListEmptyComponent,
  }: {
    data: SnapshotSubagent[];
    renderItem: (info: { item: SnapshotSubagent; index: number }) => React.ReactElement;
    ListEmptyComponent: React.ReactElement;
  }) =>
    React.createElement(
      "ul",
      null,
      data.length === 0 ? ListEmptyComponent : data.map((item, index) => React.createElement("li", { key: item.id }, renderItem({ item, index }))),
    ),
}));
vi.mock("../../lib/sessionContext", () => ({
  useSessionCtx: () => ({ snapshot: ctx.snapshot, snapshotTimedOut: false, send: () => true }),
}));

const agent = (over: Partial<SnapshotSubagent>): SnapshotSubagent => ({
  id: "6ddf5b5c-0000",
  agent: "plan",
  task: "draft the plan",
  model: "inferhub::cb/deepseek-v4.1-flash",
  phase: null,
  last: "",
  done: false,
  ok: true,
  cost: 0,
  permission_prompt: null,
  ...over,
});

const render = (subagents: SnapshotSubagent[]) => {
  ctx.snapshot = { subagents };
  return renderToStaticMarkup(React.createElement(SessionAgents));
};

describe("Session Agents tab", () => {
  it("says no active agents only when the snapshot has none", () => {
    expect(render([])).toContain("no active agents");
  });

  // Detached (background) agents ride in `snapshot.subagents` exactly like blocking ones, so the
  // tab lists them while they run after the turn that admitted them has ended.
  it("lists detached agents from the snapshot with role, model and live state", () => {
    const html = render([
      agent({ last: "reading the router" }),
      agent({ id: "e5cb1130-0000", agent: "explore", task: "map the crate" }),
    ]);
    expect(html).not.toContain("no active agents");
    expect(html).toContain("plan");
    expect(html).toContain("explore");
    expect(html).toContain("inferhub::cb/deepseek-v4.1-flash");
    expect(html).toContain("reading the router");
    expect(html).toContain("plan: running");
    expect(html).toContain("explore: running");
  });

  it("keeps a settled detached agent visible with its outcome and cost", () => {
    const html = render([
      agent({ done: true, ok: true, cost: 0.02 }),
      agent({ id: "e5cb1130-0000", agent: "explore", done: true, ok: false, last: "cancelled" }),
    ]);
    expect(html).toContain("plan: done");
    expect(html).toContain("check");
    expect(html).toContain("$0.02");
    expect(html).not.toContain("explore: running");
    expect(html).toContain("cancelled");
  });
});
