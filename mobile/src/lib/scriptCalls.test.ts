import { describe, expect, it } from "vitest";

import { scriptCalls, scriptHeadline, scriptSource } from "./scriptCalls";
import { callHeadline, callVerb, readableArgs } from "./toolLabels";
import { summarize, summaryText } from "./toolGroups";

const script = (code: string) => JSON.stringify({ code });
const footer = (n: number, failed = 0) => `done\n\n[run_code: ${n} tool calls, ${n - failed} ok, ${failed} failed, 1.2s]`;

describe("scriptCalls", () => {
  it("reads the tools, literal shell commands and the result footer", () => {
    const s = scriptCalls(script('await tools.shell({command: "cargo test"});\nawait tools.read_file({path: "a"})'), footer(2));
    expect(s).toEqual({ total: 2, failed: 0, tools: ["shell", "read_file"], commands: ["cargo test"] });
  });

  it("reads a store-capped script whose JSON and command string were cut off", () => {
    const args = '{"code": "const r = await tools.shell({command: \\"sleep 280; cd /home/u/vm-ab …';
    expect(scriptSource(args)).toContain("tools.shell");
    expect(scriptCalls(args, null).commands[0]).toMatch(/^sleep 280; cd \/home\/u\/vm-ab/);
  });

  it("headlines a shell script by its command and a mixed one by its verbs", () => {
    expect(scriptHeadline(script('tools.shell({command: "ls"}); tools.shell({command: "pwd"})'), null)).toBe("ls (+1 more)");
    expect(scriptHeadline(script("tools.edit_file({}); tools.shell({command: x})"), footer(8))).toBe("edit, run · 8 calls");
  });
});

describe("script summaries", () => {
  it("counts a script that only ran shell commands by those commands, as the TUI does", () => {
    const calls = [1, 2, 3].map(() => ({ name: "run_code", args: script("for (const c of cs) await tools.shell({command: c})"), status: "ok" as const, result: footer(3) }));
    expect(summaryText(summarize(calls))).toBe("Ran 9 shell commands");
  });

  it("counts mixed scripts by their calls and keeps footerless ones as scripts", () => {
    const mixed = { name: "run_code", args: script("tools.read_file({}); tools.shell({})"), status: "ok" as const, result: footer(5) };
    expect(summaryText(summarize([mixed, mixed]))).toBe("Ran 10 tool calls in 2 scripts");
    expect(summaryText(summarize([{ name: "run_code", args: script("1"), status: "ok" }]))).toBe("Ran 1 script");
  });
});

describe("tool labels", () => {
  it("names a call by verb and subject, never the bare tool name", () => {
    expect(callVerb("run_code")).toBe("Script");
    expect(callVerb("mcp__forge__shell")).toBe("Run");
    expect(callHeadline("shell", '{"command":"cargo test"}', null)).toBe("cargo test");
    expect(callHeadline("run_code", script('tools.shell({command: "make"})'), footer(1))).toBe("make");
  });

  it("decodes a script's arguments into commands and source instead of escaped JSON", () => {
    const sections = readableArgs("run_code", script('// probe\nconst r = await tools.shell({command: "ls -la"});\nreturn r;'));
    expect(sections.map((s) => s.label)).toEqual(["commands", "script"]);
    expect(sections[0].text).toBe("$ ls -la");
    expect(sections[1].text).toContain('tools.shell({command: "ls -la"})');
    expect(readableArgs("read_file", '{"offset":1,"path":"a.rs"}')[0].text).toBe("path: a.rs\noffset: 1");
  });
});
