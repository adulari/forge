// Pure state logic for the desktop onboarding screens: following jobs, the "set up everything
// recommended" checklist, phase 0 (install Forge, start the daemon), the step rail and the
// "what we found" lines each step leads with. No React, no network: the screens feed events in
// and render what comes out, which keeps every transition unit-testable.
import type {
  Action,
  ActionResult,
  JobEvent,
  JobPoll,
  Machine,
  PlanStep,
  PromptKind,
  Recommended,
  Snapshot,
  StepId,
} from "./types";

// ---------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------

const MAX_LINES = 60;

export interface JobView {
  id: number;
  label: string;
  state: "running" | "succeeded" | "failed";
  lines: string[];
  progress: { label: string; fraction: number | null } | null;
  prompt: { kind: PromptKind; message: string } | null;
  summary: string | null;
  error: string | null;
  result: ActionResult | null;
  /** Next event position to ask the daemon for. */
  cursor: number;
}

export function newJobView(id: number, label: string): JobView {
  return {
    id,
    label,
    state: "running",
    lines: [],
    progress: null,
    prompt: null,
    summary: null,
    error: null,
    result: null,
    cursor: 0,
  };
}

function applyEvent(view: JobView, event: JobEvent): JobView {
  switch (event.type) {
    case "started":
      return view;
    case "output":
      return { ...view, lines: [...view.lines, event.line].slice(-MAX_LINES) };
    case "progress":
      return { ...view, progress: { label: event.label, fraction: event.fraction } };
    case "prompt":
      return { ...view, prompt: { kind: event.prompt, message: event.message } };
    case "done":
      return {
        ...view,
        state: "succeeded",
        prompt: null,
        progress: null,
        summary: event.summary,
        result: event.result,
      };
    case "failed":
      return { ...view, state: "failed", prompt: null, progress: null, error: event.error };
  }
}

/** Fold one poll reply into the view. Replies that start before the view's cursor (a retried
 * request) are ignored, so an event is never applied twice. */
export function applyJobPoll(view: JobView, poll: JobPoll): JobView {
  if (poll.next < view.cursor) return view;
  const skip = Math.max(0, view.cursor - (poll.next - poll.events.length));
  let next = { ...view, cursor: poll.next };
  for (const event of poll.events.slice(skip)) next = applyEvent(next, event);
  return next;
}

export function jobFinished(view: JobView | null | undefined): boolean {
  return view != null && view.state !== "running";
}

/** A finished job whose live check was refused, or that could not capture a credential. */
export function jobNeedsAttention(view: JobView): boolean {
  if (view.state === "failed") return true;
  const result = view.result;
  return result != null && (
    (result.kind === "verified" && result.status === "rejected") || result.kind === "paste_needed"
  );
}

// ---------------------------------------------------------------------------
// Set up everything recommended
// ---------------------------------------------------------------------------

export type ChecklistStatus = "pending" | "running" | "waiting" | "done" | "failed";

export interface ChecklistItem {
  key: string;
  title: string;
  step: StepId;
  action: Action | null;
  /** A sign-in or a real choice: the run stops here until the person has done it. */
  needsYou: boolean;
  status: ChecklistStatus;
  job: number | null;
  note: string | null;
}

export function buildChecklist(items: Recommended[]): ChecklistItem[] {
  return items.map((item, index) => ({
    key: `${index}:${item.action?.action ?? "choose"}`,
    title: item.title,
    step: item.step,
    action: item.action,
    needsYou: item.needs_you,
    status: "pending",
    job: null,
    note: null,
  }));
}

export function checklistProgress(list: ChecklistItem[]): { done: number; total: number } {
  return { done: list.filter((i) => i.status === "done").length, total: list.length };
}

/** Index of the next item the run should start, or -1 when it is finished or blocked. A
 * choice with no action, or a sign-in that did not finish, blocks the items after it. */
