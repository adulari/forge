import React from "react";
import { AbsoluteFill, interpolate, useCurrentFrame, random } from "remotion";
import { C, FONT, glow } from "../theme";

export const edgeFade = (
  frame: number,
  dur: number,
  fadeIn = 12,
  fadeOut = 14,
) =>
  interpolate(
    frame,
    [0, fadeIn, dur - fadeOut, dur],
    [0, 1, 1, 0],
    { extrapolateLeft: "clamp", extrapolateRight: "clamp" },
  );

// Dark base + faint hex/dot grid + vignette. Static, cheap, on-brand.
export const Backdrop: React.FC<{ tint?: string }> = ({ tint }) => {
  return (
    <AbsoluteFill
      style={{
        background: `radial-gradient(circle at 50% 42%, ${C.base} 0%, ${C.mantle} 55%, ${C.crust} 100%)`,
      }}
    >
      <AbsoluteFill
        style={{
          backgroundImage: `radial-gradient(${C.overlay}22 1px, transparent 1px)`,
          backgroundSize: "34px 34px",
          opacity: 0.5,
        }}
      />
      {tint ? (
        <AbsoluteFill
          style={{
            background: `radial-gradient(circle at 50% 46%, ${tint}14 0%, transparent 60%)`,
          }}
        />
      ) : null}
      <AbsoluteFill
        style={{
          boxShadow: "inset 0 0 320px rgba(0,0,0,0.65)",
        }}
      />
    </AbsoluteFill>
  );
};

// Floating ember sparks drifting up. Deterministic via random(seed).
export const Embers: React.FC<{ count?: number; opacity?: number; width?: number; height?: number }> = ({
  count = 26,
  opacity = 1,
  width = 1920,
  height = 1080,
}) => {
  const frame = useCurrentFrame();
  return (
    <AbsoluteFill style={{ pointerEvents: "none" }}>
      {new Array(count).fill(0).map((_, i) => {
        const seed = i * 3.31;
        const x = random(`x${seed}`) * width;
        const speed = 0.35 + random(`s${seed}`) * 0.9;
        const size = 1.5 + random(`z${seed}`) * 3.5;
        const phase = random(`p${seed}`) * 1000;
        const y = ((height + 100) - ((frame * speed + phase) % (height + 180))) as number;
        const sway = Math.sin((frame + phase) / 26) * 18;
        const flick = 0.3 + 0.7 * Math.abs(Math.sin((frame + phase) / 12));
        return (
          <div
            key={i}
            style={{
              position: "absolute",
              left: x + sway,
              top: y,
              width: size,
              height: size,
              borderRadius: "50%",
              background: C.ember,
              opacity: flick * 0.55 * opacity,
              boxShadow: glow(C.orange, 0.5),
            }}
          />
        );
      })}
    </AbsoluteFill>
  );
};

// Kinetic typewriter line.
export const Typewriter: React.FC<{
  text: string;
  startFrame?: number;
  cps?: number; // chars per second (at 30fps assume caller passes frame)
  frame: number;
  style?: React.CSSProperties;
  caret?: boolean;
}> = ({ text, startFrame = 0, cps = 34, frame, style, caret = true }) => {
  const elapsed = Math.max(0, frame - startFrame);
  const chars = Math.floor((elapsed / 30) * cps);
  const shown = text.slice(0, chars);
  const done = chars >= text.length;
  const blink = Math.floor(frame / 15) % 2 === 0;
  return (
    <span style={{ fontFamily: FONT, whiteSpace: "pre", ...style }}>
      {shown}
      {caret && (!done || blink) ? (
        <span style={{ color: C.ember, opacity: done ? (blink ? 1 : 0) : 1 }}>▊</span>
      ) : null}
    </span>
  );
};

export const Chip: React.FC<{
  children: React.ReactNode;
  color: string;
  style?: React.CSSProperties;
}> = ({ children, color, style }) => (
  <span
    style={{
      fontFamily: FONT,
      fontWeight: 700,
      color,
      border: `1.5px solid ${color}66`,
      background: `${color}14`,
      borderRadius: 8,
      padding: "6px 14px",
      ...style,
    }}
  >
    {children}
  </span>
);
