const MIN_TILE = 150;
const GAP = 8;

/** Columns of at least MIN_TILE wide that fit `width`, and the width each tile gets. */
export function tileLayout(width: number): { columns: number; tile: number } {
  const columns = Math.max(1, Math.floor((width + GAP) / (MIN_TILE + GAP)));
  return { columns, tile: Math.floor((width - GAP * (columns - 1)) / columns) };
}

export { MIN_TILE, GAP };
