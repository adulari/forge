import React from "react";
import { AbsoluteFill, Sequence, interpolate, spring, useCurrentFrame, useVideoConfig } from "remotion";
import { C, FONT, glow } from "../theme";
import { Backdrop, edgeFade } from "../components/common";

const VIG = 120; // frames per vignette

const Shell: React.FC<{
  tag: string;
  caption: React.ReactNode;
  children: React.ReactNode;
  tint: string;
}> = ({ tag, caption, children, tint }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const tagS = spring({ frame: frame - 4, fps, config: { damping: 15 } });
  const capS = spring({ frame: frame - 14, fps, config: { damping: 200 } });
  return (
    <AbsoluteFill style={{ opacity: edgeFade(frame, VIG, 10, 12) }}>
      <div style={{ position: "absolute", top: 84, left: 0, right: 0, textAlign: "center", opacity: tagS, transform: `translateY(${(1 - tagS) * -12}px)` }}>
        <span style={{ fontFamily: FONT, fontWeight: 800, fontSize: 40, color: tint, textShadow: glow(tint, 0.5) }}>{tag}</span>
      </div>
      <AbsoluteFill style={{ justifyContent: "center", alignItems: "center" }}>{children}</AbsoluteFill>
      <div style={{ position: "absolute", bottom: 110, left: 0, right: 0, textAlign: "center", opacity: capS }}>
        <span style={{ fontFamily: FONT, fontSize: 27, color: C.subtext }}>{caption}</span>
      </div>
    </AbsoluteFill>
  );
};

const RacerPanel: React.FC<{ name: string; color: string; prog: number; win: boolean; frame: number; fps: number; side: "l" | "r" }> = ({ name, color, prog, win, frame, fps, side }) => {
  const bannerS = win ? spring({ frame: frame - 70, fps, config: { damping: 12 } }) : 0;
  return (
    <div style={{ position: "relative", width: 460, background: C.mantle, border: `2px solid ${win ? color : C.surface}`, borderRadius: 16, padding: 26, boxShadow: win ? glow(color, 0.5) : "none" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 18 }}>
        <div style={{ width: 14, height: 14, borderRadius: "50%", background: color, boxShadow: glow(color, 0.5) }} />
        <span style={{ fontFamily: FONT, fontWeight: 700, fontSize: 26, color: C.text }}>{name}</span>
      </div>
      <div style={{ height: 16, background: C.crust, borderRadius: 8, overflow: "hidden" }}>
        <div style={{ width: `${prog * 100}%`, height: "100%", background: color, borderRadius: 8 }} />
      </div>
      <div style={{ marginTop: 12, fontFamily: FONT, fontSize: 18, color: C.muted }}>
        {prog >= 1 ? (win ? "solved · 8.1s" : "solved · 11.4s") : "solving…"}
      </div>
      {win ? (
        <div style={{ position: "absolute", top: -26, left: side === "l" ? 20 : "auto", right: side === "r" ? 20 : "auto", transform: `scale(${bannerS})`, fontFamily: FONT, fontWeight: 800, fontSize: 20, color: C.base, background: color, padding: "6px 16px", borderRadius: 10, boxShadow: glow(color, 0.6) }}>
          ⚒ WINNER
        </div>
      ) : null}
    </div>
  );
};

