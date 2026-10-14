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
