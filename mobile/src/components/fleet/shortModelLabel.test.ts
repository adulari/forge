import { describe, expect, it } from "vitest";

import { shortModelLabel } from "./shortModelLabel";

describe("shortModelLabel", () => {
  it("keeps just the model name so the fleet row still shows host and project", () => {
    expect(shortModelLabel("openrouter::dots-studio/dots-3-note-preview:free")).toBe("dots-3-note-preview:free");
    expect(shortModelLabel("kimi::k3-256k")).toBe("k3-256k");
    expect(shortModelLabel("gpt-5")).toBe("gpt-5");
  });

  it("names an Automatic session instead of showing the daemon's dash", () => {
    expect(shortModelLabel("—")).toBe("auto");
    expect(shortModelLabel("")).toBe("auto");
  });
});
