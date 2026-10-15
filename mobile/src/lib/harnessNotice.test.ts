import { describe, expect, it } from "vitest";

import { formatAnyHarnessNotice, formatHarnessNotice, isAnyHarnessNotice, isHarnessNotice } from "./harnessNotice";

describe("formatHarnessNotice", () => {
  it("renders a wakeup with its reason underneath", () => {
    const text =
      "[scheduled wakeup] Check the probe file\nsecond line\n\n(scheduled because: Waiting for the probe answer)";
    expect(formatHarnessNotice(text)).toBe("⏰ wakeup · Check the probe file\n    because: Waiting for the probe answer");
  });

  it("renders a wakeup without a reason", () => {
    expect(formatHarnessNotice("[scheduled wakeup] ping")).toBe("⏰ wakeup · ping");
  });

  it("renders labelled and bare heartbeats", () => {
    expect(formatHarnessNotice("[heartbeat: ci-watch] check CI")).toBe("♻ heartbeat ci-watch · check CI");
    expect(formatHarnessNotice("[heartbeat] check in")).toBe("♻ heartbeat · check in");
  });

  it("drops a peer message's framing and keeps its body", () => {
    const text = '[message from session "a" (3fa91c2d)] This is a peer message.\n---\nplease check\nthanks';
    expect(formatHarnessNotice(text)).toBe('✉ peer message from session "a" (3fa91c2d)\n    please check\n    thanks');
  });

  it("clips long prompts and is keyed on the host's kind tags", () => {
    expect(formatHarnessNotice(`[scheduled wakeup] ${"x".repeat(300)}`).length).toBeLessThan(140);
    expect(isHarnessNotice("harness_notice")).toBe(true);
    expect(isHarnessNotice("job_notice")).toBe(false);
    expect(isAnyHarnessNotice("job_notice")).toBe(true);
    expect(isAnyHarnessNotice("user")).toBe(false);
    expect(formatAnyHarnessNotice("job_notice", "[background job 9 `make` exited 2]")).toContain("job 9");
  });
});
