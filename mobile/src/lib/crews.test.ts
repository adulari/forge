import { describe, expect, it } from "vitest";

import { crewHasErrors, emptyCrewDraft, emptyMember, serializeCrew, validateCrew, type CrewDraft } from "./crews";

const ok = (): CrewDraft => ({
  name: "review-team",
  description: " Reviews PRs ",
  members: [
    { role: "reviewer", tier: "complex", effort: "high", no_edits: true, brief: " Check logic " },
    { role: "tester", tier: null, effort: null, no_edits: false, brief: "" },
  ],
});

describe("validateCrew", () => {
  it("accepts a good crew", () => {
    expect(crewHasErrors(validateCrew(ok()))).toBe(false);
  });

  it("rejects bad names and roles", () => {
    const d = ok();
    d.name = "Bad Name";
    d.members[0].role = "";
    const e = validateCrew(d);
    expect(e.name).toBeTruthy();
    expect(e.member[0]?.role).toBeTruthy();
  });

  it("rejects duplicate roles", () => {
    const d = ok();
    d.members[1].role = "reviewer";
    expect(validateCrew(d).member[1]?.role).toBe("Roles must be unique.");
  });

  it("enforces 1-8 members", () => {
    const none = ok();
    none.members = [];
    expect(validateCrew(none).members).toBeTruthy();
    const many = ok();
    many.members = Array.from({ length: 9 }, (_, i) => ({ ...emptyMember(), role: `r${i}` }));
    expect(validateCrew(many).members).toBeTruthy();
  });

  it("caps the brief at 4000 characters", () => {
    const d = ok();
    d.members[0].brief = "x".repeat(4001);
    expect(validateCrew(d).member[0]?.brief).toBeTruthy();
  });

  it("flags a blank new draft", () => {
    expect(crewHasErrors(validateCrew(emptyCrewDraft()))).toBe(true);
  });
});

describe("serializeCrew", () => {
  it("trims text and omits unset tier/effort", () => {
    expect(serializeCrew(ok())).toEqual({
      name: "review-team",
      description: "Reviews PRs",
      members: [
        { role: "reviewer", tier: "complex", effort: "high", no_edits: true, brief: "Check logic" },
        { role: "tester", no_edits: false, brief: "" },
      ],
    });
  });

  it("drops unknown tier or effort values", () => {
    const d = ok();
    d.members[0].tier = "huge";
    d.members[0].effort = "max";
    const m = serializeCrew(d).members[0];
    expect("tier" in m).toBe(false);
    expect("effort" in m).toBe(false);
  });
});
