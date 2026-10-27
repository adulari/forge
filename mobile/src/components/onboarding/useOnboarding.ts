// The desktop onboarding controller: phase 0 (install Forge, start the daemon, connect), then
// the daemon's plan, jobs and the "set up everything recommended" run. Screens render what this
// returns; the pure transitions live in lib/onboarding/flow.ts.
import { router } from "expo-router";
import { useCallback, useEffect, useRef, useState } from "react";

import { ApiError } from "../../lib/api";
import { useAuth } from "../../lib/auth";
import {
  installForgeCli,
  installForgeService,
  localStatus,
  type InstallProgress,
} from "../../lib/desktopOnboarding";
import { detectForgeServe, pollForForgeServe } from "../../lib/desktopServe";
import {
  applyChecklistJob,
  applyJobPoll,
  buildChecklist,
  dismissOnboardingForThisLaunch,
  isLoopbackBase,
  jobFinished,
  newJobView,
  nextToRun,
  phase0FromProgress,
  phase0Needed,
  phase0Plan,
  phase0Update,
  stepIndexOf,
  type ChecklistItem,
  type JobView,
  type Phase0Row,
} from "../../lib/onboarding/flow";
import {
  cancelJob,
  getOnboarding,
  pollJob,
  sendJobInput,
  setFlow,
  setStep,
  startAction,
} from "../../lib/onboarding/api";
import { DEFAULT_ONLY_STEPS } from "../../lib/onboarding/defaults";
import type { Action, Snapshot, StepId } from "../../lib/onboarding/types";
import { takePendingRoute } from "../../lib/pendingRoute";

export type OnboardingPhase = "loading" | "phase0" | "daemon" | "unavailable" | "error";

export interface RunOptions {
  step?: StepId;
  /** Which place on screen shows this job (a card, the checklist). */
  slot: string;
  label?: string;
  onUpdate?: (view: JobView) => void;
}

export interface Controller {
  phase: OnboardingPhase;
  error: string | null;
  phase0: Phase0Row[];
  retryPhase0: () => void;
  snapshot: Snapshot | null;
  index: number;
  goTo: (index: number) => void;
  next: () => void;
  back: () => void;
  refresh: () => Promise<Snapshot | null>;
  markDone: (step: StepId) => Promise<void>;
  skip: (step: StepId) => Promise<void>;
  busy: boolean;
  run: (action: Action, options: RunOptions) => Promise<JobView>;
  slotJob: (slot: string) => JobView | null;
  clearSlot: (slot: string) => void;
  sendInput: (job: number, text: string) => Promise<void>;
  cancel: (job: number) => Promise<void>;
  checklist: ChecklistItem[] | null;
  checklistRunning: boolean;
  runRecommended: () => Promise<void>;
  finish: (destination?: string) => Promise<void>;
  later: () => Promise<void>;
}

const LONG_POLL_MS = 8_000;

/** Tauri rejects a command with a plain string, not an Error. */
function message(error: unknown, fallback: string): string {
  if (typeof error === "string" && error.trim()) return error.trim();
  return error instanceof Error && error.message ? error.message : fallback;
}

