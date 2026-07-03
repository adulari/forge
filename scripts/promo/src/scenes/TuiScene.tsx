import React from "react";
import { AbsoluteFill, interpolate, spring, useCurrentFrame, useVideoConfig } from "remotion";
import { C, FONT, glow } from "../theme";
import { Backdrop, edgeFade, Typewriter } from "../components/common";
import { TerminalWindow } from "../components/TerminalWindow";
import { Wordmark } from "../components/Wordmark";

type Line = { at: number; render: (frame: number) => React.ReactNode };

const dot = (color: string) => (
  <span style={{ color, textShadow: glow(color, 0.4) }}>●</span>
);

export const TuiScene: React.FC<{ dur: number }> = ({ dur }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const winPop = spring({ frame, fps, config: { damping: 15, mass: 0.7 } });
  const winScale = 0.94 + winPop * 0.06;

  // input typing then submit at 74
  const submitted = frame >= 74;

  const lines: Line[] = [
    {
      at: 78,
      render: () => (
        <div style={{ color: C.text }}>
          <span style={{ color: C.ember }}>› </span>add pagination to the user list
        </div>
      ),
    },
    {
      at: 92,
      render: () => (
        <div style={{ color: C.subtext }}>
          <span style={{ color: C.ember }}>↳</span> routed to{" "}
          <span style={{ color: C.ember }}>claude-cli::sonnet</span>
          <span style={{ color: C.muted }}> · effort medium · cheapest capable</span>
        </div>
      ),
    },
    {
      at: 116,
      render: () => (
        <div style={{ color: C.text }}>
          {dot(C.blue)} <span style={{ color: C.blue }}>read_file</span>{" "}
          <span style={{ color: C.subtext }}>src/users.rs</span>
        </div>
      ),
    },
    {
      at: 142,
      render: () => (
        <div style={{ color: C.text }}>
          {dot(C.blue)} <span style={{ color: C.blue }}>read_file</span>{" "}
          <span style={{ color: C.subtext }}>src/routes.rs</span>
        </div>
      ),
    },
    {
      at: 170,
      render: () => (
        <div style={{ color: C.text }}>
          {dot(C.ember)} <span style={{ color: C.ember }}>edit</span>{" "}
          <span style={{ color: C.subtext }}>src/routes.rs</span>{" "}
          <span style={{ color: C.green }}>+18</span> <span style={{ color: C.red }}>−2</span>
        </div>
      ),
    },
    {
      at: 202,
      render: (f) => (
        <div style={{ color: C.text }}>
          {dot(C.yellow)} <span style={{ color: C.yellow }}>shell</span>{" "}
          <span style={{ color: C.subtext }}>cargo test</span>
          {f >= 236 ? (
            <span style={{ color: C.green }}> → ✓ 42 passed</span>
          ) : (
            <span style={{ color: C.muted }}> → running…</span>
          )}
        </div>
      ),
    },
    {
      at: 262,
      render: (f) => (
        <div style={{ color: C.green, marginTop: 6 }}>
          ✓{" "}
          <span style={{ color: C.text }}>
            <Typewriter
              text="Added limit/offset pagination to /users — tested, all green."
              frame={f}
              startFrame={262}
              cps={44}
              caret={false}
            />
          </span>
        </div>
      ),
    },
  ];

  // token gauge grows as work happens
  const tokens = interpolate(frame, [92, 260], [0.8, 12.4], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  const gaugeFill = Math.min(1, tokens / 200);
  const gaugeCells = 24;
  const filled = Math.round(gaugeFill * gaugeCells * 8);

  // task panel items reveal
  const tasks = [
    { at: 120, label: "read current user routes" },
    { at: 175, label: "add limit/offset params" },
    { at: 240, label: "run test suite" },
  ];

  return (
    <AbsoluteFill style={{ opacity: edgeFade(frame, dur), justifyContent: "center", alignItems: "center" }}>
      <Backdrop tint={C.ember} />
      <div style={{ transform: `scale(${winScale})`, opacity: winPop }}>
        <TerminalWindow width={1640} height={900} title="forge chat">
          <div style={{ display: "flex", height: "100%" }}>
            {/* main column */}
            <div style={{ flex: 1, display: "flex", flexDirection: "column", padding: "26px 30px", minWidth: 0 }}>
              {/* header */}
              <div style={{ display: "flex", alignItems: "flex-end", gap: 18, marginBottom: 20 }}>
                <Wordmark progress={1} cell={11} glowStrength={0.5} />
                <div style={{ fontFamily: FONT, fontSize: 16, color: C.muted, paddingBottom: 4 }}>
                  model-mesh coding agent
                </div>
              </div>

              {/* conversation */}
              <div style={{ flex: 1, fontFamily: FONT, fontSize: 21, lineHeight: 1.7, display: "flex", flexDirection: "column", gap: 3 }}>
                {lines.map((l, i) => {
                  if (frame < l.at) return null;
                  const s = spring({ frame: frame - l.at, fps, config: { damping: 200 } });
                  return (
                    <div key={i} style={{ opacity: s, transform: `translateY(${(1 - s) * 8}px)` }}>
                      {l.render(frame)}
                    </div>
                  );
                })}
              </div>

              {/* input box */}
              <div
                style={{
                  border: `1.5px solid ${C.orange}`,
                  borderRadius: 12,
                  padding: "14px 18px",
                  fontFamily: FONT,
                  fontSize: 21,
                  color: C.text,
                  position: "relative",
                  marginTop: 12,
                }}
              >
                <span style={{ position: "absolute", top: -12, left: 16, background: C.base, padding: "0 8px", fontSize: 15, color: C.ember }}>
                  ✦ message
                </span>
                <span style={{ color: C.ember }}>› </span>
                {submitted ? (
                  <span style={{ color: C.muted }}>Message…  / commands · @ files · ? keys</span>
                ) : (
                  <Typewriter text="add pagination to the user list" frame={frame} startFrame={18} cps={26} />
                )}
              </div>
            </div>

            {/* task panel */}
            <div style={{ width: 400, borderLeft: `1px solid ${C.crust}`, padding: "26px 24px", background: C.mantle }}>
              <div style={{ fontFamily: FONT, fontSize: 15, color: C.muted, letterSpacing: 2, marginBottom: 16 }}>
                ⚒ TASKS
              </div>
              <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
                {tasks.map((t, i) => {
                  const active = frame >= t.at;
                  const done = frame >= t.at + 44;
                  const s = spring({ frame: frame - t.at, fps, config: { damping: 200 } });
                  return (
                    <div key={i} style={{ display: "flex", gap: 12, fontFamily: FONT, fontSize: 19, opacity: active ? s : 0.25, alignItems: "baseline" }}>
                      <span style={{ color: done ? C.green : C.ember, width: 20 }}>
                        {done ? "✓" : active ? "◆" : "○"}
                      </span>
                      <span style={{ color: done ? C.subtext : C.text, textDecoration: done ? "line-through" : "none" }}>
                        {t.label}
                      </span>
                    </div>
                  );
                })}
              </div>
            </div>
          </div>

          {/* statusline */}
          <div
            style={{
              position: "absolute",
              bottom: 0,
              left: 0,
              right: 0,
              height: 40,
              background: C.mantle,
              borderTop: `1px solid ${C.crust}`,
              display: "flex",
              alignItems: "center",
              padding: "0 22px",
              fontFamily: FONT,
              fontSize: 16,
              color: C.subtext,
              gap: 20,
            }}
          >
            <span style={{ color: C.green }}>◆ Auto-edit</span>
            <span style={{ color: C.ember }}>claude-cli::sonnet</span>
            <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <span style={{ color: C.muted, letterSpacing: -1 }}>
                {new Array(gaugeCells).fill(0).map((_, i) => {
                  const cellFill = Math.max(0, Math.min(8, filled - i * 8));
                  const on = cellFill > 4;
                  return (
                    <span key={i} style={{ color: on ? C.ember : C.overlay }}>
                      {on ? "▮" : "▯"}
                    </span>
                  );
                })}
              </span>
              <span style={{ color: C.muted }}>{tokens.toFixed(1)}k / 200k</span>
            </span>
            <span style={{ flex: 1 }} />
            <span style={{ color: C.muted }}>v2.4.0 · ⇧⇥ temper</span>
          </div>
        </TerminalWindow>
      </div>
    </AbsoluteFill>
  );
};
