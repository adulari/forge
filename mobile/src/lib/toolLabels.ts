// The words one tool call's row shows: a short verb ("Run", "Read", "Script") and what the call
// was about (the command, the path), after the TUI's tool cards (crates/forge-tui/src/app/
// tool_cards.rs `verb`/`headline`). The raw tool name and argument blob stay in the expanded
// details; a row that says only "run_code" tells the reader nothing.
import { headEllipsis } from "../components/review/pathTruncate";
import { tildify } from "./pathDisplay";
import { scriptCalls, scriptHeadline, scriptSource, scriptVerb } from "./scriptCalls";
import { bareToolName, displayPath, field, headline, pythonCode } from "./toolGroups";
import { parseArgs, summarizeToolArgs } from "./toolRows";

const VERBS: Record<string, string> = {
  read_file: "Read",
  write_file: "Write",
  edit_file: "Edit",
  multi_edit: "Edit",
  append_file: "Append",
  delete_file: "Delete",
  apply_patch: "Patch",
  notebook_edit: "Edit",
  shell: "Run",
  shell_job: "Run",
  grep: "Search",
  search: "Search",
  glob: "Find",
  list_dir: "List",
  web_fetch: "Fetch",
  web_search: "Web",
  spawn_agents: "Agents",
  submit_result: "Result",
  dispatch_sessions: "Agents",
  update_tasks: "Tasks",
  run_code: "Script",
  python: "Python",
};

const PATH_TOOLS = new Set(["read_file", "write_file", "edit_file", "multi_edit", "append_file", "list_dir", "notebook_edit"]);
// A path keeps its tail: the filename is what a reader scans for, and head-truncation never cuts it.
const PATH_MAX = 40;

/** The inner tool a `tool_call{name, arguments}` wrapper reached; the wrapper's own name is
 * internal and never shown. */
function wrappedName(args: string): string | null {
  const parsed = parseArgs(args);
  const name = parsed?.name ?? parsed?.tool;
  return typeof name === "string" && name.trim() ? bareToolName(name.trim()) : null;
}

export function callVerb(name: string | null, args?: string): string {
  const tool = bareToolName(name ?? "tool");
  if (tool === "run_code" && args) return scriptVerb(args);
  if (tool === "tool_call") {
    const inner = args ? wrappedName(args) : null;
    return inner ? callVerb(inner) : "Call";
  }
  if (VERBS[tool]) return VERBS[tool];
  if (tool.startsWith("mcp__")) {
    const rest = tool.slice("mcp__".length);
    const at = rest.indexOf("__");
    return at >= 0 ? rest.slice(at + 2) : rest;
  }
  return tool;
}

export function callHeadline(name: string | null, args: string, result: string | null): string {
  const tool = bareToolName(name ?? "tool");
  if (tool === "run_code") return scriptHeadline(args, result);
  if (tool === "tool_call") {
    const inner = wrappedName(args);
    if (!inner) return "a tool";
    const innerArgs = parseArgs(args)?.arguments;
    const text = typeof innerArgs === "string" ? innerArgs : JSON.stringify(innerArgs ?? {});
    return VERBS[inner] ? callHeadline(inner, text, result) : "";
  }
  if (PATH_TOOLS.has(tool)) {
    const path = field(args, ["path", "file_path", "notebook_path"]);
    if (path) return headEllipsis(tildify(displayPath(path, field(args, ["cwd"]))), PATH_MAX);
  }
  return headline(tool, args) || summarizeToolArgs(args, 80);
}

const LEAD_KEYS = ["command", "commands", "path", "file_path", "pattern", "query", "url", "code", "script"];

export interface ArgSection {
  label: string;
  text: string;
}

/** The expanded row's argument sections, decoded for reading: a script's commands and its source
 * (never the JSON-escaped `{"code": "const r = await tools.shell(...)"}`), and for other tools one
 * `key: value` line per argument with the identifying one first. The raw blob stays one tap away. */
export function readableArgs(name: string | null, args: string): ArgSection[] {
  const tool = bareToolName(name ?? "tool");
  if (tool === "run_code") {
    const s = scriptCalls(args, null);
    const source = scriptSource(args)
      .split("\n")
      .filter((line) => line.trim() && !line.trim().startsWith("```"))
      .join("\n");
    const sections: ArgSection[] = [];
    if (s.commands.length > 0) sections.push({ label: "commands", text: s.commands.map((c) => `$ ${tildify(c)}`).join("\n") });
    if (source) sections.push({ label: "script", text: source });
    return sections;
  }
  if (tool === "python") {
    const cell = pythonCode(args);
    if (cell) return [{ label: "code", text: cell }];
  }
  const parsed = parseArgs(args);
  if (!parsed) return args.trim() ? [{ label: "arguments", text: args.trim() }] : [];
  const keys = Object.keys(parsed).sort((a, b) => {
    const ia = LEAD_KEYS.indexOf(a);
    const ib = LEAD_KEYS.indexOf(b);
    return (ia < 0 ? 99 : ia) - (ib < 0 ? 99 : ib) || a.localeCompare(b);
  });
  const lines = keys.map((key) => {
    const value = parsed[key];
    const text = tildify(typeof value === "string" ? value : JSON.stringify(value));
    return text.includes("\n") ? `${key}:\n${text}` : `${key}: ${text}`;
  });
  return lines.length ? [{ label: "arguments", text: lines.join("\n") }] : [];
}
