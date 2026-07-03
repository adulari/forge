import React from "react";
import { AbsoluteFill, Sequence } from "remotion";
import { loadFonts } from "./fonts";
import { Backdrop } from "./components/common";
import { ColdOpen } from "./scenes/ColdOpen";
import { MeshRouting } from "./scenes/MeshRouting";
import { Duel } from "./scenes/FeatureRun";
import { Close } from "./scenes/Close";
import timeline from "./timeline.json";

loadFonts();

const S = timeline.teaser.scenes;
const XF = timeline.crossfade;

export const TEASER_DURATION = S.close[0] + S.close[1]; // 950

export const Teaser: React.FC = () => {
  return (
    <AbsoluteFill>
      <Backdrop />
      <Sequence from={S.cold[0]} durationInFrames={S.cold[1]}><ColdOpen dur={S.cold[1]} /></Sequence>
      <Sequence from={S.mesh[0]} durationInFrames={S.mesh[1]}><MeshRouting dur={S.mesh[1]} /></Sequence>
      {/* duel lingers XF frames so Close (fading in on top) crossfades over it
          instead of hard-cutting to the backdrop */}
      <Sequence from={S.duel[0]} durationInFrames={S.duel[1] + XF}><Duel /></Sequence>
      <Sequence from={S.close[0]} durationInFrames={S.close[1]}><Close dur={S.close[1]} fadeIn={12} /></Sequence>
    </AbsoluteFill>
  );
};
