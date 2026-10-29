import { describe, expect, it } from "vitest";

import type { HistoryRow } from "./api";
import {
  collapsedCalls,
  foldToolGroups,
  GroupMemo,
  summarize,
  summaryText,
  type CallDiff,
  type GroupCall,
  type ToolGroupEntry,
} from "./toolGroups";
import type { TranscriptEntry } from "./toolRows";

const done = (name: string, args: string, diff?: CallDiff): GroupCall => ({ name, args, status: "ok", diff });
const failed = (name: string, args: string): GroupCall => ({ name, args, status: "failed" });
const text = (calls: GroupCall[]) => summaryText(summarize(calls));
const edit = (path: string, adds: number, dels: number) =>
  done("edit_file", JSON.stringify({ path }), { kind: "modified", path, adds, dels });

describe("summary text (ported from tool_groups_tests.rs)", () => {
  it("aggregates categories in order of first appearance with plurals", () => {
    expect(
      text([
        done("read_file", '{"path":"a.rs"}'),
        done("grep", '{"pattern":"fn main"}'),
        done("shell", '{"command":"ls"}'),
        done("read_file", '{"path":"b.rs"}'),
        done("glob", '{"pattern":"**/*.rs"}'),
        done("shell", '{"command":"pwd"}'),
        done("read_file", '{"path":"c.rs"}'),
        done("shell", '{"command":"date"}'),
        done("shell", '{"command":"id"}'),
      ]),
    ).toBe("Read 3 files, searched for 2 patterns, ran 4 shell commands");
  });

  it("is singular for one of a kind", () => {
    expect(text([done("list_dir", '{"path":"src"}'), done("shell", '{"command":"ls"}'), done("shell", '{"command":"pwd"}')])).toBe(
      "Listed 1 directory, ran 2 shell commands",
    );
    expect(text([done("list_dir", '{"path":"src"}'), done("list_dir", '{"path":"docs"}'), done("web_search", '{"query":"x"}')])).toBe(
      "Listed 2 directories, ran 1 web search",
    );
  });

  it("gives file changes their combined stat", () => {
    expect(text([edit("a.rs", 2, 1), edit("b.rs", 2, 1)])).toBe("Edited a.rs, b.rs (+4 −2)");
  });

  it("never folds a deletion into edited", () => {
    expect(
      text([
        edit("a.rs", 1, 1),
        done("write_file", '{"path":"new.rs"}', { kind: "created", path: "new.rs", adds: 1, dels: 0 }),
        done("delete_file", '{"path":"old.rs"}'),
      ]),
    ).toBe("Edited a.rs (+1 −1), created new.rs (+1), deleted old.rs");
  });

  it("names up to two changed files and counts beyond", () => {
    const three = ["a.rs", "b.rs", "c.rs"].map((f) => edit(f, 1, 1));
    expect(text(three)).toBe("Edited 3 files (+3 −3)");
    expect(text(three.slice(0, 2))).toBe("Edited a.rs, b.rs (+2 −2)");
  });

  it("names what a single call touched", () => {
    expect(text([done("shell", '{"command":"cargo test -p upload"}')])).toBe("Ran cargo test -p upload");
    expect(text([done("read_file", '{"path":"Cargo.toml"}')])).toBe("Read Cargo.toml");
    expect(text([done("write_file", '{"path":"notes.md"}', { kind: "created", path: "notes.md", adds: 2, dels: 0 })])).toBe(
      "Created notes.md (+2)",
    );
    expect(text([done("run_code", '{"code":"1"}')])).toBe("Ran 1 script");
  });

  it("gives agents, MCP, tasks and unknown tools their own words", () => {
    expect(
      text([
        done("spawn_agents", '{"agents":[{"task":"a"},{"task":"b"}]}'),
        done("update_tasks", '{"tasks":[]}'),
        done("mcp__github__list_issues", "{}"),
        done("mcp__github__get_issue", "{}"),
        done("device", '{"action":"tap"}'),
      ]),
    ).toBe("Ran 2 agents, updated the task list, called github 2 times, called device 1 time");
  });

  it("groups bridged mcp__forge__ calls by their bare tool name", () => {
    expect(
      text([
        done("mcp__forge__edit_file", '{"path":"calc.py"}'),
        done("mcp__forge__edit_file", '{"path":"calc.py"}'),
      ]),
    ).toBe("Edited calc.py");
    expect(text([done("mcp__forge__shell", '{"command":"ls"}'), done("mcp__forge__read_file", '{"path":"a"}')])).toBe(
      "Ran 1 shell command, read 1 file",
    );
  });

  it("counts failures and running calls without hiding them", () => {
    const s = summarize([
      done("shell", '{"command":"ls"}'),
      failed("shell", '{"command":"false"}'),
      { name: "shell", args: '{"command":"sleep 9"}', status: "running" },
    ]);
    expect([s.finished, s.failed, s.running]).toEqual([2, 1, 1]);
    expect(summaryText(s)).toBe("Ran ls");
  });

  it("counts chunked reads and repeated edits of one file once", () => {
    const read = (path: string, offset: number) => done("read_file", JSON.stringify({ path, offset }));
    expect(
      text([
        read("src/big.rs", 0),
        read("src/big.rs", 200),
        read("./src/big.rs", 400),
        done("shell", '{"command":"cargo check"}'),
        edit("src/big.rs", 1, 1),
        edit("src/big.rs", 2, 1),
        edit("src/big.rs", 1, 1),
      ]),
    ).toBe("Read 1 file, ran 1 shell command, edited src/big.rs (+4 −3)");
    expect(text([read("src/big.rs", 0), read("src/big.rs", 200)])).toBe("Read src/big.rs");
    expect(text([edit("a.rs", 1, 1), edit("a.rs", 1, 1)])).toBe("Edited a.rs (+2 −2)");
  });

  it("counts the same pattern searched twice as one", () => {
    expect(
      text([
        done("grep", '{"pattern":"fn main","path":"src"}'),
        done("grep", '{"pattern":"fn main","path":"tests"}'),
        done("list_dir", '{"path":"src"}'),
        done("list_dir", '{"path":"./src"}'),
      ]),
    ).toBe("Searched for 1 pattern, listed 1 directory");
  });

  it("never tallies a failed call as done", () => {
    const s = summarize([
      failed("edit_file", '{"path":"sample.rs","old":"x","new":"y"}'),
      done("write_file", '{"path":"notes.md","content":""}'),
    ]);
    expect(summaryText(s)).toBe("Edited notes.md");
    expect(s.failed).toBe(1);
  });

  it("adds nothing to the stat for a failed change", () => {
    const bad: GroupCall = { ...edit("b.rs", 2, 1), status: "failed" };
    expect(text([edit("a.rs", 1, 1), bad])).toBe("Edited a.rs (+1 −1)");
  });

  it("says what a group tried when nothing succeeded", () => {
    expect(text([failed("edit_file", '{"path":"sample.rs","old":"x","new":"y"}')])).toBe("Tried to edit sample.rs");
    expect(text([failed("shell", '{"command":"false"}'), failed("shell", '{"command":"exit 3"}')])).toBe(
      "Tried to run 2 shell commands",
    );
    expect(text([failed("read_file", '{"path":"gone.rs"}'), failed("shell", '{"command":"false"}')])).toBe(
      "Tried to read 1 file, run 1 shell command",
    );
  });

  it("reads a target out of args the store cut short", () => {
    expect(text([done("edit_file", '{"path":"src/a.rs","old_string":"very long…')])).toBe("Edited src/a.rs");
    expect(text([done("shell", '{"command":"cargo build","cwd":"/x…')])).toBe("Ran cargo build");
  });

  it("shows paths relative to the call's cwd and never the raw blob", () => {
    expect(text([done("read_file", '{"path":"/repo/src/a.rs","cwd":"/repo"}')])).toBe("Read src/a.rs");
    expect(text([done("shell", "{not json")])).toBe("Ran 1 shell command");
  });
});

