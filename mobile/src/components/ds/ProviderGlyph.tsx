// Monochrome provider marks. Each is a small geometric abstraction drawn on a 16-unit grid in the
// icon stroke weight the rest of the shell uses; they say which provider answers at a glance and
// take their colour from the caller (ink2 by default), never a brand colour. The same family
// table drives the TUI's one-cell mark (forge-tui theme/glyphs.rs).
import React from "react";
import Svg, { Circle, Ellipse, G, Line, Path, Rect } from "react-native-svg";

import { providerFamily, providerFamilyLabel, type ProviderFamily } from "../../lib/providerFamily";
import { useTokens } from "../../theme/ThemeProvider";

export interface ProviderGlyphProps {
  /** A routed model id (`claude-cli::sonnet`), a provider id (`codex-oauth`) or a family. */
  provider: string | null | undefined;
  size?: number;
  color?: string;
  /** Spoken label; the family name by default. Pass `null` when text beside it already says it. */
  accessibilityLabel?: string | null;
}

const STROKE = 1.4;

function Mark({ family, color }: { family: ProviderFamily; color: string }) {
  const line = { stroke: color, strokeWidth: STROKE, strokeLinecap: "round" as const, strokeLinejoin: "round" as const, fill: "none" };
  switch (family) {
    case "claude":
      return (
        <G {...line}>
          {[0, 45, 90, 135].map((angle) => (
            <Line key={angle} x1="8" y1="1.8" x2="8" y2="14.2" transform={`rotate(${angle} 8 8)`} />
          ))}
        </G>
      );
    case "openai":
      return (
        <G {...line}>
          <Path d="M8 1.6 L13.6 4.8 V11.2 L8 14.4 L2.4 11.2 V4.8 Z" />
          <Path d="M8 1.6 V8 L13.6 11.2 M8 8 L2.4 11.2" />
        </G>
      );
    case "gemini":
      return <Path d="M8 1 C8.6 5.4 10.6 7.4 15 8 C10.6 8.6 8.6 10.6 8 15 C7.4 10.6 5.4 8.6 1 8 C5.4 7.4 7.4 5.4 8 1 Z" fill={color} />;
    case "opencode":
      return (
        <G {...line}>
          <Path d="M5.2 2.8 H2.8 V13.2 H5.2" />
          <Path d="M10.8 2.8 H13.2 V13.2 H10.8" />
          <Rect x="7" y="7" width="2" height="2" fill={color} stroke="none" />
        </G>
      );
    case "grok":
      return (
        <G {...line}>
          <Ellipse cx="8" cy="8" rx="6.6" ry="2.8" transform="rotate(-35 8 8)" />
          <Circle cx="8" cy="8" r="1.7" fill={color} stroke="none" />
        </G>
      );
    case "kimi":
      return (
        <G {...line}>
          <Path d="M4.6 2.8 V13.2" />
          <Path d="M11.6 2.8 L4.6 8.2 L11.6 13.2" />
        </G>
      );
    case "qwen":
      return (
        <G {...line}>
          <Circle cx="7.4" cy="7.4" r="4.8" />
          <Path d="M10.6 10.6 L13.8 13.8" />
        </G>
      );
    case "ollama":
      return (
        <G {...line}>
          <Path d="M5.4 6 V2.4 M10.6 6 V2.4" />
          <Rect x="3.4" y="5.8" width="9.2" height="8.2" rx="4" />
          <Circle cx="6.2" cy="9.4" r="0.7" fill={color} stroke="none" />
          <Circle cx="9.8" cy="9.4" r="0.7" fill={color} stroke="none" />
        </G>
      );
    case "openrouter":
      return (
        <G {...line}>
          <Path d="M2 5 H11.4 M8.8 2.4 L11.4 5 L8.8 7.6" />
          <Path d="M14 11 H4.6 M7.2 8.4 L4.6 11 L7.2 13.6" />
        </G>
      );
    case "bedrock":
      return (
        <G {...line}>
          <Rect x="1.8" y="2.8" width="12.4" height="2.8" rx="0.6" />
          <Rect x="3.8" y="6.6" width="8.4" height="2.8" rx="0.6" />
          <Rect x="5.8" y="10.4" width="4.4" height="2.8" rx="0.6" />
        </G>
      );
    default:
      return (
        <G {...line}>
          <Path d="M8 2 L14 8 L8 14 L2 8 Z" />
          <Circle cx="8" cy="8" r="1.2" fill={color} stroke="none" />
        </G>
      );
  }
}

export function ProviderGlyph({ provider, size = 14, color, accessibilityLabel }: ProviderGlyphProps) {
  const tokens = useTokens();
  const family = providerFamily(provider);
  const label = accessibilityLabel === undefined ? providerFamilyLabel(family) : accessibilityLabel;
  return (
    <Svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      accessibilityRole={label ? "image" : undefined}
      accessibilityLabel={label ?? undefined}
      accessible={label != null}
    >
      <Mark family={family} color={color ?? tokens.ink2} />
    </Svg>
  );
}
