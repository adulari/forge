import { describe, expect, it } from "vitest";

import { crewHasErrors, draftFromCrew, duplicateNameError, isNameConflict, isProjectCrew, type Crew, emptyCrewDraft, emptyMember, serializeCrew, validateCrew, type CrewDraft } from "./crews";

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

  it("passes unrecognised tier/effort through for the server to judge", () => {
    const d = ok();
    d.members[0].tier = "huge";
    d.members[0].effort = "max";
    const m = serializeCrew(d).members[0];
    expect(m.tier).toBe("huge");
    expect(m.effort).toBe("max");
  });

  it("adds create_only only for a new crew", () => {
    expect(serializeCrew(ok(), { createOnly: true }).create_only).toBe(true);
    expect("create_only" in serializeCrew(ok())).toBe(false);
  });
});

const crew = (over: Partial<Crew> = {}): Crew => ({ name: "a", description: "", scope: "user", editable: true, members: [], ...over });

describe("draftFromCrew", () => {
  it("normalises hand-written tier and effort so a save keeps them", () => {
    const d = draftFromCrew(
      crew({ members: [{ role: "r", tier: "Complex", effort: "max", no_edits: false, brief: "" }] }),
    );
    expect(d.members[0].tier).toBe("complex");
    expect(d.members[0].effort).toBe("xhigh");
    const m = serializeCrew(d).members[0];
    expect(m.tier).toBe("complex");
    expect(m.effort).toBe("xhigh");
  });
});

describe("duplicateNameError", () => {
  it("flags a name that already exists", () => {
    expect(duplicateNameError("a", [crew()])).toBe("A crew named a already exists.");
    expect(duplicateNameError("b", [crew()])).toBeUndefined();
    expect(duplicateNameError("a", undefined)).toBeUndefined();
  });
});

describe("isNameConflict / isProjectCrew", () => {
  it("recognises the server 409", () => {
    expect(isNameConflict(Object.assign(new Error("x"), { status: 409 }))).toBe(true);
    expect(isNameConflict(new Error("a crew with that name already exists"))).toBe(true);
    expect(isNameConflict(new Error("boom"))).toBe(false);
  });

  it("only project-scope crews are project crews", () => {
    expect(isProjectCrew(crew({ scope: "project", editable: false }))).toBe(true);
    expect(isProjectCrew(crew())).toBe(false);
  });
});
