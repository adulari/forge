import { describe, expect, it } from "vitest";

import {
  applyChecklistJob,
  applyJobPoll,
  buildChecklist,
  checklistBlockedOn,
  checklistFinished,
  checklistProgress,
  detectedLines,
  failureSummary,
  isLoopbackBase,
  showsNew,
  dismissOnboardingForThisLaunch,
  jobNeedsAttention,
  newJobView,
  nextStepIndex,
  nextToRun,
  onboardingDismissed,
  phase0Active,
  phase0FromProgress,
  phase0Needed,
  phase0Plan,
  railEntries,
  resetOnboardingDismissal,
} from "./flow";
import type { JobEvent, JobPoll, Machine, PlanStep, Recommended, Snapshot, StepId } from "./types";

function poll(events: JobEvent[], next: number, state: JobPoll["state"] = "running"): JobPoll {
  return { job: 1, action: "x", state, events, next };
}

describe("job views", () => {
  it("folds output, progress, prompts and the end into one view", () => {
    let view = newJobView(1, "Connecting Claude Code");
    view = applyJobPoll(
      view,
      poll(
        [
          { type: "started", job: 1, action: "Connecting Claude Code" },
          { type: "output", job: 1, line: "Opening the sign-in page" },
          { type: "progress", job: 1, label: "Waiting", fraction: 0.5 },
          {
            type: "prompt",
            job: 1,
            prompt: { kind: "open_url", url: "https://claude.ai/oauth/x" },
            message: "Open this page",
          },
        ],
        4,
      ),
    );
    expect(view.lines).toEqual(["Opening the sign-in page"]);
    expect(view.progress).toEqual({ label: "Waiting", fraction: 0.5 });
    expect(view.prompt?.kind).toEqual({ kind: "open_url", url: "https://claude.ai/oauth/x" });
    expect(view.state).toBe("running");

    view = applyJobPoll(
      view,
      poll([{ type: "done", job: 1, summary: "Connected.", result: { kind: "changed" } }], 5, "succeeded"),
    );
    expect(view.state).toBe("succeeded");
    expect(view.prompt).toBeNull();
    expect(view.summary).toBe("Connected.");
    expect(view.cursor).toBe(5);
  });

  it("never applies an event twice when a reply is repeated or overlaps", () => {
    let view = newJobView(1, "x");
    const first = poll([{ type: "output", job: 1, line: "a" }, { type: "output", job: 1, line: "b" }], 2);
    view = applyJobPoll(view, first);
    view = applyJobPoll(view, first);
    expect(view.lines).toEqual(["a", "b"]);
    view = applyJobPoll(
      view,
      poll([{ type: "output", job: 1, line: "b" }, { type: "output", job: 1, line: "c" }], 3),
    );
    expect(view.lines).toEqual(["a", "b", "c"]);
  });

  it("keeps only the most recent output lines", () => {
    let view = newJobView(1, "x");
    const events: JobEvent[] = Array.from({ length: 100 }, (_, i) => ({
      type: "output",
      job: 1,
      line: `line ${i}`,
    }));
    view = applyJobPoll(view, poll(events, 100));
    expect(view.lines).toHaveLength(60);
    expect(view.lines.at(-1)).toBe("line 99");
  });

  it("flags a failure, a refused key and a missing paste as needing attention", () => {
    const failed = applyJobPoll(newJobView(1, "x"), poll([{ type: "failed", job: 1, error: "nope" }], 1, "failed"));
    expect(jobNeedsAttention(failed)).toBe(true);
    const rejected = applyJobPoll(
      newJobView(1, "x"),
      poll(
        [{ type: "done", job: 1, summary: "", result: { kind: "verified", source: "groq", status: "rejected", detail: "refused" } }],
        1,
        "succeeded",
      ),
    );
    expect(jobNeedsAttention(rejected)).toBe(true);
    const paste = applyJobPoll(
      newJobView(1, "x"),
      poll([{ type: "done", job: 1, summary: "", result: { kind: "paste_needed", slot: "claude_oauth_token" } }], 1, "succeeded"),
    );
    expect(jobNeedsAttention(paste)).toBe(true);
    const fine = applyJobPoll(
      newJobView(1, "x"),
      poll([{ type: "done", job: 1, summary: "ok", result: { kind: "verified", source: "groq", status: "unchecked", detail: null } }], 1, "succeeded"),
    );
    expect(jobNeedsAttention(fine)).toBe(false);
  });
});

const item = (title: string, step: StepId, action: Recommended["action"], needs_you = false): Recommended => ({
  title,
  step,
  action,
  needs_you,
});

