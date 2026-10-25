// What a `run_code` call did, read from its stored arguments and result. A script's own tool
// calls never reach history as rows of their own, so the script source (`tools.shell({...})`)
// and the result footer the daemon appends (`[run_code: 8 tool calls, 8 ok, 0 failed, 1.2s]`,
// crates/forge-core/src/codemode/mod.rs `footer`) are the only record. The TUI counts a script
// by the calls it made ("Ran 8 shell commands", not "Ran 3 scripts"); this lets the app do the
// same, and lets a script's row say what it ran instead of just "run_code".
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

export function scriptCalls(args: string, result: string | null): ScriptCalls {
  const code = scriptSource(args);
  const tools: string[] = [];
  for (const m of code.matchAll(TOOL_CALL)) {
    if (!tools.includes(m[1])) tools.push(m[1]);
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
};

function oneline(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}

/** The line a script's row shows: its command when it ran literal shell commands, else the
 * verbs of the tools it called with the call count ("edit, run · 8 calls"). */
export function scriptHeadline(args: string, result: string | null): string {
  const s = scriptCalls(args, result);
  const calls = s.total != null && s.total > 0 ? `${s.total} call${s.total === 1 ? "" : "s"}` : "";
  if (s.tools.length === 1 && s.tools[0] === "shell" && s.commands.length > 0) {
    const first = oneline(s.commands[0]);
    const more = s.commands.length - 1;
    return more > 0 ? `${first} (+${more} more)` : first;
  }
  const verbs: string[] = [];
  for (const tool of s.tools) {
    const verb = VERBS[tool] ?? tool;
    if (!verbs.includes(verb)) verbs.push(verb);
  }
  return [verbs.join(", "), calls].filter(Boolean).join(" · ");
}
