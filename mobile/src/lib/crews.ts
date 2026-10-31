// Saved crews: named rosters of subagent roles. Pure validation/serialisation shared by the
// Crews page; the daemon re-validates (PUT /api/crews answers 400 {error} on a bad body).

export const CREW_TIERS = ["trivial", "standard", "complex"] as const;
export type CrewTier = (typeof CREW_TIERS)[number];

// Mirrors EFFORT_LEVELS in components/session/EffortPicker (kept RN-free so this stays unit-testable).
export const CREW_EFFORTS = ["low", "medium", "high", "xhigh", "whitehot"] as const;

export const CREW_NAME_RE = /^[a-z0-9_-]{1,32}$/;
export const CREW_MAX_MEMBERS = 8;
export const CREW_BRIEF_MAX = 4000;

export interface CrewMember {
  role: string;
  tier?: string | null;
  effort?: string | null;
  no_edits: boolean;
  brief: string;
}

export interface Crew {
  name: string;
  description: string;
  members: CrewMember[];
  /** "user" crews are editable; "project" crews live in the repo and are read-only. */
  scope: "user" | "project";
  editable: boolean;
}

export interface CrewDraft {
  name: string;
  description: string;
  members: CrewMember[];
}

export interface CrewErrors {
  name?: string;
  description?: string;
  members?: string;
  /** Per member index. */
  member: Record<number, { role?: string; brief?: string }>;
}

export const emptyMember = (): CrewMember => ({ role: "", tier: null, effort: null, no_edits: false, brief: "" });
export const emptyCrewDraft = (): CrewDraft => ({ name: "", description: "", members: [emptyMember()] });

// The daemon accepts hand-written spellings ("Complex", effort "max" = xhigh); normalise them so
// the controls show the value and a save keeps it.
const normTier = (t?: string | null): string | null => (t && t.trim() ? t.trim().toLowerCase() : null);
const normEffort = (e?: string | null): string | null => {
  const v = e && e.trim() ? e.trim().toLowerCase() : null;
  return v === "max" ? "xhigh" : v;
};

export const draftFromCrew = (c: Crew): CrewDraft => ({
  name: c.name,
  description: c.description,
  members: c.members.map((m) => ({ ...m, tier: normTier(m.tier), effort: normEffort(m.effort) })),
});

export const isProjectCrew = (c: Crew): boolean => c.scope === "project";

export function duplicateNameError(name: string, existing: readonly Crew[] | undefined): string | undefined {
  const n = name.trim();
  return n && existing?.some((c) => c.name === n) ? `A crew named ${n} already exists.` : undefined;
}

export function isNameConflict(err: unknown): boolean {
  const status = (err as { status?: number } | null)?.status;
  return status === 409 || (err instanceof Error && /already exists/i.test(err.message));
}

export function validateCrew(d: CrewDraft): CrewErrors {
  const errors: CrewErrors = { member: {} };
  if (!CREW_NAME_RE.test(d.name)) errors.name = "Use 1-32 characters: a-z, 0-9, - or _.";
  if (d.members.length < 1) errors.members = "Add at least one member.";
  else if (d.members.length > CREW_MAX_MEMBERS) errors.members = `At most ${CREW_MAX_MEMBERS} members.`;
  const seen = new Set<string>();
  d.members.forEach((m, i) => {
    const e: { role?: string; brief?: string } = {};
    if (!CREW_NAME_RE.test(m.role)) e.role = "Use 1-32 characters: a-z, 0-9, - or _.";
    else if (seen.has(m.role)) e.role = "Roles must be unique.";
    seen.add(m.role);
    if (m.brief.length > CREW_BRIEF_MAX) e.brief = `Brief is over ${CREW_BRIEF_MAX} characters.`;
    if (e.role || e.brief) errors.member[i] = e;
  });
  return errors;
}

export function crewHasErrors(e: CrewErrors): boolean {
  return Boolean(e.name || e.description || e.members) || Object.keys(e.member).length > 0;
}

/** Request body for PUT /api/crews: optional tier/effort omitted when unset, text trimmed. */
export function serializeCrew(d: CrewDraft, opts: { createOnly?: boolean } = {}) {
  return {
    ...(opts.createOnly ? { create_only: true } : {}),
    name: d.name.trim(),
    description: d.description.trim(),
    members: d.members.map((m) => ({
      role: m.role.trim(),
      ...(m.tier?.trim() ? { tier: m.tier.trim() } : {}),
      ...(m.effort?.trim() ? { effort: m.effort.trim() } : {}),
      no_edits: m.no_edits,
      brief: m.brief.trim(),
    })),
  };
}

export type CrewBody = ReturnType<typeof serializeCrew>;
