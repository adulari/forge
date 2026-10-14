// A background-job completion notice ("[background job 7 `make` exited 2]" + optional log tail) is
// stored as a user-role message because the model needs a user turn, but the host tags the row
// `kind: "job_notice"` (remote_projection.rs `map_history_row`). It is Forge reporting, never the
// person's words, so it renders as dim system output via `formatJobNotice`.
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
