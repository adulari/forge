// Checks as plain data: turn a project action's terminal output into pass/fail, file:line links,
// the files a failing check implicates, and the "ask the agent to fix it" prompt.
export type CheckState = "idle" | "running" | "pass" | "fail";

export function checkState(action: { running: boolean; exit_code: number | null; terminal_id: string | null }): CheckState {
  if (action.running) return "running";
  if (action.exit_code == null) return "idle";
  return action.exit_code === 0 ? "pass" : "fail";
}

const ANSI = /\u001b(?:\[[0-9;?]*[ -/]*[@-~]|\][^\u0007\u001b]*(?:\u0007|\u001b\\)|[()][A-Za-z0-9]|[=>])/g;

export function stripAnsi(text: string): string {
  return text.replace(ANSI, "").replace(/\r(?!\n)/g, "\n");
}

export interface FileRef {
  path: string;
  line: number;
  column: number | null;
}

// `path:line[:col]` (rustc, pytest, go, eslint-stylish -> after a path line), `path(line,col)` (tsc).
const COLON_REF = /(?:^|[\s("'`>])((?:\.{0,2}\/)?[\w@.+~-]+(?:\/[\w@.+~-]+)*\.[A-Za-z][A-Za-z0-9]{0,5}):(\d+)(?::(\d+))?/g;
const PAREN_REF = /(?:^|[\s("'`>])((?:\.{0,2}\/)?[\w@.+~-]+(?:\/[\w@.+~-]+)*\.[A-Za-z][A-Za-z0-9]{0,5})\((\d+)(?:,(\d+))?\)/g;

function refsIn(line: string): FileRef[] {
  const out: FileRef[] = [];
  for (const re of [COLON_REF, PAREN_REF]) {
    re.lastIndex = 0;
    for (let m = re.exec(line); m; m = re.exec(line)) {
      const path = m[1].replace(/^\.\//, "");
      // Version strings and hosts look like file refs ("v1.2:3", "localhost:8080").
      if (/^\d+(\.\d+)+$/.test(path)) continue;
      out.push({ path, line: Number(m[2]), column: m[3] ? Number(m[3]) : null });
    }
  }
  return out;
}

export function parseFileRefs(text: string, max = 200): FileRef[] {
  const out: FileRef[] = [];
  const seen = new Set<string>();
  for (const line of stripAnsi(text).split("\n")) {
    for (const ref of refsIn(line)) {
      const key = `${ref.path}:${ref.line}`;
      if (seen.has(key)) continue;
      seen.add(key);
      out.push(ref);
      if (out.length >= max) return out;
    }
  }
  return out;
}

/** A ref names a changed file when one path is the other's suffix (actions can run in a subdir). */
export function samePath(changed: string, ref: string): boolean {
  if (changed === ref) return true;
  return changed.endsWith(`/${ref}`) || ref.endsWith(`/${changed}`);
}

/** The changed files a failing check's output points at. */
export function failingChangedFiles(changedPaths: readonly string[], refs: readonly FileRef[]): string[] {
  return changedPaths.filter((path) => refs.some((ref) => samePath(path, ref.path)));
}

/** Resolve a ref to the path the editor should open, preferring a changed file. */
export function resolveRefPath(ref: FileRef, changedPaths: readonly string[]): string {
  return changedPaths.find((path) => samePath(path, ref.path)) ?? ref.path;
}

const FIX_TAIL_LINES = 60;

export function tailLines(text: string, count = FIX_TAIL_LINES): string {
  const lines = stripAnsi(text).split("\n");
  return lines.slice(Math.max(0, lines.length - count)).join("\n").trim();
}

export function fixPrompt(name: string, command: string, exitCode: number | null, output: string): string {
  const code = exitCode == null ? "" : ` (exit ${exitCode})`;
  return `The "${name}" check failed${code}. Command: ${command}\n\nLast output:\n\n${tailLines(output)}\n\nPlease fix what is failing and run it again.`;
}

export interface LineSegment {
  text: string;
  ref: FileRef | null;
}

/** One output line split so the first `path:line` in it can be a link. */
export function splitLineRefs(line: string): LineSegment[] {
  const clean = stripAnsi(line);
  const [ref] = refsIn(clean);
  if (!ref) return [{ text: clean, ref: null }];
  const needle = clean.indexOf(ref.path);
  if (needle < 0) return [{ text: clean, ref: null }];
  const tail = clean.slice(needle + ref.path.length);
  const suffix = /^(?::\d+(?::\d+)?|\(\d+(?:,\d+)?\))/.exec(tail)?.[0] ?? "";
  const end = needle + ref.path.length + suffix.length;
  const out: LineSegment[] = [];
  if (needle > 0) out.push({ text: clean.slice(0, needle), ref: null });
  out.push({ text: clean.slice(needle, end), ref });
  if (end < clean.length) out.push({ text: clean.slice(end), ref: null });
  return out;
}

const CHECK_NAME = /\b(test|tests|lint|build|check|clippy|fmt|format|tsc|typecheck|types|vitest|jest|pytest|ci)\b/i;

/** Project actions that are checks (test, lint, build). If none look like one, every action is shown. */
export function checkActions<T extends { id: string; name: string; command: string }>(actions: readonly T[]): T[] {
  const checks = actions.filter((action) => CHECK_NAME.test(`${action.id} ${action.name} ${action.command}`));
  return checks.length > 0 ? checks : [...actions];
}

export const OUTPUT_LIMIT = 200_000;

export function appendOutput(buffer: string, chunk: string): string {
  const next = buffer + chunk;
  return next.length > OUTPUT_LIMIT ? next.slice(next.length - OUTPUT_LIMIT) : next;
}