export function nextToRun(list: ChecklistItem[]): number {
  for (let i = 0; i < list.length; i += 1) {
    const item = list[i];
    if (item.status === "pending") return item.action ? i : -1;
    if (item.status === "running" || item.status === "waiting") return -1;
    if (item.status === "failed" && item.needsYou) return -1;
  }
  return -1;
}

/** The checklist is blocked on the person: a choice to make, or a sign-in to finish. */
export function checklistBlockedOn(list: ChecklistItem[]): ChecklistItem | null {
  for (const item of list) {
    if (item.status === "pending" && !item.action) return item;
    if (item.status === "waiting") return item;
    if (item.status === "failed" && item.needsYou) return item;
  }
  return null;
}

export function checklistFinished(list: ChecklistItem[]): boolean {
  return list.every((i) => i.status === "done" || i.status === "failed");
}

/** Update the item a job belongs to from the job's view. */
export function applyChecklistJob(list: ChecklistItem[], index: number, view: JobView): ChecklistItem[] {
  return list.map((item, i) => {
    if (i !== index) return item;
    if (view.state === "running") {
      return { ...item, job: view.id, status: view.prompt ? "waiting" : "running", note: null };
    }
    if (jobNeedsAttention(view)) {
      return { ...item, job: view.id, status: "failed", note: failureNote(view) };
    }
    return { ...item, job: view.id, status: "done", note: view.summary };
  });
}

/** The line of a failed command's output that says what went wrong: the last one marked with a
 * cross or "error", else the last non-empty line. Keeps a 6-line CLI report to one sentence. */
export function failureSummary(text: string): string {
  const lines = text
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);
  const marked = [...lines].reverse().find((l) => /^(✗|x\s|error[:\s])/i.test(l));
  const line = marked ?? lines.at(-1) ?? text;
  return line.replace(/^(✗|x)\s+/i, "").trim();
}

function failureNote(view: JobView): string {
  if (view.error) return failureSummary(view.error);
  const result = view.result;
  if (result?.kind === "verified" && result.status === "rejected") {
    return result.detail ?? "The check did not pass.";
  }
  if (result?.kind === "paste_needed") return "Paste the token it could not capture.";
  return "That did not finish.";
}

// ---------------------------------------------------------------------------
// Phase 0: install Forge, start the daemon, connect
// ---------------------------------------------------------------------------

export type Phase0Key = "install" | "service" | "connect";
export type Phase0Status = "skipped" | "pending" | "running" | "done" | "failed";

export interface Phase0Row {
  key: Phase0Key;
  title: string;
  status: Phase0Status;
  message: string | null;
  fraction: number | null;
}

/** What `onboarding_local_status` reports, as much of it as phase 0 needs. */
export interface LocalStatusLike {
  forge_binary: string | null;
  serve: unknown | null;
}

/** `paired`: the app already has a server to talk to (so only a missing daemon needs work). */
export function phase0Plan(status: LocalStatusLike | null, paired = false): Phase0Row[] {
  const missingBinary = status != null && status.forge_binary == null;
  const noDaemon = status != null && status.serve == null;
  const needsConnect = status != null && (noDaemon || !paired);
  const row = (key: Phase0Key, title: string, needed: boolean): Phase0Row => ({
    key,
    title,
    status: needed ? "pending" : "skipped",
    message: null,
    fraction: null,
  });
  return [
    row("install", "Install Forge", missingBinary),
    row("service", "Start Forge", noDaemon),
    row("connect", "Connect this app", needsConnect),
  ];
}

/** Whether a server base URL points at this computer. */
export function isLoopbackBase(baseUrl: string): boolean {
  try {
    const host = new URL(baseUrl).hostname.replace(/^\[|\]$/g, "");
    return host === "127.0.0.1" || host === "localhost" || host === "::1";
  } catch {
    return false;
  }
}

export function phase0Needed(rows: Phase0Row[]): boolean {
  return rows.some((r) => r.status !== "skipped");
}

export function phase0Update(
  rows: Phase0Row[],
  key: Phase0Key,
  patch: Partial<Pick<Phase0Row, "status" | "message" | "fraction">>,
): Phase0Row[] {
  return rows.map((r) => (r.key === key ? { ...r, ...patch } : r));
}

