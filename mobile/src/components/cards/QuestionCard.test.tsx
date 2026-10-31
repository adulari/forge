import React from "react";
// @ts-expect-error -- react-dom ships no type declarations in this app (react-native-web pulls it in)
import * as server from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { FormQuestion } from "../../lib/ws";
import { QuestionCard } from "./QuestionCard";

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
  };
});
vi.mock("react-native-reanimated", () => ({ default: { View: (props: Props) => React.createElement("b", null, props.children) } }));
vi.mock("lucide-react-native", () => {
  const icon = () => () => null;
  return { Check: icon(), ChevronLeft: icon(), ChevronRight: icon(), Send: icon() };
});
vi.mock("../ds/Button", () => ({ Button: (p: Props) => React.createElement("button", null, p.label as string) }));
vi.mock("../ds/Card", () => ({ Card: (p: Props) => React.createElement("section", null, p.children) }));
vi.mock("../ds/IconButton", () => ({ IconButton: () => null }));
vi.mock("../ds/Input", () => ({
  Input: (p: Props) =>
    React.createElement("input", {
      type: p.secureTextEntry ? "password" : "text",
      "data-label": p.label as string | undefined,
      "data-clearable": String(p.clearable !== false),
      "data-autocorrect": String(p.autoCorrect),
      "data-spellcheck": String(p.spellCheck),
      placeholder: p.placeholder as string,
      "aria-label": p.accessibilityLabel as string,
      value: p.value as string,
      readOnly: true,
    }),
}));
vi.mock("../ds/ToastHost", () => ({ useToast: () => ({ show: () => undefined }) }));
vi.mock("../../lib/haptics", () => ({ haptics: { select: () => undefined, mergeConflict: () => undefined } }));
vi.mock("../../theme/motion", () => ({ useStrike: () => ({ style: {}, onPressIn: () => undefined, onPressOut: () => undefined }) }));
vi.mock("../../theme/ThemeProvider", () => ({
  useTokens: () => new Proxy({}, { get: () => "#000" }),
}));

const form = (secret?: boolean): FormQuestion[] => [
  { header: "", text: "Enter the API token", options: [], multi: false, allow_other: true, allow_note: false, secret },
];
const render = (f: FormQuestion[]) =>
  renderToStaticMarkup(
    React.createElement(QuestionCard, { question: "", options: [], allowOther: true, form: f, promptSeq: 1, send: () => true }),
  );

describe("QuestionCard secret questions", () => {
  it("renders a masked input with no history or autocorrect", () => {
    const html = render(form(true));
    expect(html).toContain('type="password"');
    expect(html).toContain('data-label="Value"');
    expect(html).toContain('placeholder="Hidden as you type"');
    expect(html).toContain('data-clearable="false"');
    expect(html).toContain('data-autocorrect="false"');
    expect(html).toContain('data-spellcheck="false"');
    expect(html).toContain('aria-label="secret answer, hidden"');
  });

  it("renders an ordinary question as plain text", () => {
    const html = render(form(false));
    expect(html).toContain('type="text"');
    expect(html).not.toContain("Hidden as you type");
  });
});
