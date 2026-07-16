export interface SlashCommandSource {
  name: string;
  description: string;
}

/** Merge static and daemon-provided commands for rendering. */
export function mergeCommandSources(
  builtins: readonly string[],
  skills: readonly SlashCommandSource[],
): SlashCommandSource[] {
  const commands = new Map<string, SlashCommandSource>();
  for (const name of builtins) commands.set(name, { name, description: "" });
  // A daemon skill with the same invocation is the richer representation of the same
  // command. Replacing the value preserves the built-in's stable list position and gives React
  // exactly one child identity while retaining the skill description.
  for (const skill of skills) commands.set(skill.name, skill);
  return [...commands.values()];
}
