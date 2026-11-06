import React from "react";
// @ts-expect-error -- react-dom ships no type declarations in this app (react-native-web pulls it in)
import * as server from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { ProjectOfferCard } from "./ProjectOfferCard";

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
vi.mock("lucide-react-native", () => ({ GitBranch: () => null }));
vi.mock("../ds/Button", () => ({
  Button: (p: Props) => React.createElement("button", { "data-testid": p.testID as string }, p.label as string),
}));
vi.mock("../ds/Card", () => ({ Card: (p: Props) => React.createElement("section", null, p.children) }));
vi.mock("../ds/ToastHost", () => ({ useToast: () => ({ show: () => undefined }) }));
vi.mock("../../theme/ThemeProvider", () => ({
  useTokens: () => new Proxy({}, { get: () => "#000" }),
}));

describe("ProjectOfferCard", () => {
  const html = renderToStaticMarkup(
    React.createElement(ProjectOfferCard, {
      suggestion: { path: "/home/u/code/forge", name: "forge", calls: 3 },
      send: () => true,
    }),
  );

  it("names the repo and offers both answers", () => {
    expect(html).toContain("This session is working in forge. Move it there?");
    expect(html).toContain("Move there");
    expect(html).toContain("Not now");
  });
});
