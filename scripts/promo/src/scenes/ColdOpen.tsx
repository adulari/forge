import React from "react";
import { AbsoluteFill, interpolate, spring, useCurrentFrame, useVideoConfig } from "remotion";
import { C, FONT, glow } from "../theme";
import { Backdrop, Embers, Typewriter, edgeFade } from "../components/common";
import { Wordmark } from "../components/Wordmark";

export const ColdOpen: React.FC<{ dur: number }> = ({ dur }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  // single ember spark rises from below and flares at center
  const sparkY = interpolate(frame, [0, 30], [420, 0], {
    extrapolateRight: "clamp",
    easing: (t) => 1 - Math.pow(1 - t, 3),
  });
  const sparkFlare = interpolate(frame, [26, 38, 52], [1, 3.4, 0], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  const sparkOpacity = interpolate(frame, [0, 6, 40, 52], [0, 1, 1, 0], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });

  // wordmark ignition
  const ignite = interpolate(frame, [30, 74], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
    easing: (t) => 1 - Math.pow(1 - t, 2),
  });
  const markScale = spring({ frame: frame - 30, fps, config: { damping: 14, mass: 0.6 } });
  const markPop = 0.9 + markScale * 0.1;

  const bgReveal = interpolate(frame, [30, 60], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });

  return (
    <AbsoluteFill style={{ opacity: edgeFade(frame, dur, 6, 18) }}>
      <AbsoluteFill style={{ opacity: bgReveal }}>
        <Backdrop tint={C.orange} />
        <Embers count={18} opacity={ignite} />
      </AbsoluteFill>
      <AbsoluteFill style={{ background: "#000", opacity: 1 - bgReveal }} />

      <AbsoluteFill
        style={{
          justifyContent: "center",
          alignItems: "center",
          flexDirection: "column",
          gap: 44,
        }}
      >
        <div style={{ position: "relative", transform: `scale(${markPop})` }}>
          {/* spark */}
          <div
            style={{
              position: "absolute",
              left: "50%",
              top: "50%",
              width: 10,
              height: 10,
              marginLeft: -5,
              marginTop: sparkY - 5,
              borderRadius: "50%",
              background: C.yellow,
              transform: `scale(${sparkFlare})`,
              opacity: sparkOpacity,
              boxShadow: glow(C.ember, 2.4),
            }}
          />
          <div style={{ opacity: ignite > 0 ? 1 : 0 }}>
            <Wordmark progress={ignite} cell={34} glowStrength={1.2} />
          </div>
        </div>

        <div
          style={{
            fontFamily: FONT,
            fontSize: 34,
            fontWeight: 500,
            color: C.subtext,
            letterSpacing: 1,
            minHeight: 42,
          }}
        >
          <Typewriter
            text="One binary.  Every model.  Your terminal."
            frame={frame}
            startFrame={66}
            cps={30}
          />
        </div>
      </AbsoluteFill>
    </AbsoluteFill>
  );
};
