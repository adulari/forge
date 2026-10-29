import type React from "react";

import type { Rankable } from "../../lib/palette/fuzzy";
import type { PaletteSection } from "../../lib/palette/sections";

export interface PaletteItem extends Rankable {
  id: string;
  section: PaletteSection;
  disabled?: boolean;
  leading?: React.ReactNode;
  trailing?: React.ReactNode;
  /** Set when `trailing` renders its own interactive control (e.g. a trailing IconButton). */
  trailingInteractive?: boolean;
  /** The local session this row opens, so a message hit replaces the live row for the same session. */
  sessionId?: string;
  /** Paint the typed text in accent inside the title and subtitle (search hits). */
  highlight?: boolean;
  onSelect: () => void;
}

/** `"search"` (⌘P) opens the same palette on the Sessions chip. */
export type PaletteMode = "default" | "search";

export function optionDomId(rowId: string): string {
  return `palette-opt-${rowId.replace(/[^a-zA-Z0-9_-]/g, "_")}`;
}

export const PALETTE_LISTBOX_ID = "palette-listbox";
