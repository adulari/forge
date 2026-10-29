import { describe, expect, it } from "vitest";

import { mergeCommandSources } from "./commandSources";
import { BUILTIN_COMMANDS, skillInvoked } from "./builtinCommands";

describe("mergeCommandSources", () => {
  it("gives every rendered slash command one stable identity", () => {
    const commands = mergeCommandSources(
      ["/help", "/orchestrate"],
      [
        { name: "/orchestrate", description: "Coordinate a complex task" },
        { name: "/project-skill", description: "Project-specific workflow" },
      ],
    );

    expect(commands.filter((command) => command.name === "/orchestrate")).toEqual([
      { name: "/orchestrate", description: "Coordinate a complex task" },
    ]);
    expect(new Set(commands.map((command) => command.name)).size).toBe(commands.length);
  });
});

describe("skillInvoked", () => {
  const skills = ["/agent-creator", "/rust", "/help"];

  it("names the skill a message starts with", () => {
    expect(skillInvoked("/agent-creator make a reviewer", skills)).toBe("/agent-creator");
    expect(skillInvoked("/Rust fix the borrow", skills)).toBe("/rust");
  });

  it("recognises the expansion a built-in skill-command is stored as", () => {
    const stored = "Apply the **rust-best-practices** skill to this task. Follow its methodology.\n\nTask: fix it";
    expect(skillInvoked(stored, skills)).toBe("/rust-best-practices");
  });

  it("ignores prose, unknown names and built-in commands that a skill shadows", () => {
    expect(skillInvoked("please /agent-creator", skills)).toBeNull();
    expect(skillInvoked("/unknown thing", skills)).toBeNull();
    expect(skillInvoked("/help", skills)).toBeNull();
  });

  it("knows the side-question commands", () => {
    expect(BUILTIN_COMMANDS).toContain("/btw");
    expect(BUILTIN_COMMANDS).toContain("/side");
  });
});
