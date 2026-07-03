import React from "react";
import { AbsoluteFill, Sequence, interpolate, spring, useCurrentFrame, useVideoConfig } from "remotion";
import { loadFonts } from "./fonts";
import { C, FONT, glow } from "./theme";
import { Backdrop, Embers, Typewriter, edgeFade } from "./components/common";
import { Wordmark } from "./components/Wordmark";
import timeline from "./timeline.json";

loadFonts();

const S = timeline.vertical.scenes;
const XF = timeline.crossfade;
const W = 1080;

export const VERTICAL_DURATION = S.close[0] + S.close[1]; // 960

// ---------- hook: ignite + bold claim (first 2s must land) ----------

const VHook: React.FC<{ dur: number }> = ({ dur }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const ignite = interpolate(frame, [8, 40], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: (t) => 1 - Math.pow(1 - t, 2) });
  const pop = 0.92 + spring({ frame: frame - 8, fps, config: { damping: 14, mass: 0.6 } }) * 0.08;
  const line1 = spring({ frame: frame - 34, fps, config: { damping: 200 } });
  const line2 = spring({ frame: frame - 48, fps, config: { damping: 200 } });

  return (
    <AbsoluteFill style={{ opacity: edgeFade(frame, dur, 4, 14) }}>
      <Backdrop tint={C.orange} />
      <Embers count={16} opacity={ignite} width={W} height={1920} />
      <AbsoluteFill style={{ justifyContent: "center", alignItems: "center", flexDirection: "column", gap: 72 }}>
        <div style={{ transform: `scale(${pop})` }}>
          <Wordmark progress={ignite} cell={28} glowStrength={1.2} />
        </div>
        <div style={{ textAlign: "center", fontFamily: FONT }}>
          <div style={{ fontSize: 92, fontWeight: 800, color: C.text, opacity: line1, transform: `translateY(${(1 - line1) * 18}px)` }}>
            One agent.
          </div>
          <div style={{ fontSize: 92, fontWeight: 800, color: C.ember, textShadow: glow(C.orange, 0.5), opacity: line2, transform: `translateY(${(1 - line2) * 18}px)`, marginTop: 10 }}>
            Every model.
          </div>
        </div>
      </AbsoluteFill>
    </AbsoluteFill>
  );
};

// ---------- mesh failover, recomposed for vertical ----------

type VNode = { id: string; label: string; sub: string; color: string; x: number; y: number };
// grid centers — 2 cols x 3 rows in the lower half, inside safe margins
const NCOL = [312, 768];
const NROW = [960, 1210, 1460];
const VNODES: VNode[] = [
  { id: "claude", label: "claude", sub: "sonnet", color: C.ember, x: NCOL[0], y: NROW[0] },
  { id: "codex", label: "codex", sub: "gpt-5", color: C.green, x: NCOL[1], y: NROW[0] },
  { id: "gemini", label: "gemini", sub: "2.5-pro", color: C.blue, x: NCOL[0], y: NROW[1] },
  { id: "groq", label: "groq", sub: "kimi-k2", color: C.teal, x: NCOL[1], y: NROW[1] },
  { id: "cerebras", label: "cerebras", sub: "qwen3", color: C.yellow, x: NCOL[0], y: NROW[2] },
  { id: "ollama", label: "ollama", sub: "local", color: C.lavender, x: NCOL[1], y: NROW[2] },
];
const ROUTER_Y = 640; // router card center-ish; routes start below it

const vArc = (x2: number, y2: number) => {
  const x1 = 540;
  const y1 = ROUTER_Y + 130;
  const mx = (x1 + x2) / 2 + (x2 < 540 ? -60 : 60);
  const my = (y1 + y2) / 2;
  return `M ${x1} ${y1} Q ${mx} ${my} ${x2} ${y2 - 64}`;
};