let seq = 0;
const tool = (name: string, args: string, result: string | null): TranscriptEntry => {
  seq += 1;
  return { kind: "tool", key: `t${seq}`, invocation: { seq, tool: name, args, result } };
};
const message = (content: string): TranscriptEntry => {
  seq += 1;
  const row: HistoryRow = { seq, role: "assistant", content, model: null, created_at: 0, visibility: "llm" };
  return { kind: "message", key: `h${seq}`, row };
};
const groups = (entries: ReturnType<typeof foldToolGroups>) =>
  entries.filter((e): e is ToolGroupEntry => e.kind === "toolGroup");

describe("foldToolGroups", () => {
  it("folds each run of calls between prose into one group, oldest call first", () => {
    // Chronological: prose, read, shell, prose, edit — newest first below.
    const a = message("first");
    const r = tool("read_file", '{"path":"a.rs"}', "ok");
    const s = tool("shell", '{"command":"ls"}', "ok");
    const b = message("second");
    const e = tool("edit_file", '{"path":"a.rs"}', "edited");
    const folded = foldToolGroups([e, b, s, r, a], false);
    expect(folded.map((x) => x.kind)).toEqual(["toolGroup", "message", "toolGroup", "message"]);
    const [newest, older] = groups(folded);
    expect(older.calls.map((c) => c.key)).toEqual([r.key, s.key]);
    expect(older.key).toBe(`g${r.key}`);
    expect(older.text).toBe("Read 1 file, ran 1 shell command");
    expect(newest.text).toBe("Edited a.rs");
  });

  it("marks a result-less call running only in the newest run of a live turn", () => {
    const old = tool("shell", '{"command":"lost"}', null);
    const prose = message("p");
    const now = tool("shell", '{"command":"cargo test"}', null);
    const fin = tool("read_file", '{"path":"a"}', "ok");
    const folded = groups(foldToolGroups([now, fin, prose, old], true));
    expect(folded[0].calls.map((c) => c.status)).toEqual(["ok", "running"]);
    expect(folded[0].text).toBe("Read a");
    expect(folded[1].calls[0].status).toBe("ok");
    expect(groups(foldToolGroups([now, fin], false))[0].calls[1].status).toBe("ok");
  });

  it("keeps the newest failures and the running calls visible under a collapsed summary", () => {
    const calls = [
      tool("shell", '{"command":"ls"}', "ok"),
      tool("shell", '{"command":"exit 3"}', "error: exit 3"),
      tool("shell", '{"command":"sleep 9"}', null),
    ].reverse();
    const [g] = groups(foldToolGroups(calls, true));
    expect(g.summary.failed).toBe(1);
    expect(collapsedCalls(g).map((c) => c.status)).toEqual(["failed", "running"]);
    const lone = groups(foldToolGroups([tool("edit_file", '{"path":"x.rs"}', "error: no match")], false))[0];
    expect(lone.text).toBe("Tried to edit x.rs");
    expect(collapsedCalls(lone)).toEqual([]);
  });

  it("rebuilds only the group whose calls changed", () => {
    const memo = new GroupMemo();
    const older = [tool("read_file", '{"path":"a"}', "ok"), tool("read_file", '{"path":"b"}', "ok")].reverse();
    const prose = message("between");
    const running = tool("shell", '{"command":"cargo test"}', null);
    const first = groups(foldToolGroups([running, prose, ...older], true, memo));
    expect(memo.builds).toBe(2);
    // A refetch hands back equal rows as new objects, and the running call finished.
    const copy = older.map((e) => ({ ...e })) as TranscriptEntry[];
    const finished: TranscriptEntry = {
      kind: "tool",
      key: running.key,
      invocation: { ...(running as Extract<TranscriptEntry, { kind: "tool" }>).invocation, result: "ok" },
    };
    const second = groups(foldToolGroups([finished, prose, ...copy], true, memo));
    expect(memo.builds).toBe(3);
    expect(second[1]).toBe(first[1]);
    expect(second[0]).not.toBe(first[0]);
    expect(second[0].text).toBe("Ran cargo test");
  });
});

