import React from "react";
// @ts-expect-error -- react-dom ships no type declarations in this app (react-native-web pulls it in)
import * as server from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { CiPlanResponse } from "../../lib/apiCi";
import { LocalCiSheet } from "./LocalCiSheet";

const renderToStaticMarkup = (server as { renderToStaticMarkup: (el: React.ReactElement) => string }).renderToStaticMarkup;

type Props = Record<string, unknown> & { children?: React.ReactNode };
const state = vi.hoisted(() => ({
  plan: undefined as unknown,
  pending: false,
  error: null as Error | null,
}));
const mutate = vi.hoisted(() => vi.fn());

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
    ScrollView: box("div"),
    Pressable: box("button"),
  };
});
vi.mock("../../lib/queriesCi", () => ({
  useSessionCi: () => ({ data: state.plan, error: state.error }),
  useRunSessionCi: () => ({ mutate, isPending: state.pending, error: null }),
}));
vi.mock("../../theme/ThemeProvider", () => ({
  useTokens: () => ({ ink: "ink", ink2: "ink2", ink3: "ink3", accent: "accent", danger: "danger", border: "border" }),
}));
vi.mock("../ds/Badge", () => ({ Badge: (props: { label: string }) => React.createElement("i", null, props.label) }));
vi.mock("../ds/Button", () => ({
  Button: (props: { label: string; disabled?: boolean }) =>
    React.createElement("button", { disabled: props.disabled, "data-run": "1" }, props.label),
}));
vi.mock("../ds/Sheet", () => ({ Sheet: (props: Props) => React.createElement("section", null, props.children) }));

const plan: CiPlanResponse = {
  rows: [
    { state: "pass", key: "ci.yml::lint::fmt", job: "lint", name: "lint / fmt", detail: "cargo fmt --check", command: "cargo fmt --check", secs: 2 },
    {
      state: "fail",
      key: "ci.yml::lint::clippy",
      job: "lint",
      name: "lint / clippy",
      detail: "cargo clippy",
      command: "cargo clippy --all-targets",
      env: ["RUSTFLAGS=-D warnings"],
      tail: "error: unused variable `x`",
      secs: 41,
    },
    { state: "skip", name: "win / build", detail: "runs on windows-latest (not Linux)", secs: 0 },
  ],
  summary: "Last run 3 min ago: 1 passed, 1 failed, 0 not run",
  noops: 1,
  gate: "auto",
  ok: false,
};

const render = () => renderToStaticMarkup(React.createElement(LocalCiSheet, { visible: true, onClose: () => {}, sessionId: "s1" }));

beforeEach(() => {
  mutate.mockReset();
  state.plan = plan;
  state.pending = false;
  state.error = null;
});

describe("LocalCiSheet", () => {
  it("lists the steps, the failing step's command, env and output tail, and the skip reasons", () => {
    const html = render();
    expect(html).toContain("1 of 2 steps failed.");
    expect(html).toContain("lint / fmt");
    expect(html).toContain("cargo clippy --all-targets");
    expect(html).toContain("RUSTFLAGS=-D warnings");
    expect(html).toContain("error: unused variable `x`");
    expect(html).toContain("fail · 41s");
    expect(html).toContain("win / build");
    expect(html).toContain("runs on windows-latest (not Linux)");
    expect(html).toContain("Goals finish and pushes happen only after this plan passes.");
  });

  it("keeps a passing step collapsed", () => {
    // The command is also the row's one-line detail, so an expanded row would print it twice.
    expect(render().split("cargo fmt --check").length - 1).toBe(1);
  });

  it("disables Run and says so while a run is going", () => {
    state.pending = true;
    const html = render();
    expect(html).toContain("Running the plan on the host");
    expect(html).toContain("Running…");
    expect(html).toContain("disabled");
  });

  it("offers no Run when nothing in the workflows can run, and says why", () => {
    state.plan = { ...plan, rows: [plan.rows[2]], summary: null, ok: undefined };
    const html = render();
    expect(html).toContain("Nothing in the workflows can run here");
    expect(html).toContain("disabled");
  });

  it("shows a host error instead of a stale plan headline", () => {
    state.plan = undefined;
    state.error = new Error("this host does not know /ci");
    expect(render()).toContain("this host does not know /ci");
  });
});
