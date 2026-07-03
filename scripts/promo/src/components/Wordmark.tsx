import React from "react";
import { C, glow } from "../theme";

// 5x7 pixel font for F O R G E.
const GLYPHS: Record<string, string[]> = {
  F: ["11111", "10000", "10000", "11110", "10000", "10000", "10000"],
  O: ["01110", "10001", "10001", "10001", "10001", "10001", "01110"],
  R: ["11110", "10001", "10001", "11110", "10100", "10010", "10001"],
  G: ["01110", "10001", "10000", "10111", "10001", "10001", "01110"],
  E: ["11111", "10000", "10000", "11110", "10000", "10000", "11111"],
};

const WORD = "FORGE";
const COLS = 5;
const ROWS = 7;
const GAP = 1;

type Pixel = { x: number; y: number; order: number };

const buildPixels = (): { pixels: Pixel[]; width: number } => {
  const pixels: Pixel[] = [];
  let cursor = 0;
  for (const ch of WORD) {
    const g = GLYPHS[ch];
    for (let r = 0; r < ROWS; r++) {
      for (let c = 0; c < COLS; c++) {
        if (g[r][c] === "1") {
          pixels.push({ x: cursor + c, y: r, order: cursor + c });
        }
      }
    }
    cursor += COLS + GAP;
  }
  const width = cursor - GAP;
  return { pixels, width };
};

const { pixels, width: GRID_W } = buildPixels();
const MAX_X = GRID_W;

export const Wordmark: React.FC<{
  // 0..1 ignition progress across the wordmark
  progress: number;
  // pixel edge size in px
  cell?: number;
  glowStrength?: number;
}> = ({ progress, cell = 18, glowStrength = 1 }) => {
  const gapPx = 2;
  const step = cell + gapPx;
  const w = GRID_W * step - gapPx;
  const h = ROWS * step - gapPx;
  // ignition sweeps left->right; each pixel lights over a short ramp
  const litFront = progress * (MAX_X + 3);

  return (
    <svg
      width={w}
      height={h}
      viewBox={`0 0 ${w} ${h}`}
      style={{ display: "block", overflow: "visible" }}
      shapeRendering="crispEdges"
    >
      {pixels.map((p, i) => {
        const local = litFront - p.x;
        const lit = Math.max(0, Math.min(1, local / 2));
        const px = p.x * step;
        const py = p.y * step;
        const opacity = 0.06 + lit * 0.94;
        const fill = lit > 0.5 ? C.ember : C.emberDeep;
        return (
          <rect
            key={i}
            x={px}
            y={py}
            width={cell}
            height={cell}
            rx={2}
            fill={fill}
            opacity={opacity}
            style={{
              filter: lit > 0.15 ? `drop-shadow(${glow(C.orange, glowStrength * lit)})` : "none",
            }}
          />
        );
      })}
    </svg>
  );
};

export const WORDMARK_ASPECT = GRID_W / ROWS;