export interface InstallProgress {
  phase: string;
  fraction?: number | null;
  message: string;
}

/** Fold a progress event from the command that is running (`key` says which one) into the rows. */
export function phase0FromProgress(rows: Phase0Row[], event: InstallProgress, key: "install" | "service"): Phase0Row[] {
  if (event.phase === "failed") {
    return phase0Update(rows, key, { status: "failed", message: event.message });
  }
  if (event.phase === "done") {
    return phase0Update(rows, key, { status: "done", message: event.message, fraction: 1 });
  }
  return phase0Update(rows, key, {
    status: "running",
    message: event.message,
    fraction: event.fraction ?? null,
  });
}

export function phase0Active(rows: Phase0Row[]): Phase0Row | null {
  return rows.find((r) => r.status === "running" || r.status === "failed") ?? null;
}

// ---------------------------------------------------------------------------
// Step rail
// ---------------------------------------------------------------------------

export type RailStatus = "done" | "current" | "todo" | "skipped";

export interface RailEntry {
  key: string;
  label: string;
  status: RailStatus;
  /** A required step that still blocks "ready". */
  required: boolean;
  /** Added in a newer flow than the one this machine last finished. */
  isNew: boolean;
}

/** "New" only means something to a machine that finished an earlier flow; on a first run every
 * step is new and the badge is noise. */
export function showsNew(step: PlanStep, snapshot: Snapshot): boolean {
  return step.new && snapshot.state.flow_version > 0;
}

export function railEntries(snapshot: Snapshot, currentIndex: number): RailEntry[] {
  return snapshot.plan.map((step, index) => {
    const blocking = snapshot.missing.includes(step.id);
    let status: RailStatus = "todo";
    if (index === currentIndex) status = "current";
    else if (step.status === "done" || (step.satisfied && !blocking)) status = "done";
    else if (step.status === "skipped") status = "skipped";
    return {
      key: step.id,
      label: step.title,
      status,
      required: step.required && blocking,
      isNew: showsNew(step, snapshot),
    };
  });
}

/** The first step a person has not dealt with, after `from`; the last step when none is left. */
export function nextStepIndex(plan: PlanStep[], from: number): number {
  return Math.min(Math.max(from + 1, 0), Math.max(plan.length - 1, 0));
}

export function stepIndexOf(plan: PlanStep[], id: StepId): number {
  return Math.max(0, plan.findIndex((s) => s.id === id));
}

// ---------------------------------------------------------------------------
// What each step leads with
// ---------------------------------------------------------------------------

function plural(count: number, one: string, many: string): string {
  return `${count} ${count === 1 ? one : many}`;
}

export function readySources(snapshot: Snapshot): string[] {
  return snapshot.sources.filter((s) => s.state === "ready").map((s) => s.name);
}

export function importSummary(machine: Machine): string[] {
  const i = machine.imports;
  const tools = [i.claude_dir && "Claude Code", i.codex_dir && "Codex", i.cursor_dir && "Cursor"].filter(
    (t): t is string => typeof t === "string",
  );
  const lines: string[] = [];
  if (tools.length > 0) lines.push(`Found ${tools.join(", ")} on this computer.`);
  const counted = [
    i.commands > 0 ? plural(i.commands, "command", "commands") : null,
    i.skills > 0 ? plural(i.skills, "skill", "skills") : null,
    i.mcp_servers > 0 ? plural(i.mcp_servers, "MCP server", "MCP servers") : null,
  ].filter((t): t is string => t != null);
  if (counted.length > 0) lines.push(`${counted.join(", ")} can come along.`);
  return lines;
}

