import { hexToRgba, type ColorTokens } from "../../theme/tokens";

/** Hover and selection are neutral ink washes: the ember accent is reserved for "needs you". */
export function rowBackground(tokens: ColorTokens, state: { hovered?: boolean; pressed?: boolean; selected?: boolean }): string {
  if (state.selected) return hexToRgba(tokens.ink, 0.075);
  if (state.pressed) return hexToRgba(tokens.ink, 0.09);
  if (state.hovered) return hexToRgba(tokens.ink, 0.045);
  return "transparent";
}
