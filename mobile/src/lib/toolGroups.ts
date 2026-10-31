// Tool groups: a run of tool calls the assistant made without writing anything in between,
// folded into one dim summary row ("Read 3 files, ran 2 shell commands"). A port of the TUI's
// crates/forge-tui/src/app/tool_groups.rs (summary wording) and group_flow.rs (membership), so
// the phone and the terminal describe the same run of calls in the same words.
//
// Pure: `summarize`/`summaryText` turn calls into the sentence, `foldToolGroups` turns a
// newest-first transcript into entries + groups. Rendering lives in components/chat/ToolGroupRow.
import { commandSummary } from "./commandSummary";
import { shortenPath } from "./pathDisplay";
import { scriptCalls, scriptHeadline, WRAPPER_TOOL } from "./scriptCalls";
import { isDecision } from "./decisionRecord";
import { screenshotPathOf } from "./mediaApi";
import { parseArgs, resultFailed, type ToolInvocation, type TranscriptEntry } from "./toolRows";

export type CallStatus = "ok" | "failed" | "running";

/** What one call did to a file, when the caller knows (the TUI's attached diff, or the line
 * counts a current daemon puts on an edit call's history row). Left unset otherwise, and the
 * summary then has no `(+a −d)`. */
export interface CallDiff {
  kind: "created" | "modified" | "deleted";
  path?: string;
  adds: number;
  dels: number;
}

export interface GroupCall {
  name: string;
  args: string;
  status: CallStatus;
  diff?: CallDiff | null;
  /** The call's stored result, which for `run_code` carries the count of calls it made. */
  result?: string | null;
}

type Category =
  | { kind: "read" | "list" | "search" | "shell" | "script" | "created" | "edited" | "deleted" }
  | { kind: "fetch" | "webSearch" | "agents" | "tasks" }
  | { kind: "mcp" | "other"; name: string };

export interface Tally {
  category: Category;
  calls: number;
  subjects: string[];
  unnamed: number;
  adds: number;
  dels: number;
  target: string;
  /** Script tallies: tool calls made by scripts whose result footer was stored. */
  scriptCalls: number;
  /** Script tallies: scripts with no stored footer, counted as scripts. */
  scriptsUncounted: number;
}

export interface Summary {
  tallies: Tally[];
  failedTallies: Tally[];
  finished: number;
  failed: number;
  running: number;
}

const MINUS = "−";
const BRIDGE_PREFIX = "mcp__forge__";

/** A claude-cli turn names Forge's own tools `mcp__forge__<tool>`; they group as the tool. */
export function bareToolName(name: string): string {
  return name.startsWith(BRIDGE_PREFIX) ? name.slice(BRIDGE_PREFIX.length) : name;
}

function categoryOf(call: GroupCall): Category {
  let name = bareToolName(call.name);
  if (name === WRAPPER_TOOL) {
    const inner = field(call.args, ["name", "tool"]);
    if (!inner) return { kind: "other", name: "" };
    name = bareToolName(inner);
    call = { ...call, name };
  }
  if (name.startsWith("mcp__")) {
    const rest = name.slice("mcp__".length);
    const at = rest.indexOf("__");
    if (at >= 0) return { kind: "mcp", name: rest.slice(0, at) };
  }
  switch (name) {
    case "read_file":
      return { kind: "read" };
    case "list_dir":
      return { kind: "list" };
    case "search":
    case "grep":
    case "glob":
      return { kind: "search" };
    case "shell":
    case "shell_job":
    case "python":
      return { kind: "shell" };
    case "run_code":
      return { kind: "script" };
    case "delete_file":
      return { kind: "deleted" };
    case "write_file":
    case "edit_file":
    case "multi_edit":
    case "apply_patch":
    case "append_file":
    case "notebook_edit":
      if (call.diff?.kind === "created") return { kind: "created" };
      if (call.diff?.kind === "deleted") return { kind: "deleted" };
      return { kind: "edited" };
    case "web_fetch":
      return { kind: "fetch" };
    case "web_search":
      return { kind: "webSearch" };
    case "spawn_agents":
    case "dispatch_sessions":
    case "task":
    case "agent":
      return { kind: "agents" };
    case "update_tasks":
      return { kind: "tasks" };
    default:
      return { kind: "other", name };
  }
}