describe("the recommended checklist", () => {
  const items: Recommended[] = [
    item("Reconnect Claude Code", "models", { action: "connect_claude" }, true),
    item("Keep the safe defaults", "behavior", { action: "set_behavior", permission: "default", credit_mode: "normal" }),
    item("Install the background service", "service", { action: "install_service", exposure: "local" }),
  ];

  it("starts at the first item and moves on only after each finishes", () => {
    let list = buildChecklist(items);
    expect(nextToRun(list)).toBe(0);
    const running = applyJobPoll(newJobView(7, "x"), poll([{ type: "output", job: 7, line: "..." }], 1));
    list = applyChecklistJob(list, 0, running);
    expect(list[0].status).toBe("running");
    expect(nextToRun(list)).toBe(-1);

    const done = applyJobPoll(running, poll([{ type: "done", job: 7, summary: "Connected.", result: { kind: "changed" } }], 2, "succeeded"));
    list = applyChecklistJob(list, 0, done);
    expect(list[0]).toMatchObject({ status: "done", job: 7, note: "Connected." });
    expect(nextToRun(list)).toBe(1);
    expect(checklistProgress(list)).toEqual({ done: 1, total: 3 });
  });

  it("waits on a sign-in prompt and blocks on a sign-in that failed", () => {
    let list = buildChecklist(items);
    const prompting = applyJobPoll(
      newJobView(7, "x"),
      poll([{ type: "prompt", job: 7, prompt: { kind: "device_code", url: "https://x", code: "ABCD" }, message: "m" }], 1),
    );
    list = applyChecklistJob(list, 0, prompting);
    expect(list[0].status).toBe("waiting");
    expect(checklistBlockedOn(list)?.title).toBe("Reconnect Claude Code");

    const failed = applyJobPoll(prompting, poll([{ type: "failed", job: 7, error: "timed out" }], 2, "failed"));
    list = applyChecklistJob(list, 0, failed);
    expect(list[0]).toMatchObject({ status: "failed", note: "timed out" });
    expect(nextToRun(list)).toBe(-1);
  });

  it("keeps going past a failed item that did not need the person", () => {
    let list = buildChecklist(items);
    list = applyChecklistJob(list, 0, applyJobPoll(newJobView(1, "x"), poll([{ type: "done", job: 1, summary: "", result: { kind: "changed" } }], 1, "succeeded")));
    list = applyChecklistJob(list, 1, applyJobPoll(newJobView(2, "x"), poll([{ type: "failed", job: 2, error: "disk full" }], 1, "failed")));
    expect(nextToRun(list)).toBe(2);
  });

  it("stops at a choice that has no action", () => {
    const list = buildChecklist([item("Connect a model", "models", null, true), items[1]]);
    expect(nextToRun(list)).toBe(-1);
    expect(checklistBlockedOn(list)?.action).toBeNull();
    expect(checklistFinished(list)).toBe(false);
  });

  it("is finished when every item is done or failed", () => {
    expect(checklistFinished([])).toBe(true);
    let list = buildChecklist(items.slice(1));
    list = applyChecklistJob(list, 0, applyJobPoll(newJobView(1, "x"), poll([{ type: "done", job: 1, summary: "", result: null }], 1, "succeeded")));
    list = applyChecklistJob(list, 1, applyJobPoll(newJobView(2, "x"), poll([{ type: "failed", job: 2, error: "e" }], 1, "failed")));
    expect(checklistFinished(list)).toBe(true);
  });
});

describe("phase 0", () => {
  it("needs the install and the daemon on a bare machine", () => {
    const rows = phase0Plan({ forge_binary: null, serve: null });
    expect(rows.map((r) => [r.key, r.status])).toEqual([
      ["install", "pending"],
      ["service", "pending"],
      ["connect", "pending"],
    ]);
    expect(phase0Needed(rows)).toBe(true);
  });

  it("skips the install when forge is there and everything when a daemon is already live", () => {
    const noDaemon = phase0Plan({ forge_binary: "/usr/bin/forge", serve: null });
    expect(noDaemon[0].status).toBe("skipped");
    expect(noDaemon[1].status).toBe("pending");
    const live = phase0Plan({ forge_binary: "/usr/bin/forge", serve: { port: 1 } }, true);
    expect(phase0Needed(live)).toBe(false);
    const unpaired = phase0Plan({ forge_binary: "/usr/bin/forge", serve: { port: 1 } }, false);
    expect(unpaired.map((r) => r.status)).toEqual(["skipped", "skipped", "pending"]);
  });

  it("maps install and service progress events onto their rows", () => {
    let rows = phase0Plan({ forge_binary: null, serve: null });
    rows = phase0FromProgress(rows, { phase: "download", fraction: 0.4, message: "Downloading" }, "install");
    expect(rows[0]).toMatchObject({ status: "running", fraction: 0.4, message: "Downloading" });
    rows = phase0FromProgress(rows, { phase: "done", message: "Installed forge 2.24.0" }, "install");
    expect(rows[0].status).toBe("done");
    rows = phase0FromProgress(rows, { phase: "service-install", message: "Installing" }, "service");
    expect(rows[1].status).toBe("running");
    expect(phase0Active(rows)?.key).toBe("service");
    rows = phase0FromProgress(rows, { phase: "failed", message: "Could not start" }, "service");
    expect(rows[1]).toMatchObject({ status: "failed", message: "Could not start" });
  });
});

function step(id: StepId, patch: Partial<PlanStep> = {}): PlanStep {
  return { id, title: id, summary: "", required: false, status: "pending", new: false, satisfied: false, ...patch };
}

