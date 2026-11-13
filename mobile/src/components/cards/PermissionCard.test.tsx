import React from "react";
// @ts-expect-error -- react-dom ships no type declarations in this app (react-native-web pulls it in)
import * as server from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { PermissionCard } from "./PermissionCard";

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
    Pressable: box("button"),
    useWindowDimensions: () => ({ width: 400, height: 800 }),
  };
});
vi.mock("react-native-reanimated", () => ({
  default: { View: (props: Props) => React.createElement("b", null, props.children) },
  FadeOut: { duration: () => undefined },
  useAnimatedStyle: () => ({}),
  useReducedMotion: () => true,
  withTiming: (v: number) => v,
}));
vi.mock("../ds/Button", () => ({
  Button: (p: Props) => React.createElement("button", { "data-testid": p.testID as string | undefined }, p.label as string),
}));
vi.mock("../ds/Card", () => ({ Card: (p: Props) => React.createElement("section", null, p.children) }));
vi.mock("../ds/CommitIcon", () => ({ CommitIcon: () => null }));
vi.mock("../ds/Input", () => ({ Input: () => null }));
vi.mock("../ds/ToastHost", () => ({ useToast: () => ({ show: () => undefined }) }));
vi.mock("../review/DiffCard", () => ({ DiffCard: () => null }));
vi.mock("../../lib/haptics", () => ({ haptics: { allow: () => undefined, deny: () => undefined, mergeConflict: () => undefined } }));
vi.mock("../../lib/shortcuts", () => ({ useHotkey: () => undefined }));
vi.mock("../../theme/motion", () => ({ durations: { gentle: 0 }, easings: { standard: undefined } }));
vi.mock("../../theme/ThemeProvider", () => ({
  useTokens: () => new Proxy({}, { get: () => "#000" }),
}));

const render = (suggestion?: { rule: string; allows: number } | null) =>
  renderToStaticMarkup(
    React.createElement(PermissionCard, {
      prompt: "allow shell (Shell): cargo test -p forge-core [y/n]",
      diff: null,
      promptSeq: 9,
      suggestion,
      send: () => true,
    }),
  );

describe("PermissionCard rule suggestion", () => {
  it("offers the exact rule as a secondary button beside Allow and Deny", () => {
    const html = render({ rule: "cargo test *", allows: 3 });
    expect(html).toContain('data-testid="permission-accept-rule"');
    expect(html).toContain("Always allow cargo test *");
    expect(html).toContain("You&#x27;ve allowed cargo test * 3× here");
    expect(html).toContain(">Allow<");
    expect(html).toContain(">Deny<");
  });

  it("shows nothing extra without a suggestion", () => {
    for (const none of [undefined, null]) {
      const html = render(none);
      expect(html).not.toContain("permission-accept-rule");
      expect(html).not.toContain("Always allow");
    }
  });
});
