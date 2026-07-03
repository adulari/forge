import React from "react";
import { AbsoluteFill, interpolate, spring, useCurrentFrame, useVideoConfig, random } from "remotion";
import { C, FONT, glow } from "../theme";
import { Backdrop, edgeFade } from "../components/common";
import { TerminalWindow } from "../components/TerminalWindow";

const QR: React.FC<{ size: number }> = ({ size }) => {
  const n = 21;
  const cell = size / n;
  const isFinder = (r: number, c: number) => {
    const inBox = (br: number, bc: number) => r >= br && r < br + 7 && c >= bc && c < bc + 7;
    return inBox(0, 0) || inBox(0, n - 7) || inBox(n - 7, 0);
  };
  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} shapeRendering="crispEdges">
      <rect width={size} height={size} fill={C.text} rx={6} />
      {new Array(n).fill(0).map((_, r) =>
        new Array(n).fill(0).map((_, c) => {
          const finder = isFinder(r, c);
          const on = finder ? (r % 6 < 4 && c % 6 < 4 ? false : true) : random(`qr${r}-${c}`) > 0.52;
          // finder squares: draw ring pattern
          const ring = finder && ((r % 7 === 0 || r % 7 === 6 || c % 7 === 0 || c % 7 === 6) || (r % 7 >= 2 && r % 7 <= 4 && c % 7 >= 2 && c % 7 <= 4));
          const draw = finder ? ring : on;
          if (!draw) return null;
          return <rect key={`${r}-${c}`} x={c * cell} y={r * cell} width={cell} height={cell} fill={C.crust} />;
        }),
      )}
    </svg>
  );
};

export const RemoteControl: React.FC<{ dur: number }> = ({ dur }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const termS = spring({ frame, fps, config: { damping: 15 } });
  const phoneS = spring({ frame: frame - 8, fps, config: { damping: 15 } });

  const phoneLit = frame >= 70;
  const tap = frame >= 128 && frame < 150;
  const tapRipple = spring({ frame: frame - 128, fps, config: { damping: 12 } });
  const approved = frame >= 150;
  const capS = spring({ frame: frame - 96, fps, config: { damping: 200 } });

  // scan beam terminal -> phone
  const beam = interpolate(frame, [34, 70], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });

  return (
    <AbsoluteFill style={{ opacity: edgeFade(frame, dur), justifyContent: "center", alignItems: "center" }}>
      <Backdrop tint={C.lavender} />

      <div style={{ display: "flex", alignItems: "center", gap: 120 }}>
        {/* terminal */}
        <div style={{ transform: `scale(${0.94 + termS * 0.06})`, opacity: termS }}>
          <TerminalWindow width={820} height={520} title="forge chat">
            <div style={{ padding: 34, height: "100%", display: "flex", flexDirection: "column", fontFamily: FONT }}>
              <div style={{ fontSize: 20, color: C.subtext, marginBottom: 8 }}>
                <span style={{ color: C.ember }}>↳</span> pair a device to control this session
              </div>
              <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center", gap: 30 }}>
                <div style={{ padding: 14, background: C.base, border: `1px solid ${C.surface}`, borderRadius: 12 }}>
                  <QR size={200} />
                </div>
              </div>
              <div style={{ fontSize: 20 }}>
                {approved ? (
                  <span style={{ color: C.green }}>✓ approved on phone → <span style={{ color: C.yellow }}>● shell</span> cargo test <span style={{ color: C.muted }}>running…</span></span>
                ) : (
                  <span style={{ color: C.muted }}>waiting for approval…</span>
                )}
              </div>
            </div>
          </TerminalWindow>
        </div>

        {/* scan beam */}
        <div style={{ position: "absolute", left: "50%", top: "50%", width: 120 * beam, height: 3, background: `linear-gradient(90deg, ${C.ember}, transparent)`, transform: "translateY(-40px)", opacity: 1 - beam }} />

        {/* phone */}
        <div style={{ transform: `scale(${0.9 + phoneS * 0.1})`, opacity: phoneS }}>
          <div style={{ width: 320, height: 640, background: C.crust, border: `8px solid ${C.surface}`, borderRadius: 44, padding: 14, boxShadow: "0 40px 100px rgba(0,0,0,0.6)", position: "relative" }}>
            <div style={{ position: "absolute", top: 22, left: "50%", transform: "translateX(-50%)", width: 110, height: 26, background: C.crust, borderRadius: 14, zIndex: 3 }} />
            <div style={{ width: "100%", height: "100%", background: C.mantle, borderRadius: 32, overflow: "hidden", opacity: phoneLit ? 1 : 0.25, display: "flex", flexDirection: "column" }}>
              <div style={{ padding: "44px 20px 14px", fontFamily: FONT, fontSize: 17, color: C.ember, fontWeight: 700 }}>⚒ forge · live</div>
              <div style={{ padding: "0 20px", fontFamily: FONT, fontSize: 15, color: C.subtext, lineHeight: 1.6, flex: 1 }}>
                <div><span style={{ color: C.ember }}>↳</span> claude-cli::sonnet</div>
                <div style={{ color: C.text, marginTop: 8 }}>● edit routes.rs <span style={{ color: C.green }}>+18</span></div>
                <div style={{ marginTop: 18, background: C.surface, borderRadius: 12, padding: 14, border: `1px solid ${C.yellow}66`, opacity: phoneLit ? interpolate(frame, [78, 96], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }) : 0 }}>
                  <div style={{ color: C.yellow, fontWeight: 700 }}>Allow shell command?</div>
                  <div style={{ color: C.muted, marginTop: 4 }}>cargo test</div>
                </div>
              </div>
              {/* allow button */}
              <div style={{ padding: 20, position: "relative" }}>
                <div style={{ position: "relative", background: approved ? C.green : C.ember, color: C.base, fontFamily: FONT, fontWeight: 800, fontSize: 20, textAlign: "center", padding: "14px 0", borderRadius: 14, boxShadow: tap ? glow(C.ember, 0.8) : "none", transform: `scale(${tap ? 0.96 : 1})`, opacity: phoneLit ? interpolate(frame, [90, 104], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }) : 0 }}>
                  {approved ? "✓ Allowed" : "Allow"}
                  {tap ? (
                    <div style={{ position: "absolute", left: "50%", top: "50%", width: 200 * tapRipple, height: 200 * tapRipple, marginLeft: -100 * tapRipple, marginTop: -100 * tapRipple, borderRadius: "50%", border: `2px solid ${C.base}`, opacity: 1 - tapRipple }} />
                  ) : null}
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>

      <div style={{ position: "absolute", bottom: 84, left: 0, right: 0, textAlign: "center", opacity: capS }}>
        <span style={{ fontFamily: FONT, fontWeight: 800, fontSize: 46, color: C.text }}>
          Full control — <span style={{ color: C.lavender }}>from your phone</span>.
        </span>
      </div>
    </AbsoluteFill>
  );
};
