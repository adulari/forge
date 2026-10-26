import { describe, expect, it } from "vitest";

import { formatJobNotice, isJobNotice } from "./jobNotice";

describe("formatJobNotice", () => {
  it("renders a header per job with its tail indented", () => {
    const text = [
      "[2 earlier background-job notice(s) were dropped; check `shell_job` list]",
      "",
      "[background job 9 `make` exited 2]",
      "last lines:",
      "error: boom",
      "",
      "[background job 3429249 `/tmp/batch.sh > /tmp/batch_result.log 2>&1` exited 0]",
    ].join("\n");
    expect(formatJobNotice(text)).toBe(
      [
        "⚙ 2 earlier job notice(s) dropped",
        "⚙ job 9 ✗ exited 2 · make",
        "    error: boom",
        "⚙ job 3429249 ✓ exited 0 · /tmp/batch.sh > /tmp/batch_result.log 2>&1",
      ].join("\n"),
    );
  });

  it("is keyed on the host's kind tag", () => {
    expect(isJobNotice("job_notice")).toBe(true);
    expect(isJobNotice("user")).toBe(false);
    expect(isJobNotice(undefined)).toBe(false);
  });
});

import { jobCommandText, jobLabel, jobLogPath, jobOutcome, parseJobNotices } from "./jobNotice";

describe("job notice rows", () => {
  const exec =
    "exec /home/floris/bin/agentlive-exec /mnt/work/agentlive/logs/2026-10-26/f3790cb8eb364e3e.log-- 'cargo test -p forge-core …";

  it("structures notices, tails and the dropped marker", () => {
    const items = parseJobNotices(
      ["[background job 9 `make` exited 2]", "last lines:", "error: boom", "", "[3 earlier background-job notice(s) were dropped; check `shell_job` list]"].join("\n"),
    );
    expect(items).toEqual([
      { type: "exit", pid: "9", command: "make", code: 2, tail: ["error: boom"] },
      { type: "dropped", count: 3 },
    ]);
  });

  it("labels a job by its program, not its command line", () => {
    expect(jobLabel(exec)).toBe("agentlive-exec");
    expect(jobLabel("FOO=1 nohup /usr/bin/python3 build.py")).toBe("python3");
    expect(jobLabel("")).toBe("job");
  });

  it("shows the command as run, with only the home folded", () => {
    expect(jobCommandText(exec)).toBe(
      "~/bin/agentlive-exec /mnt/work/agentlive/logs/2026-10-26/f3790cb8eb364e3e.log-- 'cargo test -p forge-core …",
    );
  });

  it("finds the log a command writes", () => {
    expect(jobLogPath(exec)).toBe("/mnt/work/agentlive/logs/2026-10-26/f3790cb8eb364e3e.log");
    expect(jobLogPath("/tmp/batch.sh > /tmp/batch_result.log 2>&1")).toBe("/tmp/batch_result.log");
    expect(jobLogPath("make")).toBeNull();
  });

  it("only mentions the outcome when it is not success", () => {
    expect(jobOutcome(0)).toBeNull();
    expect(jobOutcome(2)).toBe("exit 2");
    expect(jobOutcome(-1)).toBe("killed");
  });
});
