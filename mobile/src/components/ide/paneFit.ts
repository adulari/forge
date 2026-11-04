// How wide the rail and agent panel really are. The stored widths are preferences; a narrow
// window gives up agent-panel width first, then rail width, before the editor drops below
// what a diff needs to be readable.
import { IDE_AGENT_WIDTH, IDE_RAIL_WIDTH } from "../../lib/ideModePrefs";

export const EDITOR_MIN_WIDTH = 480;
export const SEAM = 1;

export function fitPanes(windowWidth: number, rail: number, agent: number): { rail: number; agent: number; editor: number } {
  let r = rail;
  let a = agent;
  const spare = () => windowWidth - r - a - SEAM * 2;
  if (spare() < EDITOR_MIN_WIDTH) a = Math.max(IDE_AGENT_WIDTH.min, a - (EDITOR_MIN_WIDTH - spare()));
  if (spare() < EDITOR_MIN_WIDTH) r = Math.max(IDE_RAIL_WIDTH.min, r - (EDITOR_MIN_WIDTH - spare()));
  return { rail: r, agent: a, editor: Math.max(0, spare()) };
}
