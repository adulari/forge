import { describe, expect, it } from "vitest";

import { mergeCommandSources } from "./commandSources";

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
