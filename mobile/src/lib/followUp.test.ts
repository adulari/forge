import { describe, expect, it } from "vitest";

import type { ConfigField } from "./api";
import { effectiveMode, followUpFromConfig, invertMode } from "./followUp";

const field = (value: string): ConfigField => ({
  key: "chat.follow_up",
  group: "Interface",
  field_type: "enum",
  label: "Enter while busy",
  help: null,
  options: ["steer", "queue"],
  value,
  default: "steer",
  modified: value !== "steer",
  source: "default",
});

describe("follow-up setting", () => {
  it("reads the daemon's config and marks an older daemon unsupported", () => {
    expect(followUpFromConfig([field("queue")])).toEqual({ mode: "queue", supported: true });
    expect(followUpFromConfig([field("steer")])).toEqual({ mode: "steer", supported: true });
    expect(followUpFromConfig([])).toEqual({ mode: "steer", supported: false });
    expect(followUpFromConfig(undefined)).toEqual({ mode: "steer", supported: false });
  });

  it("Cmd/Ctrl+Enter does the opposite of the setting", () => {
    const queue = followUpFromConfig([field("queue")]);
    expect(effectiveMode(queue, false)).toBe("queue");
    expect(effectiveMode(queue, true)).toBe("steer");
    const steer = followUpFromConfig([field("steer")]);
    expect(effectiveMode(steer, true)).toBe("queue");
    expect(invertMode("queue")).toBe("steer");
  });

  it("never asks an older daemon to queue", () => {
    const old = followUpFromConfig([]);
    expect(effectiveMode(old, true)).toBe("steer");
  });
});
