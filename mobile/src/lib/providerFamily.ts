// Which provider family a routed model id (`namespace::model`) or a bare provider id belongs to.
// The marks and colours in ProviderGlyph key off this, and the TUI keeps the same table in
// forge-tui's theme/glyphs.rs, so a provider reads the same on every surface.

export type ProviderFamily =
  | "claude"
  | "openai"
  | "gemini"
  | "opencode"
  | "grok"
  | "kimi"
  | "qwen"
  | "ollama"
  | "openrouter"
  | "bedrock"
  | "other";

const FAMILY_NEEDLES: readonly (readonly [ProviderFamily, readonly string[]])[] = [
  ["openrouter", ["openrouter"]],
  ["opencode", ["opencode"]],
  ["bedrock", ["bedrock"]],
  ["claude", ["claude", "anthropic"]],
  ["openai", ["codex", "openai", "gpt"]],
  ["gemini", ["gemini", "google"]],
  ["grok", ["grok", "xai"]],
  ["kimi", ["kimi", "moonshot"]],
  ["qwen", ["qwen", "dashscope"]],
  ["ollama", ["ollama"]],
];

function familyOfName(name: string): ProviderFamily {
  const lower = name.toLowerCase();
  for (const [family, needles] of FAMILY_NEEDLES) {
    if (needles.some((needle) => lower.includes(needle))) return family;
  }
  return "other";
}

/** The namespace decides; a custom namespace (`inferhub::cb/gemini-flash`) falls back to what the
 * model name says. */
export function providerFamily(id: string | null | undefined): ProviderFamily {
  if (!id) return "other";
  const split = id.indexOf("::");
  if (split < 0) return familyOfName(id);
  const family = familyOfName(id.slice(0, split));
  return family === "other" ? familyOfName(id.slice(split + 2)) : family;
}

const FAMILY_LABEL: Record<ProviderFamily, string> = {
  claude: "Claude",
  openai: "OpenAI",
  gemini: "Gemini",
  opencode: "OpenCode",
  grok: "Grok",
  kimi: "Kimi",
  qwen: "Qwen",
  ollama: "Ollama",
  openrouter: "OpenRouter",
  bedrock: "Bedrock",
  other: "Model",
};

export const providerFamilyLabel = (family: ProviderFamily) => FAMILY_LABEL[family];

/** The id to draw a session's provider mark from: the model id when it names a known family,
 * otherwise the daemon-reported provider namespace. */
export function providerIdOf(row: { model: string; provider?: string | null }): string {
  return providerFamily(row.model) !== "other" ? row.model : row.provider ?? row.model;
}