function sameCategory(a: Category, b: Category): boolean {
  if (a.kind !== b.kind) return false;
  return !("name" in a) || ("name" in b && a.name === b.name);
}

function bySubject(c: Category): boolean {
  return (
    c.kind === "read" ||
    c.kind === "list" ||
    c.kind === "search" ||
    c.kind === "created" ||
    c.kind === "edited" ||
    c.kind === "deleted" ||
    c.kind === "fetch"
  );
}

/** A string field of a call's args. The store caps long values with a trailing `…`, which
 * leaves the JSON unparsable, so a field that precedes the cut is still read from the text. */
export function field(args: string, keys: string[]): string | null {
  const parsed = parseArgs(args);
  for (const key of keys) {
    let value: unknown = parsed?.[key];
    if (!parsed) {
      const m = new RegExp(`"${key}"\\s*:\\s*("(?:[^"\\\\]|\\\\.)*")`).exec(args);
      if (m) {
        try {
          value = JSON.parse(m[1]);
        } catch {
          value = undefined;
        }
      }
    }
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return null;
}

function oneline(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}

export function displayPath(path: string, cwd: string | null): string {
  if (cwd) {
    const base = cwd.endsWith("/") ? cwd : `${cwd}/`;
    if (path.startsWith(base) && path.length > base.length) return path.slice(base.length);
  }
  return path;
}

/** What a call is about, for a group of one: the command run, the file read. "" when nothing is
 * worth naming (the raw argument blob is never shown). */
/** The cell a `python` call runs, decoded from its JSON arguments. */
export function pythonCode(args: string): string {
  return field(args, ["code"]) ?? "";
}

export function headline(name: string, args: string): string {
  const tool = bareToolName(name);
  const cwd = field(args, ["cwd"]);
  let first: string | null = null;
  switch (tool) {
    case "shell":
      first = field(args, ["command"]);
      if (!first) {
        const list = parseArgs(args)?.commands;
        if (Array.isArray(list) && typeof list[0] === "string") {
          first = list.length === 1 ? list[0] : `${list[0]} (+${list.length - 1} more)`;
        }
      }
      return first ? commandSummary(first) : "";
    case "read_file":
    case "write_file":
    case "edit_file":
    case "list_dir": {
      const p = field(args, ["path"]);
      first = p ? shortenPath(displayPath(p, cwd)) : null;
      break;
    }
    case "apply_patch": {
      const p = field(args, ["path"]);
      first = p ? shortenPath(displayPath(p, cwd)) : cwd ? shortenPath(cwd) : null;
      break;
    }
    case "search":
    case "grep":
      first = field(args, ["pattern", "query"]);
      break;
    case "glob":
      first = field(args, ["pattern"]);
      break;
    case "web_fetch":
      first = field(args, ["url"]);
      break;
    case "web_search":
      first = field(args, ["query"]);
      break;
    case "run_code":
      return scriptHeadline(args, null);
    case "python":
      first = pythonCode(args)
        .split("\n")
        .map((line) => line.trim())
        .find((line) => line && !line.startsWith("#") && !line.startsWith("```")) ?? null;
      return first ? oneline(first) : "";
  }
  if (first === null) {
    const parsed = parseArgs(args);
    const value = parsed ? Object.values(parsed).find((v) => typeof v === "string") : undefined;
    first = typeof value === "string" ? value : null;
  }
  return first ? oneline(first) : "";
}

/** What one call touched, so repeats count once (`./src/a.rs` and `src/a.rs` are one file). */
function subject(category: Category, args: string): string | null {
  if (category.kind === "search") return field(args, ["pattern", "query"]);
  if (category.kind === "fetch") return field(args, ["url"]);
  let path = field(args, ["path", "file_path", "notebook_path"]);
  if (path === null) return null;
  while (path.startsWith("./")) path = path.slice(2);
  return path;
}

function agentsSpawned(args: string): number {
  const parsed = parseArgs(args);
  for (const key of ["agents", "tasks", "items"]) {
    const v = parsed?.[key];
    if (Array.isArray(v) && v.length > 0) return v.length;
  }
  return 1;
}

export function tallyCount(t: Tally): number {
  return bySubject(t.category) ? t.subjects.length + t.unnamed : t.calls;
}

function touch(t: Tally, s: string | null) {
  if (s === null) t.unnamed += 1;
  else if (!t.subjects.includes(s)) t.subjects.push(s);
}

function newTally(category: Category, target: string): Tally {
  return { category, calls: 0, subjects: [], unnamed: 0, adds: 0, dels: 0, target, scriptCalls: 0, scriptsUncounted: 0 };
}

/** A finished, successful `run_code` call whose source calls exactly one tool, as the calls it
 * made (count from the stored result footer). Null for anything else, which tallies as a script. */
function expandScript(call: GroupCall): GroupCall[] | null {
  if (bareToolName(call.name) !== "run_code" || call.status !== "ok") return null;
  const s = scriptCalls(call.args, call.result ?? null);
  if (s.tools.length !== 1 || s.total == null || s.total < 1 || s.failed > 0) return null;
  const tool = s.tools[0];
  if (categoryOf({ name: tool, args: "", status: "ok" }).kind === "script") return null;
  return Array.from({ length: s.total }, (_, i) => ({
    name: tool,
    args: tool === "shell" && s.commands[i] ? JSON.stringify({ command: s.commands[i] }) : "",
    status: "ok" as const,
  }));
}

/** Running calls are counted, not tallied: the sentence is in the past tense. Failed calls go to
 * their own tallies and never count as done. */
export function summarize(calls: readonly GroupCall[]): Summary {
  const out: Summary = { tallies: [], failedTallies: [], finished: 0, failed: 0, running: 0 };
  for (const original of calls) {
    const expanded = expandScript(original);
    if (expanded) {
      // A script that only called one tool reads as those calls: "Ran 8 shell commands".
      out.finished += 1;
      const category = categoryOf(expanded[0]);
      let tally = out.tallies.find((t) => sameCategory(t.category, category));
      if (!tally) {
        tally = newTally(category, expanded.length === 1 ? headline(expanded[0].name, expanded[0].args) : "");
        out.tallies.push(tally);
      }
      for (const call of expanded) {
        tally.calls += 1;
        if (bySubject(category)) touch(tally, subject(category, call.args));
      }
      continue;
    }
    const call = original;
    if (call.status === "running") {
      out.running += 1;
      continue;
    }
    out.finished += 1;
    const failed = call.status === "failed";
    if (failed) out.failed += 1;
    const tallies = failed ? out.failedTallies : out.tallies;
    const category = categoryOf(call);
    let tally = tallies.find((t) => sameCategory(t.category, category));
    if (!tally) {
      tally = newTally(category, headline(call.name, call.args));
      tallies.push(tally);
    }
    if (category.kind === "agents") {
      tally.calls += agentsSpawned(call.args);
    } else if (bySubject(category)) {
      tally.calls += 1;
      const fromDiff = call.diff?.path?.trim() || null;
      touch(tally, subject(category, call.args) ?? fromDiff);
    } else {
      tally.calls += 1;
      if (category.kind === "script") {
        const total = scriptCalls(call.args, call.result ?? null).total;
        if (total != null && total > 0) tally.scriptCalls += total;
        else tally.scriptsUncounted += 1;
      }
    }
    if (!failed && call.diff) {
      tally.adds += call.diff.adds;
      tally.dels += call.diff.dels;
    }
  }
  return out;
}

function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

function stat(t: Tally): string {
  const { adds: a, dels: d } = t;
  if (a === 0 && d === 0) return "";
  if (d === 0) return ` (+${a})`;
  if (a === 0) return ` (${MINUS}${d})`;
  return ` (+${a} ${MINUS}${d})`;
}

type Tense = "past" | "tried";

function changedFiles(t: Tally): string {
  if (t.unnamed === 0 && t.subjects.length >= 1 && t.subjects.length <= 2) return t.subjects.join(", ");
  return plural(tallyCount(t), "file", "files");
}

function counted(t: Tally, tense: Tense): string {
  const n = tallyCount(t);
  const v = (past: string, inf: string) => (tense === "past" ? past : inf);
  const c = t.category;
  switch (c.kind) {
    case "read":
      return `read ${plural(n, "file", "files")}`;
    case "list":
      return `${v("listed", "list")} ${plural(n, "directory", "directories")}`;
    case "search":
      return `${v("searched for", "search for")} ${plural(n, "pattern", "patterns")}`;
    case "shell":
      return `${v("ran", "run")} ${plural(n, "shell command", "shell commands")}`;
    case "script":
      if (t.scriptCalls > 0 && t.scriptsUncounted === 0) {
        return `${v("ran", "run")} ${plural(t.scriptCalls, "tool call", "tool calls")} in ${plural(n, "script", "scripts")}`;
      }
      return `${v("ran", "run")} ${plural(n, "script", "scripts")}`;
    case "created":
      return `${v("created", "create")} ${changedFiles(t)}${stat(t)}`;
    case "edited":
      return `${v("edited", "edit")} ${changedFiles(t)}${stat(t)}`;
    case "deleted":
      return `${v("deleted", "delete")} ${changedFiles(t)}${stat(t)}`;
    case "fetch":
      return `${v("fetched", "fetch")} ${plural(n, "page", "pages")}`;
    case "webSearch":
      return `${v("ran", "run")} ${plural(n, "web search", "web searches")}`;
    case "agents":
      return `${v("ran", "run")} ${plural(n, "agent", "agents")}`;
    case "tasks":
      if (tense === "tried") return "update the task list";
      return n === 1 ? "updated the task list" : `updated the task list ${n} times`;
    case "mcp":
    case "other":
      if (!c.name) return `${v("called", "call")} ${plural(n, "tool", "tools")}`;
      return `${v("called", "call")} ${c.name} ${plural(n, "time", "times")}`;
  }
}

const SINGLE_VERBS: Partial<Record<Category["kind"], [string, string]>> = {
  read: ["read", "read"],
  list: ["listed", "list"],
  search: ["searched for", "search for"],
  shell: ["ran", "run"],
  created: ["created", "create"],
  edited: ["edited", "edit"],
  deleted: ["deleted", "delete"],
  fetch: ["fetched", "fetch"],
  webSearch: ["searched the web for", "search the web for"],
};

function single(t: Tally, tense: Tense): string | null {
  const target = t.target.trim();
  if (!target || tallyCount(t) !== 1) return null;
  const verbs = SINGLE_VERBS[t.category.kind];
  if (!verbs) return null;
  const verb = tense === "past" ? verbs[0] : verbs[1];
  const withStat = t.category.kind === "created" || t.category.kind === "edited" ? stat(t) : "";
  return `${verb} ${target}${withStat}`;
}

/** The sentence: categories in order of first appearance, failed calls left out, and a group
 * where nothing succeeded saying what it tried ("Tried to edit sample.rs"). */
export function summaryText(s: Summary): string {
  const tense: Tense = s.tallies.length === 0 ? "tried" : "past";
  const tallies = tense === "past" ? s.tallies : s.failedTallies;
  let text =
    tallies.length === 0
      ? ""
      : tallies.length === 1
        ? (single(tallies[0], tense) ?? counted(tallies[0], tense))
        : tallies.map((t) => counted(t, tense)).join(", ");
  if (tense === "tried" && text) text = `tried to ${text}`;
  return text ? text[0].toUpperCase() + text.slice(1) : "";
}

// ---- Membership ------------------------------------------------------------------------------

/** The failed calls a collapsed group keeps on screen, newest last. */
export const FAILED_ROWS = 3;
/** The running calls a collapsed group shows under its summary. */
export const RUNNING_ROWS = 3;

export interface ToolGroupCall {
  key: string;
  invocation: ToolInvocation;
  status: CallStatus;
}

export interface ToolGroupEntry {
  kind: "toolGroup";
  /** `g` + the key of the group's first (oldest) call, which a growing group keeps. */
  key: string;
  /** Oldest first. */
  calls: ToolGroupCall[];
  summary: Summary;
  text: string;
}

export type FoldedEntry = TranscriptEntry | ToolGroupEntry;

/** What an edit call changed, when the daemon counted it. Unset for history from an older host. */
export function invocationDiff(invocation: ToolInvocation): CallDiff | undefined {
  if (invocation.adds == null || invocation.dels == null) return undefined;
  return { kind: "modified", adds: invocation.adds, dels: invocation.dels };
}

export function invocationStatus(invocation: ToolInvocation, live: boolean): CallStatus {
  if (invocation.result === null) return live ? "running" : "ok";
  return resultFailed(invocation.result) ? "failed" : "ok";
}

/** Reuses a group's summary and object while its calls are unchanged, so a refetch or a
 * streamed token rebuilds only the group that actually changed (in practice the running one). */
export class GroupMemo {
  private groups = new Map<string, ToolGroupEntry>();
  builds = 0;

  get(key: string, calls: ToolGroupCall[]): ToolGroupEntry {
    const prev = this.groups.get(key);
    if (prev && sameCalls(prev.calls, calls)) return prev;
    this.builds += 1;
    const summary = summarize(
      calls.map((c) => ({
        name: c.invocation.tool ?? "tool",
        args: c.invocation.args,
        status: c.status,
        result: c.invocation.result,
        diff: invocationDiff(c.invocation),
      })),
    );
    const group: ToolGroupEntry = { kind: "toolGroup", key, calls, summary, text: summaryText(summary) };
    this.groups.set(key, group);
    return group;
  }

  keepOnly(keys: Set<string>) {
    for (const key of this.groups.keys()) if (!keys.has(key)) this.groups.delete(key);
  }
}

function sameCalls(a: ToolGroupCall[], b: ToolGroupCall[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const x = a[i];
    const y = b[i];
    if (
      x.key !== y.key ||
      x.status !== y.status ||
      x.invocation.tool !== y.invocation.tool ||
      x.invocation.args !== y.invocation.args ||
      x.invocation.result !== y.invocation.result ||
      x.invocation.adds !== y.invocation.adds ||
      x.invocation.dels !== y.invocation.dels
    ) {
      return false;
    }
  }
  return true;
}

function isGroupable(entry: TranscriptEntry): boolean {
  return entry.kind === "tool" && !isDecision(entry.invocation);
}

/**
 * Folds each run of consecutive tool entries (no message between them) into one group. Input
 * and output are newest-first, as the inverted chat list consumes them. `live` marks the turn as
 * in flight: a call without a result in the newest run is then running; elsewhere a missing
 * result means it was never stored, and the call counts as finished.
 */
export function foldToolGroups(
  entries: readonly TranscriptEntry[],
  live: boolean,
  memo: GroupMemo = new GroupMemo(),
): FoldedEntry[] {
  const out: FoldedEntry[] = [];
  const keys = new Set<string>();
  let i = 0;
  while (i < entries.length) {
    const entry = entries[i];
    if (entry.kind !== "tool" || isDecision(entry.invocation)) {
      out.push(entry);
      i += 1;
      continue;
    }
    const newest = i === 0;
    let j = i;
    while (j < entries.length && isGroupable(entries[j])) j += 1;
    const calls: ToolGroupCall[] = [];
    for (let k = j - 1; k >= i; k--) {
      const e = entries[k] as Extract<TranscriptEntry, { kind: "tool" }>;
      calls.push({ key: e.key, invocation: e.invocation, status: invocationStatus(e.invocation, live && newest) });
    }
    const key = `g${calls[0].key}`;
    keys.add(key);
    out.push(memo.get(key, calls));
    i = j;
  }
  memo.keepOnly(keys);
  return out;
}

const DELEGATING = new Set(["spawn_agents", "run_workflow", "workflow"]);

function isDelegation(call: ToolGroupCall): boolean {
  return call.invocation.tool != null && DELEGATING.has(bareToolName(call.invocation.tool));
}

const SCREENSHOT_ROWS = 3;

/** The calls a collapsed group keeps visible under its summary: its newest failures (once more
 * than one call finished; a lone failure is the summary itself), the calls still running, any
 * call that started child agents, whose rows are the way into them, and the newest screenshots,
 * whose picture is the result. */
export function collapsedCalls(group: ToolGroupEntry): ToolGroupCall[] {
  const { finished } = group.summary;
  const failed = finished > 1 ? group.calls.filter((c) => c.status === "failed").slice(-FAILED_ROWS) : [];
  const running = group.calls.filter((c) => c.status === "running").slice(0, RUNNING_ROWS);
  const delegated = group.calls.filter(isDelegation);
  const screenshots = group.calls.filter((c) => screenshotPathOf(c.invocation.result) != null).slice(-SCREENSHOT_ROWS);
  const shown = new Set([...failed, ...running, ...delegated, ...screenshots]);
  return group.calls.filter((c) => shown.has(c));
}
