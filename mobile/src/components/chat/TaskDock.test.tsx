import React from "react";
// @ts-expect-error -- react-dom ships no type declarations in this app (react-native-web pulls it in)
import * as server from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { SnapshotTask } from "../../lib/ws";
import { TaskDock } from "./TaskDock";

const renderToStaticMarkup = (server as { renderToStaticMarkup: (el: React.ReactElement) => string }).renderToStaticMarkup;

type Props = Record<string, unknown> & { children?: React.ReactNode };

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
vi.mock("react-native-reanimated", () => ({ default: { View: (props: Props) => React.createElement("b", null, props.children) } }));
vi.mock("lucide-react-native", () => {
  const icon = (name: string) => {
    const Icon = () => React.createElement("i", null, name);
    Icon.displayName = name;
    return Icon;
  };
  return { Check: icon("Check"), ListChecks: icon("ListChecks") };
});
vi.mock("../../theme/motion", () => ({ useEmberdot: () => ({ dotStyle: {} }) }));
vi.mock("../../theme/ThemeProvider", () => ({
  useTokens: () => ({ ink: "ink", ink2: "ink2", ink3: "ink3", ink4: "ink4", accent: "accent" }),
}));
vi.mock("./DockFrame", () => ({
  DockFrame: (props: Props & { title: string; meta?: string; headline?: string; collapsed: boolean }) =>
    React.createElement(
      "dock",
      null,
      `${props.title} ${props.meta ?? ""}|${props.collapsed ? `folded:${props.headline ?? ""}` : "open"}`,
      props.collapsed ? null : props.children,
    ),
}));

const t = (title: string, status: SnapshotTask["status"]): SnapshotTask => ({ title, status });
const render = (tasks: SnapshotTask[], collapsed = false) =>
  renderToStaticMarkup(React.createElement(TaskDock, { tasks, busy: true, collapsed, onToggle: vi.fn(), menu: [] }));

describe("TaskDock", () => {
  const plan = [t("read the parser", "done"), t("write the fix", "in_progress"), t("add tests", "pending")];

  it("renders nothing for an empty plan", () => {
    expect(render([])).toBe("");
  });

  it("heads the plan with how far along it is and lists every step", () => {
    const html = render(plan);
    expect(html).toContain("Plan 1/3|open");
    for (const step of plan) expect(html).toContain(step.title);
    expect(html.match(/<i>Check<\/i>/g)).toHaveLength(1);
    expect(html.indexOf("read the parser")).toBeLessThan(html.indexOf("write the fix"));
    expect(html.indexOf("write the fix")).toBeLessThan(html.indexOf("add tests"));
  });

  it("folds to the step being worked on", () => {
    const html = render(plan, true);
    expect(html).toContain("folded:write the fix");
    expect(html).not.toContain("add tests");
  });

  it("says when everything is done", () => {
    expect(render([t("a", "done"), t("b", "done")], true)).toContain("folded:all done");
  });

  it("windows a long plan around the active step and counts what it hides", () => {
    const long = Array.from({ length: 12 }, (_, i) => t(`step ${i}`, i < 5 ? "done" : i === 5 ? "in_progress" : "pending"));
    const html = render(long);
    expect(html).toContain("4 earlier");
    expect(html).toContain("2 more");
    expect(html).toContain("step 5");
    expect(html).not.toContain("step 0<");
  });
});
