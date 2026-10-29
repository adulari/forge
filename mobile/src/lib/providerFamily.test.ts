import { describe, expect, it } from "vitest";

import { providerFamily, providerIdOf } from "./providerFamily";

describe("providerFamily", () => {
  it("lets the namespace decide", () => {
    expect(providerFamily("claude-cli::sonnet")).toBe("claude");
    expect(providerFamily("codex-oauth::gpt-5.5")).toBe("openai");
    expect(providerFamily("bedrock::global.moonshotai.kimi-k3")).toBe("bedrock");
    expect(providerFamily("openrouter::qwen/qwen3")).toBe("openrouter");
    expect(providerFamily("opencode::muse")).toBe("opencode");
  });

  it("falls back to the model name for a custom namespace", () => {
    expect(providerFamily("inferhub::cb/gemini-flash")).toBe("gemini");
    expect(providerFamily("inferhub::cb/deepseek-v4.1-flash")).toBe("other");
  });

  it("reads a bare provider id and tolerates nothing", () => {
    expect(providerFamily("codex-cli")).toBe("openai");
    expect(providerFamily("ollama")).toBe("ollama");
    expect(providerFamily("mock")).toBe("other");
    expect(providerFamily(null)).toBe("other");
    expect(providerFamily("—")).toBe("other");
  });

  it("draws a session from its model, else from the reported provider", () => {
    expect(providerIdOf({ model: "claude-cli::sonnet", provider: "x" })).toBe("claude-cli::sonnet");
    expect(providerIdOf({ model: "mock", provider: "ollama" })).toBe("ollama");
    expect(providerIdOf({ model: "mock" })).toBe("mock");
  });
});
