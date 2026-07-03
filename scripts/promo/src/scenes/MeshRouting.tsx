import React from "react";
import { AbsoluteFill, interpolate, spring, useCurrentFrame, useVideoConfig } from "remotion";
import { C, FONT, glow } from "../theme";
import { Backdrop, edgeFade } from "../components/common";

const RX = 520;
const RY = 540;

type Node = { id: string; label: string; sub: string; color: string; x: number; y: number };
const NODES: Node[] = [
  { id: "claude", label: "claude", sub: "sonnet", color: C.ember, x: 1250, y: 300 },
  { id: "codex", label: "codex", sub: "gpt-5", color: C.green, x: 1540, y: 300 },
  { id: "gemini", label: "gemini", sub: "2.5-pro", color: C.blue, x: 1250, y: 540 },
  { id: "groq", label: "groq", sub: "kimi-k2", color: C.teal, x: 1540, y: 540 },
  { id: "cerebras", label: "cerebras", sub: "qwen3", color: C.yellow, x: 1250, y: 780 },
  { id: "ollama", label: "ollama", sub: "local", color: C.lavender, x: 1540, y: 780 },
];

const arcPath = (x1: number, y1: number, x2: number, y2: number) => {
  const mx = (x1 + x2) / 2;
  const my = (y1 + y2) / 2 - 70;
  return `M ${x1} ${y1} Q ${mx} ${my} ${x2} ${y2}`;
};

const Meter: React.FC<{ label: string; value: number; color: string }> = ({ label, value, color }) => (
  <div style={{ display: "flex", alignItems: "center", gap: 10, fontFamily: FONT, fontSize: 17 }}>
    <span style={{ color: C.subtext, width: 110, textAlign: "right", whiteSpace: "nowrap", flexShrink: 0 }}>{label}</span>
    <div style={{ width: 128, height: 9, background: C.crust, borderRadius: 5, overflow: "hidden", flexShrink: 0 }}>
      <div
        style={{
          width: `${value * 100}%`,
          height: "100%",
          background: color,
          borderRadius: 5,
          boxShadow: glow(color, 0.5),
        }}
      />
    </div>
  </div>
);