const Duel: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const left = interpolate(frame, [22, 66], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const right = interpolate(frame, [22, 92], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  return (
    <Shell tag="/duel" tint={C.ember} caption={<>race two models on one task — <span style={{ color: C.ember }}>keep the winner</span></>}>
      <div style={{ display: "flex", gap: 48, alignItems: "flex-start" }}>
        <RacerPanel name="claude sonnet" color={C.ember} prog={left} win={frame >= 66} frame={frame} fps={fps} side="l" />
        <RacerPanel name="gemini pro" color={C.blue} prog={right} win={false} frame={frame} fps={fps} side="r" />
      </div>
    </Shell>
  );
};

const Workflows: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const agents = [
    { a: -150, c: C.ember, l: "lint" },
    { a: -90, c: C.green, l: "test" },
    { a: -30, c: C.blue, l: "docs" },
    { a: 30, c: C.lavender, l: "refactor" },
    { a: 90, c: C.teal, l: "bench" },
    { a: 150, c: C.yellow, l: "review" },
  ];
  return (
    <Shell tag="workflows" tint={C.lavender} caption={<>one JS script fans out into <span style={{ color: C.lavender }}>parallel agents</span></>}>
      <div style={{ position: "relative", width: 900, height: 480 }}>
        <div style={{ position: "absolute", left: 350, top: 200, width: 200, background: C.mantle, border: `2px solid ${C.lavender}`, borderRadius: 12, padding: "14px 16px", fontFamily: FONT, fontSize: 18, color: C.text, textAlign: "center", boxShadow: glow(C.lavender, 0.4), zIndex: 2 }}>
          <span style={{ color: C.lavender }}>{"{ }"}</span> workflow.js
        </div>
        <svg width={900} height={480} style={{ position: "absolute", inset: 0 }}>
          {agents.map((ag, i) => {
            const s = spring({ frame: frame - 20 - i * 6, fps, config: { damping: 14 } });
            const rad = (ag.a * Math.PI) / 180;
            const dist = 250 * s;
            const ex = 450 + Math.sin(rad) * dist;
            const ey = 232 - Math.cos(rad) * dist * 0.62 - 40;
            return <line key={i} x1={450} y1={232} x2={ex} y2={ey} stroke={ag.c} strokeWidth={2} strokeOpacity={0.5 * s} />;
          })}
        </svg>
        {agents.map((ag, i) => {
          const s = spring({ frame: frame - 20 - i * 6, fps, config: { damping: 14 } });
          const rad = (ag.a * Math.PI) / 180;
          const dist = 250 * s;
          const ex = 450 + Math.sin(rad) * dist;
          const ey = 232 - Math.cos(rad) * dist * 0.62 - 40;
          return (
            <div key={i} style={{ position: "absolute", left: ex - 44, top: ey - 22, opacity: s, transform: `scale(${s})` }}>
              <div style={{ display: "flex", alignItems: "center", gap: 8, background: C.surface, border: `1.5px solid ${ag.c}`, borderRadius: 20, padding: "6px 14px" }}>
                <div style={{ width: 10, height: 10, borderRadius: "50%", background: ag.c, boxShadow: glow(ag.c, 0.5) }} />
                <span style={{ fontFamily: FONT, fontSize: 17, color: C.text }}>{ag.l}</span>
              </div>
            </div>
          );
        })}
      </div>
    </Shell>
  );
};

const Autopilot: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const tasks = ["fix flaky auth test", "bump deps + changelog", "add /health endpoint", "tidy error messages"];
  return (
    <Shell tag="queue autopilot" tint={C.blue} caption={<>queue tasks — they <span style={{ color: C.blue }}>drain overnight</span> into branches</>}>
      <div style={{ display: "flex", alignItems: "center", gap: 80 }}>
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ fontFamily: FONT, fontSize: 20, color: C.muted, marginBottom: 6 }}>◐ queued</div>
          {tasks.map((t, i) => {
            const done = frame >= 26 + i * 18;
            const s = spring({ frame: frame - i * 4, fps, config: { damping: 200 } });
            return (
              <div key={i} style={{ display: "flex", gap: 12, fontFamily: FONT, fontSize: 21, opacity: done ? 0.4 : s, alignItems: "center" }}>
                <span style={{ color: done ? C.green : C.blue }}>{done ? "✓" : "○"}</span>
                <span style={{ color: C.text, textDecoration: done ? "line-through" : "none" }}>{t}</span>
              </div>
            );
          })}
        </div>
        <div style={{ fontSize: 60, opacity: interpolate(frame, [0, 20], [0, 1], { extrapolateRight: "clamp" }) }}>🌙</div>
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ fontFamily: FONT, fontSize: 20, color: C.muted, marginBottom: 6 }}>⎇ branches</div>
          {tasks.map((t, i) => {
            const born = frame >= 34 + i * 18;
            const s = born ? spring({ frame: frame - (34 + i * 18), fps, config: { damping: 13 } }) : 0;
            return (
              <div key={i} style={{ opacity: s, transform: `translateX(${(1 - s) * -20}px)`, fontFamily: FONT, fontSize: 19, color: C.green, background: `${C.green}18`, border: `1px solid ${C.green}66`, borderRadius: 8, padding: "6px 12px" }}>
                forge/auto-{i + 1}
              </div>
            );
          })}
        </div>
      </div>
    </Shell>
  );
};

