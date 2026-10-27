// What a `run_code` call did, read from its stored arguments and result. A script's own tool
// calls never reach history as rows of their own, so the script source (`tools.shell({...})`)
// and the result footer the daemon appends (`[run_code: 8 tool calls, 8 ok, 0 failed, 1.2s]`,
// crates/forge-core/src/codemode/mod.rs `footer`) are the only record. The TUI counts a script
// by the calls it made ("Ran 8 shell commands", not "Ran 3 scripts"); this lets the app do the
// same, and lets a script's row say what it ran instead of just "run_code".
import { friendlyCommand, shortenPath } from "./pathDisplay";
import { parseArgs } from "./toolRows";

export interface ScriptCalls {
  /** Calls the script made, from the result footer; null when no footer was stored. */
  total: number | null;
  failed: number;
  /** Distinct tools the source calls, in order of first use. */
  tools: string[];
  /** Literal shell commands in the source, in order. */
  commands: string[];
}

/** `tool_call` is the deferred-tool wrapper (`tool_call{name, arguments}`), never what the call
 * was about. */
export const WRAPPER_TOOL = "tool_call";

const FOOTER = /\[run_code: (\d+) tool calls?, (\d+) ok, (\d+) failed/;
const TOOL_CALL = /tools\.([A-Za-z_]\w*)\s*\(/g;
// The closing quote may be missing: the store cuts a long script off mid-string with "…".
const SHELL_COMMAND = /tools\.shell\s*\(\s*\{[^}]*?\bcommand\s*:\s*(["'`])((?:\\.|(?!\1)[^\\])*)(?:\1|$)/g;

/** The script source. The store caps long values with a trailing `…`, which leaves the JSON
 * unparsable; the source is then read straight out of the text up to the cut. */
export function scriptSource(args: string): string {
  const parsed = parseArgs(args);
  for (const key of ["code", "script"]) {
    const value = parsed?.[key];
    if (typeof value === "string") return value;
  }
  const m = /"(?:code|script)"\s*:\s*"((?:[^"\\]|\\.)*)/.exec(args);
  if (m) {
    try {
      return JSON.parse(`"${m[1].replace(/\\$/, "")}"`) as string;
    } catch {
      return m[1].replace(/\\n/g, "\n").replace(/\\"/g, '"');
    }
  }
  return parsed ? "" : args;
}

const WRAPPED_NAME = /tools\.tool_call\s*\(\s*\{[^}]*?\bname\s*:\s*(["'`])((?:\\.|(?!\1)[^\\])*?)\1/g;

/** The tool names a script's `tools.tool_call({name: "x"})` calls reach, in order. */
export function wrappedToolNames(code: string): string[] {
  const names: string[] = [];
  for (const m of code.matchAll(WRAPPED_NAME)) if (!names.includes(m[2])) names.push(m[2]);
  return names;
}

export function scriptCalls(args: string, result: string | null): ScriptCalls {
  const code = scriptSource(args);
  const tools: string[] = [];
  for (const m of code.matchAll(TOOL_CALL)) {
    if (m[1] === WRAPPER_TOOL) {
      const inner = wrappedToolNames(code);
      for (const name of inner.length ? inner : [WRAPPER_TOOL]) if (!tools.includes(name)) tools.push(name);
    } else if (!tools.includes(m[1])) tools.push(m[1]);
  }
  const commands = [...code.matchAll(SHELL_COMMAND)].map((m) => m[2].replace(/\\(.)/g, "$1").trim()).filter(Boolean);
  const footer = result ? FOOTER.exec(result) : null;
  return {
    total: footer ? Number(footer[1]) : null,
    failed: footer ? Number(footer[3]) : 0,
    tools,
    commands,
  };
}

const VERBS: Record<string, string> = {
  read_file: "read",
  write_file: "write",
  edit_file: "edit",
  multi_edit: "edit",
  apply_patch: "patch",
  shell: "run",
  grep: "search",
  search: "search",
  glob: "find",
  list_dir: "list",
  web_fetch: "fetch",
  web_search: "web",
  spawn_agents: "agents",
  [WRAPPER_TOOL]: "tool",
};

/** What a single-tool script's row says with the row verb ("Read", "Write", ...). */
const ROW_VERBS: Record<string, [string, string, string]> = {
  read_file: ["Read", "file", "files"],
  write_file: ["Write", "file", "files"],
  edit_file: ["Edit", "file", "files"],
  multi_edit: ["Edit", "file", "files"],
  append_file: ["Append", "file", "files"],
  delete_file: ["Delete", "file", "files"],
  list_dir: ["List", "directory", "directories"],
  grep: ["Search", "pattern", "patterns"],
  search: ["Search", "pattern", "patterns"],
  glob: ["Find", "pattern", "patterns"],
  web_fetch: ["Fetch", "page", "pages"],
  web_search: ["Web", "search", "searches"],
};

const PATH_LITERAL = /tools\.(\w+)\s*\(\s*\{[^}]*?\b(?:path|file_path)\s*:\s*(["'`])((?:\\.|(?!\2)[^\\])*)\2/g;

/** The literal `path` arguments the source passes to `tool`, in order. */
function scriptPaths(code: string, tool: string): string[] {
  const out: string[] = [];
  for (const m of code.matchAll(PATH_LITERAL)) if (m[1] === tool) out.push(m[3].replace(/\\(.)/g, "$1"));
  return out;
}

/** The tool a script is about when it only ever called one (a `tool_call` wrapper resolved to the
 * tool it reached), else null. */
function onlyTool(s: ScriptCalls): string | null {
  return s.tools.length === 1 ? s.tools[0] : null;
}

/** The verb a script's row leads with: the tool's own ("Run", "Write") when the script called one
 * tool, "Script" when it mixed several. */
export function scriptVerb(args: string): string {
  const only = onlyTool(scriptCalls(args, null));
  if (!only) return "Script";
  if (only === "shell") return "Run";
  if (only === WRAPPER_TOOL) return "Call";
  return ROW_VERBS[only]?.[0] ?? (only.startsWith("mcp__") ? "Call" : "Script");
}

/** The line a script's row shows after its verb. A script of one tool says what that tool did: the
 * command for a shell script, the path or "2 files" for file tools, the tool's name for a wrapped
 * call. A mixed script lists the verbs of the tools it called with the call count. */
export function scriptHeadline(args: string, result: string | null): string {
  const s = scriptCalls(args, result);
  const calls = s.total != null && s.total > 0 ? `${s.total} call${s.total === 1 ? "" : "s"}` : "";
  const only = onlyTool(s);
  if (only === "shell" && s.commands.length > 0) {
    const first = friendlyCommand(s.commands[0]);
    const more = s.commands.length - 1;
    return more > 0 ? `${first} (+${more} more)` : first;
  }
  if (only === "shell") return calls ? calls.replace("call", "command") : "";
  if (only && only !== WRAPPER_TOOL && ROW_VERBS[only]) {
    const [, one, many] = ROW_VERBS[only];
    const paths = [...new Set(scriptPaths(scriptSource(args), only))];
    if (paths.length === 1 && (s.total == null || s.total <= 1)) return shortenPath(paths[0]);
    const n = s.total != null && s.total > 0 ? s.total : paths.length;
    if (n > 0) return `${n} ${n === 1 ? one : many}`;
    return "";
  }
  if (only === WRAPPER_TOOL) return "a tool";
  if (only && only.startsWith("mcp__")) return only.slice(only.indexOf("__", 5) + 2) || only;
  const verbs: string[] = [];
  for (const tool of s.tools) {
    const verb = VERBS[tool] ?? tool;
    if (!verbs.includes(verb)) verbs.push(verb);
  }
  return [verbs.join(", "), calls].filter(Boolean).join(" · ");
}
