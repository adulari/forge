import { describe, expect, it } from "vitest";

import { composerPlaceholder, modelChipLabel } from "./modelChipLabel";

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

  it("drops the provider prefix of a pin on a phone", () => {
    expect(modelChipLabel("inferhub::cb/deepseek-v4.1-flash", true, true)).toBe("deepseek-v4.1-flash");
  });
});

describe("composerPlaceholder", () => {
  it("names the model that will answer", () => {
    expect(composerPlaceholder("inferhub::cb/deepseek-v4.1-flash")).toBe("Ask deepseek-v4.1-flash anything about this workspace");
    expect(composerPlaceholder("inferhub::cb/deepseek-v4.1-flash", true)).toBe("Ask deepseek-v4.1-flash anything");
  });

  it("falls back to the plain prompt before a model is known", () => {
    expect(composerPlaceholder(null)).toBe("message…");
    expect(composerPlaceholder("—")).toBe("message…");
  });
});