export const MeshRouting: React.FC<{ dur: number }> = ({ dur }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const picked = NODES[0]; // claude
  const failover = NODES[1]; // codex

  // node entrance
  const nodesIn = (i: number) => spring({ frame: frame - 6 - i * 4, fps, config: { damping: 13 } });

  // task pill flying in
  const taskT = interpolate(frame, [30, 78], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: (t) => 1 - Math.pow(1 - t, 3) });
  const taskX = interpolate(taskT, [0, 1], [-360, RX - 150]);
  const taskAbsorb = interpolate(frame, [78, 92], [1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });

  // router weighing
  const weigh = interpolate(frame, [92, 150], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const routerPulse = 1 + 0.04 * Math.sin(frame / 5) * interpolate(frame, [92, 150, 165], [0, 1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });

  // scanning highlight across nodes during weigh
  const scanIdx = Math.floor(interpolate(frame, [96, 150], [0, NODES.length], { extrapolateLeft: "clamp", extrapolateRight: "clamp" })) % NODES.length;
  const scanning = frame >= 96 && frame < 150;

  // primary route draw to claude
  const routeDraw = interpolate(frame, [156, 190], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  // 429 flash
  const flash429 = frame >= 196 && frame < 246;
  const flashPulse = flash429 ? 0.5 + 0.5 * Math.abs(Math.sin((frame - 196) / 4)) : 0;
  const routeBreak = interpolate(frame, [232, 246], [1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });

  // failover route draw to codex
  const foDraw = interpolate(frame, [246, 286], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const foSuccess = frame >= 286;
  const checkPop = spring({ frame: frame - 286, fps, config: { damping: 12 } });

  // captions
  const capMain = interpolate(frame, [300, 320], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const capMain2 = interpolate(frame, [326, 346], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });

  return (
    <AbsoluteFill style={{ opacity: edgeFade(frame, dur) }}>
      <Backdrop tint={C.lavender} />

      {/* top status caption */}
      <div
        style={{
          position: "absolute",
          top: 64,
          left: 0,
          right: 0,
          textAlign: "center",
          fontFamily: FONT,
          fontSize: 26,
          color: C.subtext,
        }}
      >
        <span style={{ color: C.ember }}>↳ </span>
        {frame < 150 ? "the router weighs difficulty · cost · quota" : frame < 246 ? "the top pick is rate limited…" : "…so Forge re-routes, instantly"}
      </div>

      {/* connecting lines */}
      <svg width={1920} height={1080} viewBox="0 0 1920 1080" style={{ position: "absolute", inset: 0 }}>
        {NODES.map((n, i) => (
          <path
            key={n.id}
            d={arcPath(RX + 90, RY, n.x - 70, n.y)}
            fill="none"
            stroke={scanning && i === scanIdx ? n.color : C.overlay}
            strokeWidth={scanning && i === scanIdx ? 3 : 1.5}
            strokeOpacity={nodesIn(i) * (scanning && i === scanIdx ? 0.9 : 0.28)}
          />
        ))}
        {/* primary route to claude */}
        <path
          d={arcPath(RX + 90, RY, picked.x - 70, picked.y)}
          fill="none"
          stroke={flash429 ? C.red : C.ember}
          strokeWidth={5}
          strokeLinecap="round"
          pathLength={1}
          strokeDasharray={1}
          strokeDashoffset={1 - routeDraw}
          strokeOpacity={routeBreak}
          style={{ filter: `drop-shadow(${glow(flash429 ? C.red : C.ember, 0.8)})` }}
        />
        {/* failover route to codex */}
        <path
          d={arcPath(RX + 90, RY, failover.x - 70, failover.y)}
          fill="none"
          stroke={C.green}
          strokeWidth={5}
          strokeLinecap="round"
          pathLength={1}
          strokeDasharray={1}
          strokeDashoffset={1 - foDraw}
          style={{ filter: `drop-shadow(${glow(C.green, 0.8)})` }}
        />
      </svg>

      {/* task pill */}
      <div
        style={{
          position: "absolute",
          left: taskX,
          top: RY - 26,
          transform: `scale(${taskAbsorb})`,
          opacity: taskAbsorb,
          fontFamily: FONT,
          fontSize: 20,
          fontWeight: 700,
          color: C.base,
          background: C.ember,
          padding: "10px 18px",
          borderRadius: 10,
          boxShadow: glow(C.orange, 0.7),
        }}
      >
        ⚒ fix the failing test
      </div>

      {/* router */}
      <div
        style={{
          position: "absolute",
          left: RX - 90,
          top: RY - 96,
          width: 300,
          transform: `scale(${routerPulse})`,
          transformOrigin: "left center",
          background: C.mantle,
          border: `2px solid ${C.orange}`,
          borderRadius: 16,
          padding: "16px 18px",
          boxShadow: glow(C.orange, 0.5),
        }}
      >
        <div style={{ fontFamily: FONT, fontWeight: 800, fontSize: 22, color: C.ember, marginBottom: 12, letterSpacing: 1 }}>
          ⚒ FORGE ROUTER
        </div>
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <Meter label="difficulty" value={0.72 * weigh} color={C.red} />
          <Meter label="cost" value={0.34 * weigh} color={C.green} />
          <Meter label="quota" value={0.9 * weigh} color={C.blue} />
        </div>
      </div>

      {/* model nodes */}
      {NODES.map((n, i) => {
        const s = nodesIn(i);
        const isPicked = n.id === picked.id;
        const isFail = n.id === failover.id;
        const red = isPicked && flash429;
        const green = isFail && foSuccess;
        const borderCol = red ? C.red : green ? C.green : scanning && i === scanIdx ? n.color : `${C.overlay}`;
        return (
          <div
            key={n.id}
            style={{
              position: "absolute",
              left: n.x - 70,
              top: n.y - 34,
              width: 168,
              transform: `translateX(${(1 - s) * 40}px) scale(${0.9 + s * 0.1})`,
              opacity: s,
              background: red ? `${C.red}1e` : green ? `${C.green}1e` : C.surface + "cc",
              border: `2px solid ${borderCol}`,
              borderRadius: 12,
              padding: "10px 14px",
              boxShadow: red ? glow(C.red, 0.6 * (0.6 + flashPulse)) : green ? glow(C.green, 0.7) : "none",
            }}
          >
            <div style={{ display: "flex", alignItems: "center", gap: 9 }}>
              <div
                style={{
                  width: 12,
                  height: 12,
                  borderRadius: "50%",
                  background: n.color,
                  boxShadow: glow(n.color, 0.5),
                  flexShrink: 0,
                }}
              />
              <div style={{ fontFamily: FONT, fontWeight: 700, fontSize: 20, color: C.text }}>{n.label}</div>
            </div>
            <div style={{ fontFamily: FONT, fontSize: 15, color: C.muted, marginTop: 3, marginLeft: 21 }}>{n.sub}</div>
            {red ? (
              <div style={{ position: "absolute", right: -12, top: -16, fontFamily: FONT, fontWeight: 800, fontSize: 16, color: C.red, background: C.crust, border: `1.5px solid ${C.red}`, borderRadius: 8, padding: "3px 9px", opacity: 0.6 + flashPulse * 0.4 }}>
                429 rate limited
              </div>
            ) : null}
            {green ? (
              <div style={{ position: "absolute", right: -14, top: -16, transform: `scale(${checkPop})`, fontFamily: FONT, fontWeight: 800, fontSize: 22, color: C.green, background: C.crust, border: `1.5px solid ${C.green}`, borderRadius: "50%", width: 38, height: 38, display: "flex", alignItems: "center", justifyContent: "center" }}>
                ✓
              </div>
            ) : null}
          </div>
        );
      })}

      {/* money caption */}
      <div style={{ position: "absolute", bottom: 92, left: 0, right: 0, textAlign: "center" }}>
        <div style={{ fontFamily: FONT, fontWeight: 800, fontSize: 52, color: C.text, opacity: capMain, transform: `translateY(${(1 - capMain) * 14}px)` }}>
          Automatic <span style={{ color: C.ember }}>failover</span>.
        </div>
        <div style={{ fontFamily: FONT, fontWeight: 500, fontSize: 30, color: C.subtext, opacity: capMain2, marginTop: 8 }}>
          You never notice.
        </div>
      </div>
    </AbsoluteFill>
  );
};
