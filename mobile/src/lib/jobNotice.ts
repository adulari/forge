// A background-job completion notice ("[background job 7 `make` exited 2]" + optional log tail) is
// stored as a user-role message because the model needs a user turn, but the host tags the row
// `kind: "job_notice"` (remote_projection.rs `map_history_row`). It is Forge reporting, never the
// person's words, so it renders as dim system output via `formatJobNotice`.
import { commandProgram, friendlyCommand } from "./pathDisplay";

const HEADER = /^\[background job (\d+) `(.*)` exited (-?\d+)\]$/;
const DROPPED = /^\[(\d+) earlier background-job notice\(s\) were dropped; check `shell_job` list\]$/;

export function isJobNotice(kind?: string): boolean {
  return kind === "job_notice";
}

export function formatJobNotice(content: string): string {
  const out: string[] = [];
  let inTail = false;
  for (const line of content.split("\n")) {
    const head = HEADER.exec(line);
    const dropped = DROPPED.exec(line);
    if (head) {
      inTail = false;
      out.push(`⚙ job ${head[1]} ${head[3] === "0" ? "✓" : "✗"} exited ${head[3]} · ${head[2]}`);
    } else if (dropped) {
      inTail = false;
      out.push(`⚙ ${dropped[1]} earlier job notice(s) dropped`);
    } else if (line === "last lines:") {
      inTail = true;
    } else if (inTail && line.trim()) {
      out.push(`    ${line}`);
    } else if (!inTail && line.trim()) {
      out.push(line);
    }
  }
  return out.join("\n");
}

export type JobNoticeItem =
  | { type: "exit"; pid: string; command: string; code: number; tail: string[] }
  | { type: "dropped"; count: number };

/** The notices in a `job_notice` row, structured for a row of their own per job (the text form
 * above stays for plain-text surfaces). */
export function parseJobNotices(content: string): JobNoticeItem[] {
  const items: JobNoticeItem[] = [];
  let current: Extract<JobNoticeItem, { type: "exit" }> | null = null;
  let inTail = false;
  for (const line of content.split("\n")) {
    const head = HEADER.exec(line);
    const dropped = DROPPED.exec(line);
    if (head) {
      current = { type: "exit", pid: head[1], command: head[2], code: Number(head[3]), tail: [] };
      items.push(current);
      inTail = false;
    } else if (dropped) {
      items.push({ type: "dropped", count: Number(dropped[1]) });
      current = null;
      inTail = false;
    } else if (line === "last lines:") {
      inTail = true;
    } else if (inTail && current) {
      current.tail.push(line);
    }
  }
  for (const item of items) {
    if (item.type === "exit") while (item.tail.length && !item.tail[item.tail.length - 1].trim()) item.tail.pop();
  }
  return items;
}

/** What a job is called in its row: the program it ran (`agentlive-exec`), not its command line. */
export function jobLabel(command: string): string {
  return commandProgram(command) || "job";
}

/** The command as the expanded row shows it: exactly as run, one line, home folded to `~`. */
export function jobCommandText(command: string): string {
  return friendlyCommand(command);
}

const LOG_PATH = /(?:^|[\s>=])(~?\/[^\s'"`>]+?\.(?:log|out))(?![A-Za-z0-9_.])/;

/** The log file a job writes, when its command names one (`> /x/run.log`, `exec run /x/y.log`). */
export function jobLogPath(command: string): string | null {
  return LOG_PATH.exec(jobCommandText(command))?.[1] ?? null;
}

/** `exit 1`, `killed` for a signal; null when the job succeeded and the row stays quiet. */
export function jobOutcome(code: number): string | null {
  if (code === 0) return null;
  return code < 0 ? "killed" : `exit ${code}`;
}
