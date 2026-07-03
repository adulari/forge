import React from "react";
import { AbsoluteFill, interpolate, spring, useCurrentFrame, useVideoConfig } from "remotion";
import { C, FONT, glow } from "../theme";
import { Backdrop } from "../components/common";

const Bar: React.FC<{
  label: string;
  value: number; // 0..10
  color: string;
  progress: number;
  highlight?: boolean;
}> = ({ label, value, color, progress, highlight }) => {
  const track = 1240;
  const w = (value / 10) * track * progress;
  const shownVal = Math.round(value * progress);
  return (
    <div style={{ width: track, position: "relative", zIndex: 1 }}>
      <div style={{ position: "relative", zIndex: 2, marginBottom: 10, fontFamily: FONT, fontSize: 26, fontWeight: highlight ? 800 : 600, color: highlight ? C.ember : C.subtext }}>
        {label}
        <span style={{ color: highlight ? color : C.subtext, fontWeight: 800, textShadow: highlight ? glow(color, 0.5) : "none" }}>
          {"  —  "}{shownVal}
        </span>
        <span style={{ color: C.muted, fontWeight: 500, fontSize: 21 }}> / 10 bugs fixed</span>
      </div>
      <div style={{ width: track, height: 56, background: C.crust, borderRadius: 10, position: "relative", overflow: "hidden", border: `1px solid ${C.surface}` }}>
        <div
          style={{
            width: w,
            height: "100%",
            background: `linear-gradient(90deg, ${color}cc, ${color})`,
            borderRadius: 10,
            boxShadow: highlight ? glow(color, 0.6) : "none",
          }}
        />
      </div>
    </div>
  );
};

const StatCard: React.FC<{ big: string; small: string; color: string; s: number }> = ({ big, small, color, s }) => (
  <div
    style={{
      opacity: s,
      transform: `translateY(${(1 - s) * 20}px)`,
      background: C.mantle,
      border: `1px solid ${C.surface}`,
      borderRadius: 14,
      padding: "22px 34px",
      textAlign: "center",
      minWidth: 240,
    }}
  >
    <div style={{ fontFamily: FONT, fontWeight: 800, fontSize: 46, color, textShadow: glow(color, 0.4) }}>{big}</div>
    <div style={{ fontFamily: FONT, fontSize: 19, color: C.subtext, marginTop: 4 }}>{small}</div>
  </div>
);

export const Proof: React.FC<{ dur: number }> = ({ dur }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const titleS = spring({ frame: frame - 4, fps, config: { damping: 16 } });
  const bar1 = interpolate(frame, [36, 96], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: (t) => 1 - Math.pow(1 - t, 3) });
  const bar2 = interpolate(frame, [52, 112], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: (t) => 1 - Math.pow(1 - t, 3) });

  const cardS = (i: number) => spring({ frame: frame - 140 - i * 10, fps, config: { damping: 15 } });

  return (
    <AbsoluteFill style={{ opacity: interpolate(frame, [0, 12], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }), justifyContent: "center", alignItems: "center", flexDirection: "column", gap: 54 }}>
      <Backdrop tint={C.green} />

      <div style={{ textAlign: "center", opacity: titleS, transform: `translateY(${(1 - titleS) * 16}px)` }}>
        <div style={{ fontFamily: FONT, fontWeight: 800, fontSize: 46, color: C.text }}>
          Same model. <span style={{ color: C.ember }}>Better results.</span>
        </div>
        <div style={{ fontFamily: FONT, fontSize: 22, color: C.muted, marginTop: 8 }}>
          identical <span style={{ color: C.subtext }}>claude sonnet</span> · SWE-bench Lite · official evaluator
        </div>
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: 26 }}>
        <Bar label="raw claude CLI" value={4} color={C.overlay} progress={bar1} />
        <Bar label="through FORGE" value={6} color={C.green} progress={bar2} highlight />
      </div>

      <div style={{ display: "flex", gap: 28 }}>
        <StatCard big="+50%" small="more bugs fixed" color={C.green} s={cardS(0)} />
        <StatCard big="−21%" small="cost per fix" color={C.ember} s={cardS(1)} />
        <StatCard big="1,560" small="tests behind every claim" color={C.lavender} s={cardS(2)} />
        <StatCard big="1" small="static binary" color={C.blue} s={cardS(3)} />
      </div>
    </AbsoluteFill>
  );
};