const Blame: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const arrowS = spring({ frame: frame - 34, fps, config: { damping: 16 } });
  const tagS = spring({ frame: frame - 52, fps, config: { damping: 13 } });
  return (
    <Shell tag="forge blame" tint={C.teal} caption={<>every line knows <span style={{ color: C.teal }}>which model wrote it — and what it cost</span></>}>
      <div style={{ display: "flex", alignItems: "center", gap: 40 }}>
        <div style={{ background: C.mantle, border: `1px solid ${C.surface}`, borderRadius: 12, padding: "20px 26px", fontFamily: FONT, fontSize: 22 }}>
          <span style={{ color: C.muted }}>42 </span>
          <span style={{ color: C.blue }}>let</span>
          <span style={{ color: C.text }}> page = </span>
          <span style={{ color: C.green }}>query.offset</span>
          <span style={{ color: C.text }}>(n).limit(</span>
          <span style={{ color: C.yellow }}>25</span>
          <span style={{ color: C.text }}>);</span>
        </div>
        <div style={{ fontFamily: FONT, fontSize: 40, color: C.teal, opacity: arrowS, transform: `translateX(${(1 - arrowS) * -20}px)` }}>↳</div>
        <div style={{ opacity: tagS, transform: `scale(${tagS})`, display: "flex", flexDirection: "column", gap: 10, background: C.surface, border: `1.5px solid ${C.teal}`, borderRadius: 12, padding: "16px 22px" }}>
          <div style={{ display: "flex", alignItems: "center", gap: 10, fontFamily: FONT, fontSize: 22, color: C.text }}>
            <div style={{ width: 12, height: 12, borderRadius: "50%", background: C.blue, boxShadow: glow(C.blue, 0.5) }} />
            gemini 2.5-pro
          </div>
          <div style={{ fontFamily: FONT, fontSize: 20, color: C.green }}>$0.0021 · 1.2k tok</div>
        </div>
      </div>
    </Shell>
  );
};

const Fork: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const split = interpolate(frame, [26, 70], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: (t) => 1 - Math.pow(1 - t, 3) });
  const diffS = spring({ frame: frame - 66, fps, config: { damping: 15 } });
  const branchY = 90 * split;
  return (
    <Shell tag="fork" tint={C.ember} caption={<>branch a live session — <span style={{ color: C.ember }}>explore two ways at once</span></>}>
      <div style={{ position: "relative", width: 900, height: 360 }}>
        <svg width={900} height={360} style={{ position: "absolute", inset: 0 }}>
          <line x1={40} y1={180} x2={420} y2={180} stroke={C.overlay} strokeWidth={4} strokeLinecap="round" />
          {[80, 200, 340].map((x, i) => (
            <circle key={i} cx={x} cy={180} r={9} fill={C.subtext} />
          ))}
          <path d={`M 420 180 Q 520 180 620 ${180 - branchY}`} fill="none" stroke={C.ember} strokeWidth={4} strokeLinecap="round" opacity={split} />
          <path d={`M 420 180 Q 520 180 620 ${180 + branchY}`} fill="none" stroke={C.blue} strokeWidth={4} strokeLinecap="round" opacity={split} />
          {split > 0.5 ? (
            <>
              <circle cx={620} cy={180 - branchY} r={10} fill={C.ember} />
              <circle cx={620} cy={180 + branchY} r={10} fill={C.blue} />
              <circle cx={820} cy={180 - branchY} r={10} fill={C.ember} opacity={interpolate(frame, [60, 74], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" })} />
              <circle cx={820} cy={180 + branchY} r={10} fill={C.blue} opacity={interpolate(frame, [66, 80], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" })} />
              <line x1={620} y1={180 - branchY} x2={820} y2={180 - branchY} stroke={C.ember} strokeWidth={4} strokeLinecap="round" opacity={interpolate(frame, [58, 74], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" })} />
              <line x1={620} y1={180 + branchY} x2={820} y2={180 + branchY} stroke={C.blue} strokeWidth={4} strokeLinecap="round" opacity={interpolate(frame, [64, 80], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" })} />
            </>
          ) : null}
        </svg>
        <div style={{ position: "absolute", left: 636, top: 180 - branchY - 44, opacity: diffS, fontFamily: FONT, fontSize: 17, color: C.green }}>+ redis cache</div>
        <div style={{ position: "absolute", left: 636, top: 180 + branchY + 20, opacity: diffS, fontFamily: FONT, fontSize: 17, color: C.blue }}>+ in-memory lru</div>
      </div>
    </Shell>
  );
};

export const FeatureRun: React.FC<{ dur: number }> = () => {
  return (
    <AbsoluteFill>
      <Backdrop tint={C.ember} />
      <Sequence from={0} durationInFrames={VIG}><Duel /></Sequence>
      <Sequence from={VIG} durationInFrames={VIG}><Workflows /></Sequence>
      <Sequence from={VIG * 2} durationInFrames={VIG}><Autopilot /></Sequence>
      <Sequence from={VIG * 3} durationInFrames={VIG}><Blame /></Sequence>
      <Sequence from={VIG * 4} durationInFrames={VIG}><Fork /></Sequence>
    </AbsoluteFill>
  );
};

export { Duel };