const VMeter: React.FC<{ label: string; value: number; color: string }> = ({ label, value, color }) => (
  <div style={{ display: "flex", alignItems: "center", gap: 14, fontFamily: FONT, fontSize: 26 }}>
    <span style={{ color: C.subtext, width: 160, textAlign: "right", whiteSpace: "nowrap", flexShrink: 0 }}>{label}</span>
    <div style={{ width: 200, height: 12, background: C.crust, borderRadius: 6, overflow: "hidden", flexShrink: 0 }}>
      <div style={{ width: `${value * 100}%`, height: "100%", background: color, borderRadius: 6, boxShadow: glow(color, 0.5) }} />
    </div>
  </div>
);

const VMesh: React.FC<{ dur: number }> = ({ dur }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const picked = VNODES[0];
  const failover = VNODES[1];

  const nodesIn = (i: number) => spring({ frame: frame - 4 - i * 3, fps, config: { damping: 13 } });
  const weigh = interpolate(frame, [24, 64], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const scanning = frame >= 28 && frame < 64;
  const scanIdx = Math.floor(interpolate(frame, [28, 64], [0, VNODES.length], { extrapolateLeft: "clamp", extrapolateRight: "clamp" })) % VNODES.length;

  const routeDraw = interpolate(frame, [66, 92], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const flash429 = frame >= 96 && frame < 132;
  const flashPulse = flash429 ? 0.5 + 0.5 * Math.abs(Math.sin((frame - 96) / 4)) : 0;
  const routeBreak = interpolate(frame, [122, 132], [1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const foDraw = interpolate(frame, [132, 162], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const foSuccess = frame >= 162;
  const checkPop = spring({ frame: frame - 162, fps, config: { damping: 12 } });

  const capMain = interpolate(frame, [172, 190], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const capMain2 = interpolate(frame, [192, 208], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });

  return (
    <AbsoluteFill style={{ opacity: edgeFade(frame, dur) }}>
      <Backdrop tint={C.lavender} />

      {/* status caption — inside top safe margin */}
      <div style={{ position: "absolute", top: 236, left: 0, right: 0, textAlign: "center", fontFamily: FONT, fontSize: 36, color: C.subtext }}>
        <span style={{ color: C.ember }}>↳ </span>
        {frame < 66 ? "the router weighs the task" : frame < 132 ? "top pick: rate limited…" : "…re-routed. instantly."}
      </div>

      <svg width={W} height={1920} viewBox={`0 0 ${W} 1920`} style={{ position: "absolute", inset: 0 }}>
        {VNODES.map((n, i) => (
          <path key={n.id} d={vArc(n.x, n.y)} fill="none" stroke={scanning && i === scanIdx ? n.color : C.overlay} strokeWidth={scanning && i === scanIdx ? 4 : 2} strokeOpacity={nodesIn(i) * (scanning && i === scanIdx ? 0.9 : 0.3)} />
        ))}
        <path d={vArc(picked.x, picked.y)} fill="none" stroke={flash429 ? C.red : C.ember} strokeWidth={7} strokeLinecap="round" pathLength={1} strokeDasharray={1} strokeDashoffset={1 - routeDraw} strokeOpacity={routeBreak} style={{ filter: `drop-shadow(${glow(flash429 ? C.red : C.ember, 0.8)})` }} />
        <path d={vArc(failover.x, failover.y)} fill="none" stroke={C.green} strokeWidth={7} strokeLinecap="round" pathLength={1} strokeDasharray={1} strokeDashoffset={1 - foDraw} style={{ filter: `drop-shadow(${glow(C.green, 0.8)})` }} />
      </svg>

      {/* router card */}
      <div style={{ position: "absolute", left: 540 - 280, top: ROUTER_Y - 120, width: 560, background: C.mantle, border: `2.5px solid ${C.orange}`, borderRadius: 20, padding: "26px 30px", boxShadow: glow(C.orange, 0.5) }}>
        <div style={{ fontFamily: FONT, fontWeight: 800, fontSize: 34, color: C.ember, marginBottom: 18, letterSpacing: 1 }}>⚒ FORGE ROUTER</div>
        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <VMeter label="difficulty" value={0.72 * weigh} color={C.red} />
          <VMeter label="cost" value={0.34 * weigh} color={C.green} />
          <VMeter label="quota" value={0.9 * weigh} color={C.blue} />
        </div>
      </div>

      {/* model nodes */}
      {VNODES.map((n, i) => {
        const s = nodesIn(i);
        const red = n.id === picked.id && flash429;
        const green = n.id === failover.id && foSuccess;
        const borderCol = red ? C.red : green ? C.green : scanning && i === scanIdx ? n.color : `${C.overlay}`;
        return (
          <div key={n.id} style={{ position: "absolute", left: n.x - 170, top: n.y - 64, width: 340, transform: `translateY(${(1 - s) * 30}px) scale(${0.9 + s * 0.1})`, opacity: s, background: red ? `${C.red}1e` : green ? `${C.green}1e` : C.surface + "cc", border: `2.5px solid ${borderCol}`, borderRadius: 16, padding: "18px 22px", boxShadow: red ? glow(C.red, 0.6 * (0.6 + flashPulse)) : green ? glow(C.green, 0.7) : "none" }}>
            <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
              <div style={{ width: 16, height: 16, borderRadius: "50%", background: n.color, boxShadow: glow(n.color, 0.5), flexShrink: 0 }} />
              <div style={{ fontFamily: FONT, fontWeight: 700, fontSize: 32, color: C.text }}>{n.label}</div>
            </div>
            <div style={{ fontFamily: FONT, fontSize: 23, color: C.muted, marginTop: 4, marginLeft: 28 }}>{n.sub}</div>
            {red ? (
              <div style={{ position: "absolute", right: -10, top: -30, fontFamily: FONT, fontWeight: 800, fontSize: 26, color: C.red, background: C.crust, border: `2px solid ${C.red}`, borderRadius: 10, padding: "5px 14px", opacity: 0.6 + flashPulse * 0.4, whiteSpace: "nowrap" }}>
                429
              </div>
            ) : null}
            {green ? (
              <div style={{ position: "absolute", right: -14, top: -26, transform: `scale(${checkPop})`, fontFamily: FONT, fontWeight: 800, fontSize: 30, color: C.green, background: C.crust, border: `2px solid ${C.green}`, borderRadius: "50%", width: 52, height: 52, display: "flex", alignItems: "center", justifyContent: "center" }}>
                ✓
              </div>
            ) : null}
          </div>
        );
      })}

      {/* money caption — inside bottom safe margin */}
      <div style={{ position: "absolute", bottom: 260, left: 0, right: 0, textAlign: "center" }}>
        <div style={{ fontFamily: FONT, fontWeight: 800, fontSize: 68, color: C.text, opacity: capMain, transform: `translateY(${(1 - capMain) * 16}px)` }}>
          Automatic <span style={{ color: C.ember }}>failover</span>.
        </div>
        <div style={{ fontFamily: FONT, fontWeight: 500, fontSize: 40, color: C.subtext, opacity: capMain2, marginTop: 12 }}>
          You never notice.
        </div>
      </div>
    </AbsoluteFill>
  );
};

// ---------- vignette shell (vertical) ----------

const fadeInHold = (frame: number, d = XF) =>
  interpolate(frame, [0, d], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });

const VShell: React.FC<{ tag: string; caption: React.ReactNode; tint: string; children: React.ReactNode }> = ({ tag, caption, tint, children }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const tagS = spring({ frame: frame - 4, fps, config: { damping: 15 } });
  const capS = spring({ frame: frame - 14, fps, config: { damping: 200 } });
  return (
    <AbsoluteFill style={{ opacity: fadeInHold(frame) }}>
      <div style={{ position: "absolute", top: 260, left: 0, right: 0, textAlign: "center", opacity: tagS, transform: `translateY(${(1 - tagS) * -12}px)` }}>
        <span style={{ fontFamily: FONT, fontWeight: 800, fontSize: 58, color: tint, textShadow: glow(tint, 0.5) }}>{tag}</span>
      </div>
      <AbsoluteFill style={{ justifyContent: "center", alignItems: "center" }}>{children}</AbsoluteFill>
      <div style={{ position: "absolute", bottom: 280, left: 90, right: 90, textAlign: "center", opacity: capS }}>
        <span style={{ fontFamily: FONT, fontSize: 40, lineHeight: 1.45, color: C.subtext }}>{caption}</span>
      </div>
    </AbsoluteFill>
  );
};

const VRacer: React.FC<{ name: string; color: string; prog: number; win: boolean; frame: number; fps: number }> = ({ name, color, prog, win, frame, fps }) => {
  const bannerS = win ? spring({ frame: frame - 72, fps, config: { damping: 12 } }) : 0;
  return (
    <div style={{ position: "relative", width: 820, background: C.mantle, border: `3px solid ${win ? color : C.surface}`, borderRadius: 20, padding: 38, boxShadow: win ? glow(color, 0.5) : "none" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 16, marginBottom: 26 }}>
        <div style={{ width: 20, height: 20, borderRadius: "50%", background: color, boxShadow: glow(color, 0.5) }} />
        <span style={{ fontFamily: FONT, fontWeight: 700, fontSize: 40, color: C.text }}>{name}</span>
      </div>
      <div style={{ height: 22, background: C.crust, borderRadius: 11, overflow: "hidden" }}>
        <div style={{ width: `${prog * 100}%`, height: "100%", background: color, borderRadius: 11 }} />
      </div>
      <div style={{ marginTop: 18, fontFamily: FONT, fontSize: 28, color: C.muted }}>
        {prog >= 1 ? (win ? "solved · 8.1s" : "solved · 11.4s") : "solving…"}
      </div>
      {win ? (
        <div style={{ position: "absolute", top: -34, left: 28, transform: `scale(${bannerS})`, fontFamily: FONT, fontWeight: 800, fontSize: 30, color: C.base, background: color, padding: "8px 22px", borderRadius: 12, boxShadow: glow(color, 0.6) }}>
          ⚒ WINNER
        </div>
      ) : null}
    </div>
  );
};

const VDuel: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const left = interpolate(frame, [20, 68], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const right = interpolate(frame, [20, 100], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  return (
    <VShell tag="/duel" tint={C.ember} caption={<>two models, one task — <span style={{ color: C.ember }}>keep the winner</span></>}>
      <div style={{ display: "flex", flexDirection: "column", gap: 66, alignItems: "center" }}>
        <VRacer name="claude sonnet" color={C.ember} prog={left} win={frame >= 68} frame={frame} fps={fps} />
        <VRacer name="gemini pro" color={C.blue} prog={right} win={false} frame={frame} fps={fps} />
      </div>
    </VShell>
  );
};

const VAuto: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const tasks = ["fix flaky auth test", "bump deps + changelog", "add /health endpoint", "tidy error messages"];
  return (
    <VShell tag="queue autopilot" tint={C.blue} caption={<>queue tasks — they <span style={{ color: C.blue }}>drain overnight</span> into branches</>}>
      <div style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 44 }}>
        <div style={{ display: "flex", flexDirection: "column", gap: 18, width: 760 }}>
          <div style={{ fontFamily: FONT, fontSize: 30, color: C.muted, marginBottom: 4 }}>◐ queued</div>
          {tasks.map((t, i) => {
            const done = frame >= 30 + i * 16;
            const s = spring({ frame: frame - i * 4, fps, config: { damping: 200 } });
            return (
              <div key={i} style={{ display: "flex", gap: 18, fontFamily: FONT, fontSize: 33, opacity: done ? 0.4 : s, alignItems: "center" }}>
                <span style={{ color: done ? C.green : C.blue }}>{done ? "✓" : "○"}</span>
                <span style={{ color: C.text, textDecoration: done ? "line-through" : "none" }}>{t}</span>
              </div>
            );
          })}
        </div>
        <div style={{ fontSize: 64, opacity: interpolate(frame, [0, 20], [0, 1], { extrapolateRight: "clamp" }) }}>🌙</div>
        <div style={{ display: "flex", flexDirection: "column", gap: 18, width: 760 }}>
          <div style={{ fontFamily: FONT, fontSize: 30, color: C.muted, marginBottom: 4 }}>⎇ branches</div>
          <div style={{ display: "flex", flexWrap: "wrap", gap: 16 }}>
            {tasks.map((t, i) => {
              const born = frame >= 38 + i * 16;
              const s = born ? spring({ frame: frame - (38 + i * 16), fps, config: { damping: 13 } }) : 0;
              return (
                <div key={i} style={{ opacity: s, transform: `translateY(${(1 - s) * 14}px)`, fontFamily: FONT, fontSize: 30, color: C.green, background: `${C.green}18`, border: `1.5px solid ${C.green}66`, borderRadius: 10, padding: "10px 20px" }}>
                  forge/auto-{i + 1}
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </VShell>
  );
};

// ---------- proof (vertical) ----------

const VBar: React.FC<{ label: string; value: number; color: string; progress: number; highlight?: boolean }> = ({ label, value, color, progress, highlight }) => {
  const track = 840;
  const w = (value / 10) * track * progress;
  const shownVal = Math.round(value * progress);
  return (
    // position:relative lifts the bar above the scene Backdrop (an absolutely
    // positioned sibling painted after non-positioned content)
    <div style={{ width: track, position: "relative", zIndex: 1 }}>
      <div style={{ marginBottom: 14, fontFamily: FONT, fontSize: 36, fontWeight: highlight ? 800 : 600, color: highlight ? C.ember : C.subtext }}>
        {label}
        <span style={{ color: highlight ? color : C.subtext, fontWeight: 800, textShadow: highlight ? glow(color, 0.5) : "none" }}>
          {"  —  "}{shownVal}
        </span>
        <span style={{ color: C.muted, fontWeight: 500, fontSize: 28 }}> / 10</span>
      </div>
      <div style={{ width: track, height: 64, background: C.crust, borderRadius: 12, overflow: "hidden", border: `1px solid ${C.surface}` }}>
        <div style={{ width: w, height: "100%", background: `linear-gradient(90deg, ${color}cc, ${color})`, borderRadius: 12, boxShadow: highlight ? glow(color, 0.6) : "none" }} />
      </div>
    </div>
  );
};

const VStat: React.FC<{ big: string; small: string; color: string; s: number }> = ({ big, small, color, s }) => (
  <div style={{ opacity: s, transform: `translateY(${(1 - s) * 20}px)`, background: C.mantle, border: `1px solid ${C.surface}`, borderRadius: 18, padding: "30px 20px", textAlign: "center", width: 400 }}>
    <div style={{ fontFamily: FONT, fontWeight: 800, fontSize: 60, color, textShadow: glow(color, 0.4) }}>{big}</div>
    <div style={{ fontFamily: FONT, fontSize: 27, color: C.subtext, marginTop: 8 }}>{small}</div>
  </div>
);

const VProof: React.FC<{ dur: number }> = ({ dur }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const titleS = spring({ frame: frame - 4, fps, config: { damping: 16 } });
  const bar1 = interpolate(frame, [22, 70], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: (t) => 1 - Math.pow(1 - t, 3) });
  const bar2 = interpolate(frame, [34, 84], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: (t) => 1 - Math.pow(1 - t, 3) });
  const cardS = (i: number) => spring({ frame: frame - 96 - i * 8, fps, config: { damping: 15 } });

  return (
    <AbsoluteFill style={{ opacity: edgeFade(frame, dur, 12, 14), justifyContent: "center", alignItems: "center", flexDirection: "column", gap: 64 }}>
      <Backdrop tint={C.green} />
      <div style={{ textAlign: "center", opacity: titleS, transform: `translateY(${(1 - titleS) * 16}px)` }}>
        <div style={{ fontFamily: FONT, fontWeight: 800, fontSize: 58, color: C.text }}>
          Same model.
          <br />
          <span style={{ color: C.ember }}>Better results.</span>
        </div>
        <div style={{ fontFamily: FONT, fontSize: 29, color: C.muted, marginTop: 16 }}>
          SWE-bench Lite · official evaluator
        </div>
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 40 }}>
        <VBar label="raw claude CLI" value={4} color={C.overlay} progress={bar1} />
        <VBar label="through FORGE" value={6} color={C.green} progress={bar2} highlight />
      </div>
      <div style={{ display: "flex", flexWrap: "wrap", gap: 28, width: 840, justifyContent: "center" }}>
        <VStat big="+50%" small="more bugs fixed" color={C.green} s={cardS(0)} />
        <VStat big="−21%" small="cost per fix" color={C.ember} s={cardS(1)} />
      </div>
    </AbsoluteFill>
  );
};

// ---------- close (vertical) ----------

const VClose: React.FC<{ dur: number }> = ({ dur }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const igniteStart = 24;
  const ignite = interpolate(frame, [igniteStart, igniteStart + 34], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: (t) => 1 - Math.pow(1 - t, 2) });
  const markS = spring({ frame: frame - igniteStart, fps, config: { damping: 14 } });
  const slugS = spring({ frame: frame - 52, fps, config: { damping: 200 } });
  const cardS = spring({ frame: frame - 74, fps, config: { damping: 200 } });
  const outFade = interpolate(frame, [dur - 24, dur], [1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const inFade = interpolate(frame, [0, 12], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });

  return (
    <AbsoluteFill style={{ opacity: inFade * outFade, justifyContent: "center", alignItems: "center" }}>
      <Backdrop tint={C.orange} />
      <Embers count={18} opacity={0.7 + ignite * 0.6} width={W} height={1920} />
      <div style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 56, transform: `scale(${0.9 + markS * 0.1})` }}>
        <div style={{ opacity: ignite > 0 ? 1 : 0 }}>
          <Wordmark progress={ignite} cell={26} glowStrength={1.1} />
        </div>
        <div style={{ fontFamily: FONT, fontSize: 46, fontWeight: 700, color: C.text, opacity: slugS, display: "flex", alignItems: "center", gap: 16 }}>
          <span style={{ color: C.ember }}>⚒</span> github.com/Adulari/forge
        </div>
        <div style={{ fontFamily: FONT, fontSize: 32, color: C.subtext, opacity: slugS }}>
          one binary · every model · your terminal
        </div>
        <div style={{ opacity: cardS, transform: `translateY(${(1 - cardS) * 16}px)`, background: C.mantle, border: `1px solid ${C.surface}`, borderRadius: 16, padding: "30px 36px", fontFamily: FONT, fontSize: 30, lineHeight: 1.7, boxShadow: "0 30px 80px rgba(0,0,0,0.5)" }}>
          <div style={{ color: C.text }}>
            <span style={{ color: C.green }}>$ </span>
            <Typewriter text="curl -fsSL https://raw.githubusercontent.com" frame={frame} startFrame={78} cps={46} caret={false} style={{ fontSize: 30 }} />
          </div>
          <div style={{ color: C.text }}>
            <Typewriter text="  /Adulari/forge/main/install.sh | sh" frame={frame} startFrame={107} cps={46} caret={frame < 140} style={{ fontSize: 30 }} />
          </div>
        </div>
      </div>
    </AbsoluteFill>
  );
};

// ---------- composition ----------

export const Vertical: React.FC = () => {
  return (
    <AbsoluteFill>
      <Backdrop />
      <Sequence from={S.hook[0]} durationInFrames={S.hook[1]}><VHook dur={S.hook[1]} /></Sequence>
      <Sequence from={S.mesh[0]} durationInFrames={S.mesh[1]}><VMesh dur={S.mesh[1]} /></Sequence>
      <Sequence from={S.duel[0]} durationInFrames={S.duel[1] + XF}><VDuel /></Sequence>
      <Sequence from={S.auto[0]} durationInFrames={S.auto[1] + XF}><VAuto /></Sequence>
      <Sequence from={S.proof[0]} durationInFrames={S.proof[1]}><VProof dur={S.proof[1]} /></Sequence>
      <Sequence from={S.close[0]} durationInFrames={S.close[1]}><VClose dur={S.close[1]} /></Sequence>
    </AbsoluteFill>
  );
};
