import React from "react";
// @ts-expect-error -- react-dom ships no type declarations in this app (react-native-web pulls it in)
import * as server from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { ExpertLine } from "./ExpertLine";

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
vi.mock("../../theme/ThemeProvider", () => ({
  useTokens: () => ({ ink3: "ink3", ink4: "ink4" }),
}));

describe("ExpertLine", () => {
  it("shows the host's expert line as is", () => {
    const html = renderToStaticMarkup(
      React.createElement(ExpertLine, { text: "expert: codex-oauth::gpt-5.6-sol · 1 consult · 2.1% of weekly window" }),
    );
    expect(html).toContain("expert: codex-oauth::gpt-5.6-sol · 1 consult · 2.1% of weekly window");
  });
});
