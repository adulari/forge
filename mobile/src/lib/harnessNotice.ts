// Wakeups, /heartbeat ticks and peer messages are stored as user-role rows (the model needs the
// user turn) but are not the person's words. The host tags them `kind: "harness_notice"`
// (remote_projection.rs `map_history_row`); the shapes below mirror
// crates/forge-types/src/harness_notice.rs. Background-job exits keep their own `job_notice` kind
// (lib/jobNotice.ts).
import { formatJobNotice, isJobNotice } from "./jobNotice";

const WAKEUP = /^\[scheduled wakeup\] ([\s\S]*)$/;
const WAKEUP_REASON = /^([\s\S]*)\n\n\(scheduled because: ([\s\S]+)\)$/;
const HEARTBEAT = /^\[heartbeat(?:: ([A-Za-z0-9_-]{1,40}))?\] ([\s\S]*)$/;
const PEER = /^\[message from ([^\n]*?)\] ([\s\S]*)$/;
const CLIP = 120;

export function isHarnessNotice(kind?: string): boolean {
  return kind === "harness_notice";
}

/** Any row kind the host tags as a harness-written user turn (job exits included). */
export function isAnyHarnessNotice(kind?: string): boolean {
  return isHarnessNotice(kind) || isJobNotice(kind);
}

function firstLine(text: string): string {
  const line = (text.split("\n").find((l) => l.trim()) ?? "").trim();
  return line.length > CLIP ? `${line.slice(0, CLIP - 1).trimEnd()}…` : line;
}

export function formatHarnessNotice(content: string): string {
  const wake = WAKEUP.exec(content);
  if (wake) {
    const withReason = WAKEUP_REASON.exec(wake[1]);
    const prompt = withReason ? withReason[1] : wake[1];
    const out = [`⏰ wakeup · ${firstLine(prompt)}`];
    if (withReason) out.push(`    because: ${firstLine(withReason[2])}`);
    return out.join("\n");
  }
  const beat = HEARTBEAT.exec(content);
  if (beat) return `♻ heartbeat${beat[1] ? ` ${beat[1]}` : ""} · ${firstLine(beat[2])}`;
  const peer = PEER.exec(content);
  if (peer) {
    const body = peer[2].includes("\n---\n") ? peer[2].slice(peer[2].indexOf("\n---\n") + 5) : peer[2];
    return [`✉ peer message from ${peer[1]}`, ...body.split("\n").map((l) => `    ${l}`)].join("\n");
  }
  return content;
}

/** Text for a row of either harness kind. */
export function formatAnyHarnessNotice(kind: string | undefined, content: string): string {
  return isJobNotice(kind) ? formatJobNotice(content) : formatHarnessNotice(content);
}
