import React from "react";
import { AbsoluteFill, interpolate, spring, useCurrentFrame, useVideoConfig } from "remotion";
import { C, FONT } from "../theme";
import { Backdrop, Embers, Typewriter } from "../components/common";
import { Wordmark } from "../components/Wordmark";

export const Close: React.FC<{ dur: number }> = ({ dur }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const cardS = spring({ frame: frame - 4, fps, config: { damping: 16 } });
  const installDone = frame >= 70;
  const chatStart = 96;

  const igniteStart = 164;
  const ignite = interpolate(frame, [igniteStart, igniteStart + 36], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: (t) => 1 - Math.pow(1 - t, 2) });
  const markS = spring({ frame: frame - igniteStart, fps, config: { damping: 14 } });
  const slugS = spring({ frame: frame - 194, fps, config: { damping: 200 } });

  const outFade = interpolate(frame, [dur - 24, dur], [1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  // command card fully clears exactly as the wordmark ignites (no overlap, no gap)
  const cardOut = interpolate(frame, [148, 164], [1, 0], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });

  return (
    <AbsoluteFill style={{ opacity: outFade, justifyContent: "center", alignItems: "center" }}>
      <Backdrop tint={C.orange} />
      <Embers count={20} opacity={0.7 + ignite * 0.6} />

      {frame < 164 ? (
        <div
          style={{
            position: "absolute",
            opacity: cardS * cardOut,
            transform: `scale(${0.96 + cardS * 0.04})`,
            background: C.mantle,
            border: `1px solid ${C.surface}`,
            borderRadius: 14,
            padding: "30px 40px",
            fontFamily: FONT,
            fontSize: 27,
            boxShadow: "0 30px 80px rgba(0,0,0,0.5)",
            minWidth: 980,
          }}
        >
          <div style={{ color: C.text }}>
            <span style={{ color: C.green }}>$ </span>
            <Typewriter
              text="curl -fsSL https://raw.githubusercontent.com/Adulari/forge/main/install.sh | sh"
              frame={frame}
              startFrame={8}
              cps={42}
              caret={!installDone}
            />
          </div>
          {installDone ? (
            <div style={{ color: C.green, marginTop: 12, fontSize: 23 }}>✓ installed forge → ~/.local/bin</div>
          ) : null}
          {frame >= chatStart ? (
            <div style={{ color: C.text, marginTop: 12 }}>
              <span style={{ color: C.green }}>$ </span>
              <Typewriter text="forge chat" frame={frame} startFrame={chatStart} cps={16} caret={frame < 150} />
            </div>
          ) : null}
        </div>
      ) : null}

      {/* wordmark hero */}
      {ignite > 0 ? (
        <div style={{ opacity: ignite, transform: `scale(${0.9 + markS * 0.1})`, display: "flex", flexDirection: "column", alignItems: "center", gap: 34 }}>
          <Wordmark progress={ignite} cell={30} glowStrength={1.2} />
          <div style={{ fontFamily: FONT, fontSize: 30, color: C.subtext, opacity: slugS, display: "flex", alignItems: "center", gap: 12 }}>
            <span style={{ color: C.ember }}>⚒</span> github.com/Adulari/forge
          </div>
          <div style={{ fontFamily: FONT, fontSize: 22, color: C.muted, opacity: slugS }}>
            one binary · every model · your terminal
          </div>
        </div>
      ) : null}
    </AbsoluteFill>
  );
};
