import React from "react";
import { AbsoluteFill, Sequence } from "remotion";
import { loadFonts } from "./fonts";
import { Backdrop } from "./components/common";
import { ColdOpen } from "./scenes/ColdOpen";
import { MeshRouting } from "./scenes/MeshRouting";
import { TuiScene } from "./scenes/TuiScene";
import { Proof } from "./scenes/Proof";
import { FeatureRun } from "./scenes/FeatureRun";
import { RemoteControl } from "./scenes/RemoteControl";
import { Close } from "./scenes/Close";

loadFonts();

// [from, dur]
const S = {
  cold: [0, 150],
  mesh: [150, 450],
  tui: [600, 360],
  proof: [960, 240],
  feat: [1200, 600],
  remote: [1800, 240],
  close: [2040, 240],
} as const;

export const PROMO_DURATION = S.close[0] + S.close[1]; // 2250

export const Promo: React.FC = () => {
  return (
    <AbsoluteFill>
      <Backdrop />
      <Sequence from={S.cold[0]} durationInFrames={S.cold[1]}><ColdOpen dur={S.cold[1]} /></Sequence>
      <Sequence from={S.mesh[0]} durationInFrames={S.mesh[1]}><MeshRouting dur={S.mesh[1]} /></Sequence>
      <Sequence from={S.tui[0]} durationInFrames={S.tui[1]}><TuiScene dur={S.tui[1]} /></Sequence>
      <Sequence from={S.proof[0]} durationInFrames={S.proof[1]}><Proof dur={S.proof[1]} /></Sequence>
      <Sequence from={S.feat[0]} durationInFrames={S.feat[1]}><FeatureRun dur={S.feat[1]} /></Sequence>
      <Sequence from={S.remote[0]} durationInFrames={S.remote[1]}><RemoteControl dur={S.remote[1]} /></Sequence>
      <Sequence from={S.close[0]} durationInFrames={S.close[1]}><Close dur={S.close[1]} /></Sequence>
    </AbsoluteFill>
  );
};
