// Pure parsing/derivation for the native overlays (mesh explain + workflow rows). Kept free of
// react-native imports so it is unit-testable in the node vitest env (same discipline as the
// pure helpers under lib/), and imported by NativeOverlayContent.tsx for rendering.
import type { OverlayRow } from "../../lib/ws";
import type { BadgeTone } from "../ds/Badge";

/** The leading glyph of an `overlay:workflow` row label maps to a phase/agent state. */
export function workflowState(label: string): "done" | "failed" | "running" | "pending" {
  const glyph = label.trimStart()[0];
  if (glyph === "✓") return "done";
  if (glyph === "✗") return "failed";
  if (glyph === "◐") return "running";
  return "pending";
}

// The badge vocabulary the daemon's `overlay:mesh` rows actually emit (see
// `mesh_overlay_snapshot` in forge-tui): a cost tag, `frontier`, and `unusable`, plus the task
// classifications / cost tiers a `/mesh` line can also carry. `unusable` is the wire's word for
// a benched / rate-limited / capability-excluded candidate.
const BADGE_KEYWORDS = new Set([
  "complex",
  "standard",
  "trivial",
  "subscription",
  "sub",
  "free",
  "api",
  "paid",
  "frontier",
  "unusable",
  "benched",
]);

export function badgeTone(badge: string): BadgeTone {
  const b = badge.toLowerCase();
  if (b === "benched" || b === "unusable") return "danger";
  if (b === "complex" || b === "standard" || b === "trivial") return "warn";
  if (b === "subscription" || b === "sub" || b === "frontier") return "accent";
  if (b === "free") return "success";
  return "neutral";
}

/** Strip a `#N ` rank prefix from a mesh candidate label, keeping the model id. */
export function modelIdFrom(label: string): string {
  const rank = label.match(/^#\d+\s+(.+)$/);
  return (rank?.[1] ?? label).trim();
}

export interface Score {
  label: string;
  value: number;
}

export interface ParsedCandidate {
  id: string;
  scores: Score[];
  badges: string[];
  reason: string;
  benched: boolean;
}

/** Split a `/mesh` candidate row (`#1 model` + `intelligence 71 · coding 65 · free · reason`)
 *  into its model id, numeric score bars, semantic badges, and free-form reject reason. */
export function parseCandidate(row: OverlayRow): ParsedCandidate {
  const id = modelIdFrom(row.label);
  const parts = row.detail.split(" · ").map((p) => p.trim()).filter(Boolean);
  const scores: Score[] = [];
  const badges: string[] = [];
  const reasonParts: string[] = [];
  for (const part of parts) {
    const score = part.match(/^([a-z][a-z ]{0,10}?)\s+(\d+(?:\.\d+)?)$/i);
    if (score) {
      scores.push({ label: `${score[1].trim()} ${score[2]}`, value: Number(score[2]) });
      continue;
    }
    if (/^[a-z0-9.:_+-]+$/i.test(part) && BADGE_KEYWORDS.has(part.toLowerCase())) {
      badges.push(part);
      continue;
    }
    reasonParts.push(part);
  }
  const reason = reasonParts.join(" · ");
  const benched = /bench|rate limit|unusable/i.test(`${reason} ${badges.join(" ")}`);
  return { id, scores, badges, reason, benched };
}
