import React from "react";
// @ts-expect-error -- react-dom ships no type declarations in this app (react-native-web pulls it in)
import * as server from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { SnapshotLiveTail } from "../../lib/ws";
import { LiveToolTail, liveTailRows } from "./LiveToolTail";

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
vi.mock("./LiveTurn", () => ({ formatElapsed: (ms: number) => `${Math.floor(ms / 1000)}s` }));
vi.mock("../../theme/ThemeProvider", () => ({
  useTokens: () => ({ ink3: "ink3", ink4: "ink4", bg0: "bg0", border: "border" }),
}));

const tail = (lines: string[], elapsed_ms = 12_400): SnapshotLiveTail => ({ tool: "shell", lines, elapsed_ms });

describe("liveTailRows", () => {
  it("keeps the last ten non-blank lines and cuts long ones", () => {
    const lines = Array.from({ length: 14 }, (_, i) => `line ${i}`);
    const rows = liveTailRows(["", ...lines, "   ", "x".repeat(500)]);
    expect(rows).toHaveLength(10);
    expect(rows[8]).toBe("line 13");
    expect(rows[9]).toHaveLength(200);
    expect(rows[9].endsWith("…")).toBe(true);
  });
});

describe("LiveToolTail", () => {
  it("shows the elapsed time and each tail line", () => {
    const html = renderToStaticMarkup(React.createElement(LiveToolTail, { tail: tail(["running 4 tests", "test result: FAILED"]) }));
    expect(html).toContain("output · 12s");
    expect(html).toContain("running 4 tests");
    expect(html).toContain("test result: FAILED");
  });

  it("renders nothing while the command has printed nothing", () => {
    expect(renderToStaticMarkup(React.createElement(LiveToolTail, { tail: tail([]) }))).toBe("");
  });
});
