import React from "react";
// @ts-expect-error -- react-dom ships no type declarations in this app (react-native-web pulls it in)
import * as server from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { ReleaseHero } from "./ReleaseHero";

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
  useTokens: () => ({ ink: "#fff", accent: "#ff8a3d" }),
}));

describe("ReleaseHero", () => {
  it("renders nothing for a release without a headline or chips", () => {
    expect(renderToStaticMarkup(<ReleaseHero release={{}} />)).toBe("");
    expect(renderToStaticMarkup(<ReleaseHero release={{ chips: [] }} />)).toBe("");
  });

  it("shows the headline and one pill per chip", () => {
    const html = renderToStaticMarkup(<ReleaseHero release={{ headline: "Threads that remember", chips: ["Gallery", "Palette"] }} />);
    expect(html).toContain("Threads that remember");
    expect(html).toContain("Gallery");
    expect(html).toContain("Palette");
  });

  it("renders chips alone when there is no headline", () => {
    expect(renderToStaticMarkup(<ReleaseHero release={{ chips: ["Goals"] }} />)).toContain("Goals");
  });
});