describe("run-aware folding", () => {
  const entry = (key: string, tool: string, args: string, result: string | null, extra: object = {}): TranscriptEntry => ({
    kind: "tool",
    key,
    invocation: { seq: 1, tool, args, result, ...extra },
  });

  it("keeps an answered question out of the group around it", () => {
    const entries = [
      entry("c", "shell", '{"command":"ls"}', "a"),
      entry("b", "ask_user", '{"question":"Go?"}', "Yes"),
      entry("a", "read_file", '{"path":"x"}', "x"),
    ];
    const folded = foldToolGroups(entries, false);
    expect(folded.map((e) => e.kind)).toEqual(["toolGroup", "tool", "toolGroup"]);
  });

  it("sums the daemon's line counts into the edit summary", () => {
    const entries = [
      entry("b", "edit_file", '{"path":"src/a.ts"}', "ok", { adds: 5, dels: 2 }),
      entry("a", "edit_file", '{"path":"src/a.ts"}', "ok", { adds: 3, dels: 0 }),
    ];
    const group = foldToolGroups(entries, false)[0] as ToolGroupEntry;
    expect(group.text).toBe("Edited src/a.ts (+8 −2)");
  });

  it("titles a shell call by what it does, not by its wrapper", () => {
    const entries = [entry("a", "shell", JSON.stringify({ command: "bash -lc 'cd /r && sed -n \"1,20p\" src/a.ts'" }), "x")];
    const group = foldToolGroups(entries, false)[0] as ToolGroupEntry;
    expect(group.text).toContain("View");
  });

  it("keeps a call that started agents on screen when the group is folded", () => {
    const entries = [
      entry("d", "read_file", '{"path":"c"}', "x"),
      entry("c", "spawn_agents", '{"agents":[{}]}', "done"),
      entry("b", "read_file", '{"path":"b"}', "x"),
      entry("a", "read_file", '{"path":"a"}', "x"),
    ];
    const group = foldToolGroups(entries, false)[0] as ToolGroupEntry;
    expect(collapsedCalls(group).map((c) => c.invocation.tool)).toEqual(["spawn_agents"]);
  });
});
