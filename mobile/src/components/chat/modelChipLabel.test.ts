import { describe, expect, it } from "vitest";

import { modelChipLabel } from "./modelChipLabel";

describe("modelChipLabel", () => {
  it("marks the mesh's pick as automatic when the host says nothing is pinned", () => {
    expect(modelChipLabel("openrouter::dots-studio/dots-3-note-preview:free", false)).toBe("auto · dots-3-note-preview:free");
  });

  it("shows a pin as the full model id", () => {
    expect(modelChipLabel("claude-cli::haiku", true)).toBe("claude-cli::haiku");
  });

  it("keeps the old behaviour for hosts that send no flag", () => {
    expect(modelChipLabel("kimi::k3-256k", undefined)).toBe("kimi::k3-256k");
    expect(modelChipLabel("—", undefined)).toBe("auto");
  });
});
