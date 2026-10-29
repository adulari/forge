import { describe, expect, it } from "vitest";

import type { ConfigField } from "./api";
import { configSection, groupConfigFields, searchSettings } from "./settingsSearch";

function field(key: string, label: string, group = "Interface", help: string | null = null): ConfigField {
  return { key, group, label, help, field_type: "text", options: [], value: "", default: "", modified: false, source: "default" };
}

const fields = [
  field("power.keep_awake", "Keep this computer awake", "Host"),
  field("git.coauthor", "Co-author commits", "Git"),
  field("host.max_terminals", "Max terminals", "Host"),
  field("tui.theme", "Colour theme"),
];

describe("settings search", () => {
  it("finds the keep-awake row from the word sleep", () => {
    const hits = searchSettings("sleep", fields);
    expect(hits[0]).toMatchObject({ kind: "field", focusKey: "power.keep_awake", href: "/configuration" });
  });

  it("returns pages too, and nothing for an empty query", () => {
    expect(searchSettings("keyboard", fields).some((hit) => hit.href === "/keybindings")).toBe(true);
    expect(searchSettings("  ", fields)).toEqual([]);
  });

  it("groups fields into the named sections", () => {
    expect(configSection(fields[1])).toBe("Git");
    expect(configSection(fields[2])).toBe("Terminal");
    expect(configSection(fields[0])).toBe("Host");
    const groups = groupConfigFields(fields).map(([name]) => name);
    expect(groups.indexOf("Terminal")).toBeLessThan(groups.indexOf("Git"));
    expect(groups).toContain("Host");
  });
});
