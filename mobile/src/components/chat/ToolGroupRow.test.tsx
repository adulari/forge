import React from "react";
// @ts-expect-error -- react-dom ships no type declarations in this app (react-native-web pulls it in)
import * as server from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { foldToolGroups, type ToolGroupEntry } from "../../lib/toolGroups";
import type { TranscriptEntry } from "../../lib/toolRows";
import { groupAccessibilityLabel, ToolGroupRow } from "./ToolGroupRow";

const renderToStaticMarkup = (server as { renderToStaticMarkup: (el: React.ReactElement) => string }).renderToStaticMarkup;

type Props = Record<string, unknown> & { children?: React.ReactNode };
const pressables = vi.hoisted(() => [] as Record<string, unknown>[]);

vi.mock("react-native", () => {
  const box = (tag: string) => {
    const Box = (props: Props) => React.createElement(tag, null, props.children);
    Box.displayName = tag;
    return Box;
  };
  return {
    ActivityIndicator: () => React.createElement("i", null, "spinner"),
    Platform: { OS: "web", select: (o: Record<string, unknown>) => o.web ?? o.default },
    Pressable: (props: Props) => {
      pressables.push(props);
      const children = typeof props.children === "function" ? null : props.children;
      return React.createElement("button", null, children);
    },
    StyleSheet: { create: (s: unknown) => s, hairlineWidth: 1 },
    Text: box("span"),
    View: box("div"),
  };
});
vi.mock("lucide-react-native", () => ({
  ChevronDown: () => React.createElement("i", null, "open"),
  ChevronRight: () => React.createElement("i", null, "closed"),
}));
vi.mock("../../lib/haptics", () => ({ haptics: { select: vi.fn() } }));
vi.mock("../../theme/ThemeProvider", () => ({
  useTokens: () => ({ ink3: "ink3", danger: "danger", bg2: "bg2" }),
}));
vi.mock("./ToolCallRow", () => ({
  ToolCallRow: ({ invocation }: { invocation: { args: string } }) => React.createElement("card", null, invocation.args),
  ChangeStat: ({ adds, dels }: { adds: number; dels: number }) => React.createElement("stat", null, `+${adds} -${dels}`),
}));

let seq = 0;
const tool = (name: string, args: string, result: string | null): TranscriptEntry => {
  seq += 1;
  return { kind: "tool", key: `t${seq}`, invocation: { seq, tool: name, args, result } };
};
// Newest first, as the chat list holds them.
const group = (entries: TranscriptEntry[], live = false) => foldToolGroups([...entries].reverse(), live)[0] as ToolGroupEntry;
const render = (g: ToolGroupEntry, expanded: boolean | undefined, onToggle = vi.fn()) =>
  renderToStaticMarkup(React.createElement(ToolGroupRow, { group: g, expanded, onToggle }));

beforeEach(() => {
  pressables.length = 0;
});

describe("ToolGroupRow", () => {
  const calls = [
    tool("read_file", '{"path":"a.rs"}', "fn a()"),
    tool("shell", '{"command":"exit 3"}', "error: exit 3"),
    tool("mcp__forge__shell", '{"command":"ls"}', "a.rs"),
  ];

  it("collapses to the summary with the failure listed beneath, marked once", () => {
    const html = render(group(calls), false);
    expect(html).toContain("Read 1 file, ran 1 shell command");
    // The failed call's own row says so; the summary does not repeat it.
    expect(html).not.toContain("1 failed");
    expect(html).toContain("closed");
    expect(html.match(/<card>/g)).toHaveLength(1);
    expect(html).toContain("exit 3");
    const row = pressables[0];
    expect(row.accessibilityRole).toBe("button");
    expect(row.accessibilityState).toEqual({ expanded: false });
    expect(row.accessibilityLabel).toBe("Read 1 file, ran 1 shell command, 1 failed");
    expect(row.focusable).toBe(true);
  });

  it("opens in place to every call card, unchanged", () => {
    const html = render(group(calls), true);
    expect(html).toContain("open");
    expect(html.match(/<card>/g)).toHaveLength(3);
    expect(pressables[0].accessibilityState).toEqual({ expanded: true });
  });

  it("toggles by group id on press", () => {
    const onToggle = vi.fn();
    const g = group(calls);
    render(g, false, onToggle);
    (pressables[0].onPress as () => void)();
    expect(onToggle).toHaveBeenCalledWith(g.key, true);
  });

  it("spins while a call runs and shows the call in flight under it", () => {
    const g = group([tool("read_file", '{"path":"a.rs"}', "ok"), tool("shell", '{"command":"cargo test"}', null)], true);
    const html = render(g, false);
    expect(html).toContain("spinner");
    expect(html).toContain("Read a.rs");
    expect(html).toContain("cargo test");
    expect(groupAccessibilityLabel(g)).toBe("Read a.rs, 1 running");
  });

  it("opens on its own while a call runs and folds once the run is done", () => {
    const running = group([tool("read_file", '{"path":"a.rs"}', "ok"), tool("read_file", '{"path":"b.rs"}', "ok"), tool("shell", '{"command":"cargo test"}', null)], true);
    expect(render(running, undefined).match(/<card>/g)).toHaveLength(3);
    expect(pressables[0].accessibilityState).toEqual({ expanded: true });
    pressables.length = 0;
    const done = group([tool("read_file", '{"path":"a.rs"}', "ok"), tool("read_file", '{"path":"b.rs"}', "ok")]);
    expect(render(done, undefined)).not.toContain("<card>");
    expect(pressables[0].accessibilityState).toEqual({ expanded: false });
  });

  it("keeps a running group closed when the person closed it", () => {
    const running = group([tool("read_file", '{"path":"a.rs"}', "ok"), tool("read_file", '{"path":"b.rs"}', "ok"), tool("shell", '{"command":"cargo test"}', null)], true);
    const onToggle = vi.fn();
    render(running, false, onToggle);
    expect(pressables[0].accessibilityState).toEqual({ expanded: false });
    (pressables[0].onPress as () => void)();
    expect(onToggle).toHaveBeenCalledWith(running.key, true);
  });

  it("folds repeated edits to one file into one row with the combined counter", () => {
    const edit = (path: string, adds: number, dels: number): TranscriptEntry => {
      const e = tool("edit_file", JSON.stringify({ path }), "ok");
      if (e.kind === "tool") Object.assign(e.invocation, { adds, dels });
      return e;
    };
    const g = group([edit("src/a.ts", 3, 1), edit("src/a.ts", 2, 0), edit("src/a.ts", 1, 1), tool("read_file", '{"path":"b.rs"}', "ok")]);
    const html = render(g, true);
    expect(html).toContain("×3");
    expect(html).toContain("+6 -2");
    expect(html.match(/<card>/g)).toHaveLength(1);
  });

  it("shows the first call alone until it finishes", () => {
    const html = render(group([tool("shell", '{"command":"cargo test"}', null)], true), false);
    expect(pressables).toHaveLength(0);
    expect(html.match(/<card>/g)).toHaveLength(1);
  });

  it("says what a group tried when nothing succeeded", () => {
    const html = render(group([tool("edit_file", '{"path":"sample.rs"}', "error: old_string not found")]), false);
    expect(html).toContain("Tried to edit sample.rs");
    expect(html).toContain("1 failed");
  });
});