function snapshot(patch: Partial<Snapshot> = {}): Snapshot {
  const machine = {
    imports: { claude_dir: true, codex_dir: false, cursor_dir: false, commands: 4, skills: 1, mcp_servers: 0, keybinds_active: false },
    search: { brave_key: false, searxng_running: false, docker: true },
    benchmarks: { own_key: false },
    service: { supported: true, installed: false, running: false },
    anywhere: { signed_in: false },
    settings: { permission: "default", credit_mode: "normal", keybind_preset: "forge", notifications: false, reporting: "auto", reporting_chosen: false, theme: "auto" },
  } as unknown as Machine;
  return {
    surface: "desktop",
    mode: "resume",
    complete: false,
    missing: ["models"],
    flow_version: 1,
    start_index: 0,
    plan: [step("welcome"), step("models", { required: true }), step("behavior", { status: "done" }), step("web_search", { status: "skipped" }), step("service", { required: true, satisfied: true })],
    state: {} as Snapshot["state"],
    machine,
    sources: [{ id: "claude-cli", name: "Claude Code", kind: "claude_code", state: "ready", detail: "", free_tier: false, key_url: null }],
    recommended: [],
    claude_reconnect: false,
    ...patch,
  };
}

describe("the step rail", () => {
  it("marks the current step, finished steps, skipped steps and what still blocks", () => {
    const entries = railEntries(snapshot(), 1);
    expect(entries.map((e) => [e.key, e.status, e.required])).toEqual([
      ["welcome", "todo", false],
      ["models", "current", true],
      ["behavior", "done", false],
      ["web_search", "skipped", false],
      ["service", "done", false],
    ]);
  });

  it("does not call a satisfied step done while it still blocks readiness", () => {
    const entries = railEntries(
      snapshot({ missing: ["service"], plan: [step("service", { required: true, satisfied: true })] }),
      5,
    );
    expect(entries[0]).toMatchObject({ status: "todo", required: true });
  });

  it("moves forward and stops at the last step", () => {
    const plan = snapshot().plan;
    expect(nextStepIndex(plan, 0)).toBe(1);
    expect(nextStepIndex(plan, plan.length - 1)).toBe(plan.length - 1);
    expect(nextStepIndex([], 0)).toBe(0);
  });
});

describe("what each step leads with", () => {
  it("summarises what the machine already has", () => {
    const s = snapshot();
    expect(detectedLines("welcome", s)[0]).toContain("Claude Code");
    expect(detectedLines("welcome", s).join(" ")).toContain("4 commands, 1 skill");
    expect(detectedLines("web_search", s)[0]).toContain("Docker");
    expect(detectedLines("service", s)[0]).toContain("not installed");
    expect(detectedLines("reporting", s)[0]).toContain("not chosen");
  });

  it("leads with the reconnect when Claude Code needs a token", () => {
    const lines = detectedLines("welcome", snapshot({ claude_reconnect: true }));
    expect(lines[0]).toContain("needs its own connection");
  });

  it("says plainly that nothing is connected", () => {
    expect(detectedLines("models", snapshot({ sources: [] }))[0]).toContain("Nothing is connected");
  });
});

describe("failure summaries", () => {
  it("keeps the line that says what went wrong", () => {
    const report = [
      "installed forge-serve (systemd)",
      "  restarted: running process could not be identified",
      "✗ the unit was written but the daemon is not running it",
      "  Next: run `forge doctor` for a health check",
    ].join("\n");
    expect(failureSummary(report)).toBe("the unit was written but the daemon is not running it");
    expect(failureSummary("just one line")).toBe("just one line");
    expect(failureSummary("a\nb\n")).toBe("b");
  });
});

describe("new badges", () => {
  it("show only on a machine that finished an earlier flow", () => {
    const first = snapshot();
    first.state = { flow_version: 0 } as Snapshot["state"];
    const later = snapshot();
    later.state = { flow_version: 1 } as Snapshot["state"];
    const fresh = step("models", { new: true });
    expect(showsNew(fresh, first)).toBe(false);
    expect(showsNew(fresh, later)).toBe(true);
    expect(showsNew(step("models"), later)).toBe(false);
  });
});

describe("loopback servers", () => {
  it("recognises this computer, and nothing that merely contains it", () => {
    for (const url of ["http://127.0.0.1:7420/tok", "http://localhost:7601/tok", "http://[::1]:7420/tok"]) {
      expect(isLoopbackBase(url), url).toBe(true);
    }
    for (const url of ["https://abc.trycloudflare.com/tok", "http://192.168.1.5:7420/tok", "http://127.0.0.1.evil.test/tok", "not a url"]) {
      expect(isLoopbackBase(url), url).toBe(false);
    }
  });
});

describe("leaving", () => {
  it("remembers that setup was dismissed for this launch", () => {
    resetOnboardingDismissal();
    expect(onboardingDismissed()).toBe(false);
    dismissOnboardingForThisLaunch();
    expect(onboardingDismissed()).toBe(true);
    resetOnboardingDismissal();
  });
});