export function useOnboarding(options: { focus?: string } = {}): Controller {
  const { baseUrl, addServer, isLoading } = useAuth();
  const [phase, setPhase] = useState<OnboardingPhase>("loading");
  const [error, setError] = useState<string | null>(null);
  const [phase0, setPhase0] = useState<Phase0Row[]>([]);
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [index, setIndex] = useState(0);
  const [busy, setBusy] = useState(false);
  const [views, setViews] = useState<Record<number, JobView>>({});
  const [slots, setSlots] = useState<Record<string, number>>({});
  const [checklist, setChecklist] = useState<ChecklistItem[] | null>(null);
  const [checklistRunning, setChecklistRunning] = useState(false);
  const [attempt, setAttempt] = useState(0);

  const baseRef = useRef<string | null>(baseUrl);
  const aborted = useRef(false);
  const started = useRef(false);
  const autoConnect = useRef(false);

  useEffect(() => {
    baseRef.current = baseUrl;
  }, [baseUrl]);

  useEffect(() => {
    aborted.current = false;
    return () => {
      aborted.current = true;
    };
  }, []);

  const load = useCallback(async (base: string, keepIndex: boolean): Promise<Snapshot> => {
    const next = await getOnboarding(base);
    setSnapshot(next);
    if (!keepIndex) {
      const focus = options.focus === "claude" ? stepIndexOf(next.plan, "models") : next.start_index;
      setIndex(focus);
    }
    return next;
  }, [options.focus]);

  // Phase 0 and the first load. Runs once the stored servers are known, and again on "try again".
  useEffect(() => {
    if (isLoading) return;
    if (started.current && attempt === 0) return;
    started.current = true;
    void (async () => {
      const status = await localStatus();
      if (aborted.current) return;
      const base = baseRef.current;
      if (base) {
        try {
          await load(base, false);
          if (!aborted.current) setPhase("daemon");
          return;
        } catch (cause) {
          if (aborted.current) return;
          if (cause instanceof ApiError && cause.status === 403) {
            setPhase("unavailable");
            return;
          }
          // A server on another machine is the person's choice; never swap it for the local one.
          if (!isLoopbackBase(base)) {
            setError(message(cause, "Forge did not answer. Check that it is running, then try again."));
            setPhase("error");
            return;
          }
        }
      }
      // No server, or a stale one on this computer (its daemon restarted with a new token):
      // find, start or install the local daemon and connect to it.
      const rows = phase0Plan(status, false);
      if (!phase0Needed(rows)) {
        if (base) {
          setError("Forge did not answer. Check that it is running, then try again.");
          setPhase("error");
        } else {
          router.replace("/connect");
        }
        return;
      }
      setPhase0(rows);
      setPhase("phase0");
      await runPhase0(rows);
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isLoading, attempt]);

  const runPhase0 = useCallback(async (initial: Phase0Row[]) => {
    let rows = initial;
    const apply = (next: Phase0Row[]) => {
      rows = next;
      if (!aborted.current) setPhase0(next);
    };
    const pending = (key: Phase0Row["key"]) => {
      const row = rows.find((r) => r.key === key);
      return row != null && row.status !== "skipped" && row.status !== "done";
    };
    const fail = (key: Phase0Row["key"], cause: unknown) => {
      apply(phase0Update(rows, key, { status: "failed", message: message(cause, "That did not work.") }));
    };
    try {
      if (pending("install")) {
        apply(phase0Update(rows, "install", { status: "running", message: "Looking for the latest release" }));
        try {
          const result = await installForgeCli((p: InstallProgress) => apply(phase0FromProgress(rows, p, "install")));
          apply(phase0Update(rows, "install", { status: "done", message: result.version ? `Forge ${result.version}` : "Installed", fraction: 1 }));
        } catch (cause) {
          return fail("install", cause);
        }
      }
      let serve = await detectForgeServe();
      if (pending("service") && !serve) {
        apply(phase0Update(rows, "service", { status: "running", message: "Installing the background service" }));
        try {
          const result = await installForgeService((p: InstallProgress) => apply(phase0FromProgress(rows, p, "service")));
          serve = result.serve ?? (await pollForForgeServe(10_000));
          if (!serve) throw new Error("Forge started but did not answer yet. Try again.");
          apply(phase0Update(rows, "service", { status: "done", message: "Running", fraction: 1 }));
        } catch (cause) {
          return fail("service", cause);
        }
      } else if (pending("service")) {
        apply(phase0Update(rows, "service", { status: "done", message: "Running", fraction: 1 }));
      }
      if (pending("connect")) {
        apply(phase0Update(rows, "connect", { status: "running", message: "Connecting" }));
        try {
          serve = serve ?? (await pollForForgeServe(10_000));
          if (!serve) throw new Error("Forge is not answering on this computer.");
          if (serve.exposure === "lan") throw new Error("Forge is only reachable over HTTPS on your network. Start it with --local.");
          await addServer(serve.base_url, { setActive: true });
          apply(phase0Update(rows, "connect", { status: "done", message: "Connected", fraction: 1 }));
        } catch (cause) {
          return fail("connect", cause);
        }
      }
      if (!aborted.current) setPhase("daemon");
    } catch (cause) {
      if (!aborted.current) {
        setError(message(cause, "Setup could not start."));
        setPhase("error");
      }
    }
  }, [addServer]);

  // After phase 0 the new server becomes the active one; load its plan.
  useEffect(() => {
    if (phase !== "daemon" || snapshot || !baseUrl) return;
    void load(baseUrl, false).catch((cause) => {
      if (aborted.current) return;
      if (cause instanceof ApiError && cause.status === 403) setPhase("unavailable");
      else {
        setError(message(cause, "Forge did not answer."));
        setPhase("error");
      }
    });
  }, [phase, snapshot, baseUrl, load]);

  const retryPhase0 = useCallback(() => {
    setError(null);
    setAttempt((n) => n + 1);
  }, []);

  const refresh = useCallback(async () => {
    const base = baseRef.current;
    if (!base) return null;
    try {
      return await load(base, true);
    } catch {
      return null;
    }
  }, [load]);

  const markStep = useCallback(
    async (step: StepId, op: "done" | "skip") => {
      const base = baseRef.current;
      if (!base) return;
      setBusy(true);
      try {
        const next = await setStep(base, step, op);
        if (!aborted.current) setSnapshot((prev) => (prev ? { ...prev, ...next, open: prev.open } : next));
      } finally {
        if (!aborted.current) setBusy(false);
      }
    },
    [],
  );

  const run = useCallback(
    async (action: Action, opts: RunOptions): Promise<JobView> => {
      const base = baseRef.current;
      if (!base) throw new Error("Forge is not connected.");
      const started = await startAction(base, action, opts.step);
      let view = newJobView(started.job, opts.label ?? started.label);
      const publish = (next: JobView) => {
        view = next;
        if (aborted.current) return;
        setViews((prev) => ({ ...prev, [started.job]: next }));
        opts.onUpdate?.(next);
      };
      setSlots((prev) => ({ ...prev, [opts.slot]: started.job }));
      publish(view);
      let failures = 0;
      while (!aborted.current && !jobFinished(view)) {
        try {
          const poll = await pollJob(base, started.job, view.cursor, LONG_POLL_MS);
          failures = 0;
          publish(applyJobPoll(view, poll));
        } catch (cause) {
          failures += 1;
          if (failures >= 3) {
            publish({ ...view, state: "failed", prompt: null, error: message(cause, "Lost contact with Forge.") });
          } else {
            await new Promise((resolve) => setTimeout(resolve, 600 * failures));
          }
        }
      }
      if (opts.step || view.result) await refresh();
      return view;
    },
    [refresh],
  );

  const slotJob = useCallback(
    (slot: string) => {
      const id = slots[slot];
      return id == null ? null : (views[id] ?? null);
    },
    [slots, views],
  );

  const clearSlot = useCallback((slot: string) => {
    setSlots((prev) => {
      const { [slot]: _gone, ...rest } = prev;
      return rest;
    });
  }, []);

  const sendInput = useCallback(async (job: number, text: string) => {
    const base = baseRef.current;
    if (base) await sendJobInput(base, job, text);
  }, []);

  const cancel = useCallback(async (job: number) => {
    const base = baseRef.current;
    if (base) await cancelJob(base, job).catch(() => undefined);
  }, []);

  const runRecommended = useCallback(async () => {
    const current = snapshot;
    if (!current || checklistRunning) return;
    let list = buildChecklist(current.recommended);
    setChecklist(list);
    setChecklistRunning(true);
    try {
      for (let guard = 0; guard < 32 && !aborted.current; guard += 1) {
        const i = nextToRun(list);
        if (i < 0) break;
        const item = list[i];
        await run(item.action as Action, {
          step: item.step,
          slot: "checklist",
          label: item.title,
          onUpdate: (view) => {
            list = applyChecklistJob(list, i, view);
            if (!aborted.current) setChecklist(list);
          },
        });
      }
      const blocked = list.some((item) => item.status === "failed" && item.needsYou);
      if (!blocked && !aborted.current) {
        for (const step of DEFAULT_ONLY_STEPS) {
          await markStep(step, "done");
        }
      }
    } finally {
      if (!aborted.current) setChecklistRunning(false);
    }
    const after = await refresh();
    if (after && !aborted.current) {
      const needsModel = after.missing.includes("models");
      const hasChoice = list.some((item) => item.status === "pending" && !item.action);
      if (needsModel || hasChoice) setIndex(stepIndexOf(after.plan, "models"));
    }
  }, [snapshot, checklistRunning, run, markStep, refresh]);

  // "Reconnect Claude Code" from the banner lands here and starts the sign-in straight away.
  useEffect(() => {
    if (options.focus !== "claude" || autoConnect.current || !snapshot || phase !== "daemon") return;
    autoConnect.current = true;
    void run({ action: "connect_claude" }, { slot: "source:claude-cli", step: "models" });
  }, [options.focus, snapshot, phase, run]);

  const leave = useCallback((destination?: string) => {
    dismissOnboardingForThisLaunch();
    router.replace((destination ?? takePendingRoute() ?? "/(tabs)") as never);
  }, []);

  const finish = useCallback(
    async (destination?: string) => {
      const base = baseRef.current;
      if (base) await setFlow(base, "complete").catch(() => undefined);
      leave(destination);
    },
    [leave],
  );

  const later = useCallback(async () => {
    const base = baseRef.current;
    if (base) await setFlow(base, "postpone").catch(() => undefined);
    leave();
  }, [leave]);

  const last = (snapshot?.plan.length ?? 1) - 1;
  return {
    phase,
    error,
    phase0,
    retryPhase0,
    snapshot,
    index,
    goTo: (i) => setIndex(Math.max(0, Math.min(i, last))),
    next: () => setIndex((i) => Math.min(i + 1, last)),
    back: () => setIndex((i) => Math.max(i - 1, 0)),
    refresh,
    markDone: (step) => markStep(step, "done"),
    skip: (step) => markStep(step, "skip"),
    busy,
    run,
    slotJob,
    clearSlot,
    sendInput,
    cancel,
    checklist,
    checklistRunning,
    runRecommended,
    finish,
    later,
  };
}