/** The "what we found" lines a step opens with. Plain sentences, no jargon. */
export function detectedLines(id: StepId, snapshot: Snapshot): string[] {
  const m = snapshot.machine;
  const ready = readySources(snapshot);
  switch (id) {
    case "install_cli":
      return ["Forge is not installed on this computer yet."];
    case "welcome": {
      const lines: string[] = [];
      if (snapshot.claude_reconnect) {
        lines.push("Claude Code is installed and signed in, but Forge needs its own connection to it.");
      }
      lines.push(
        ready.length > 0
          ? `${plural(ready.length, "model source is", "model sources are")} ready: ${ready.join(", ")}.`
          : "No model is connected yet.",
      );
      lines.push(...importSummary(m));
      return lines;
    }
    case "models":
      return ready.length > 0
        ? [`Ready: ${ready.join(", ")}.`]
        : ["Nothing is connected yet. Pick the one you already pay for, or a free one."];
    case "behavior": {
      const s = m.settings;
      return [`Right now: ${permissionLabel(s.permission)}, ${creditLabel(s.credit_mode).toLowerCase()}.`];
    }
    case "web_search": {
      if (m.search.brave_key) return ["A Brave Search key is set."];
      if (m.search.searxng_running) return ["A local search engine is running."];
      return [m.search.docker
        ? "Search already works with no key. Docker is here, so a private local search engine is one click."
        : "Search already works with no key."];
    }
    case "benchmarks":
      return [m.benchmarks.own_key
        ? "Your own Artificial Analysis key is set and overrides the shared data."
        : "Forge uses shared benchmark data. No key needed."];
    case "import": {
      const lines = importSummary(m);
      return lines.length > 0 ? lines : ["Nothing to bring over."];
    }
    case "service":
      if (m.service.installed && m.service.running) return ["The background service is installed and running."];
      if (m.service.installed) return ["The background service is installed but not running."];
      return [m.service.supported
        ? "The background service is not installed."
        : "This system has no background service support."];
    case "anywhere":
      return [m.anywhere.signed_in ? "You are signed in to Forge Anywhere." : "Not signed in to Forge Anywhere."];
    case "surfaces":
      return ["The desktop app is running. Your phone can pair with it."];
    case "preferences":
      return [`Notifications are ${m.settings.notifications ? "on" : "off"}. Theme: ${m.settings.theme || "auto"}.`];
    case "reporting":
      return [m.settings.reporting_chosen
        ? `Error reports are set to ${m.settings.reporting}.`
        : "You have not chosen yet. The default is automatic scrubbed reports."];
    case "done":
      return ready.length > 0 ? [`Ready: ${ready.join(", ")}.`] : ["No model is connected yet."];
  }
}

export const PERMISSION_CHOICES = [
  { value: "plan", label: "Read-only", example: "Forge looks and plans, and never changes a file." },
  { value: "default", label: "Ask first", example: "Forge asks before it edits a file or runs a command." },
  { value: "accept-edits", label: "Auto-edit", example: "Forge edits files freely and asks before running commands." },
  { value: "auto", label: "Auto", example: "Forge decides what is safe and asks only about the risky parts." },
] as const;

export const CREDIT_CHOICES = [
  { value: "normal", label: "Balanced", example: "Each task goes to the best model you have." },
  { value: "frugal", label: "Frugal", example: "Prefers free and cheaper models, and saves your subscription." },
  { value: "strict", label: "Strict", example: "Never spends paid credits without asking first." },
] as const;

export function permissionLabel(key: string): string {
  return PERMISSION_CHOICES.find((c) => c.value === key)?.label.toLowerCase() ?? "ask first";
}

export function creditLabel(key: string): string {
  return CREDIT_CHOICES.find((c) => c.value === key)?.label ?? "Balanced";
}

// ---------------------------------------------------------------------------
// Opening and leaving
// ---------------------------------------------------------------------------

let dismissedThisLaunch = false;

/** The person left setup (finished it or chose "skip for now"): do not send them back to it
 * until the app restarts. */
export function dismissOnboardingForThisLaunch(): void {
  dismissedThisLaunch = true;
}

export function onboardingDismissed(): boolean {
  return dismissedThisLaunch;
}

/** Test seam. */
export function resetOnboardingDismissal(): void {
  dismissedThisLaunch = false;
}
