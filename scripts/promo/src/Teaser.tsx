import React from "react";
import { AbsoluteFill, Sequence } from "remotion";
import { loadFonts } from "./fonts";
import { Backdrop } from "./components/common";
import { ColdOpen } from "./scenes/ColdOpen";
import { MeshRouting } from "./scenes/MeshRouting";
import { Duel } from "./scenes/FeatureRun";
import { Close } from "./scenes/Close";

loadFonts();

const S = {
  cold: [0, 120],
  mesh: [120, 400],
  duel: [520, 120],
  close: [640, 260],
} as const;

export const TEASER_DURATION = S.close[0] + S.close[1]; // 900

export const Teaser: React.FC = () => {
  return (
    <AbsoluteFill>
      <Backdrop />
      <Sequence from={S.cold[0]} durationInFrames={S.cold[1]}><ColdOpen dur={S.cold[1]} /></Sequence>
      <Sequence from={S.mesh[0]} durationInFrames={S.mesh[1]}><MeshRouting dur={S.mesh[1]} /></Sequence>
      <Sequence from={S.duel[0]} durationInFrames={S.duel[1]}><Duel /></Sequence>
      <Sequence from={S.close[0]} durationInFrames={S.close[1]}><Close dur={S.close[1]} /></Sequence>
    </AbsoluteFill>
  );
};
