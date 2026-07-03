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

import timeline from "./timeline.json";

// [from, dur] — single source of truth shared with the audio generator
// (scripts/promo/audio/make_audio.py) so sound accents land exactly on cuts.
const S = timeline.promo.scenes;

export const PROMO_DURATION = S.close[0] + S.close[1]; // 2340

// Scenes bordering the FeatureRun montage linger XF frames into the next scene
// (which fades in on top) so the proof->feat and feat->remote cuts crossfade
// instead of dipping to a blank backdrop frame.
const XF = timeline.crossfade;

export const Promo: React.FC = () => {
  return (
    <AbsoluteFill>
      <Backdrop />
      <Sequence from={S.cold[0]} durationInFrames={S.cold[1]}><ColdOpen dur={S.cold[1]} /></Sequence>
      <Sequence from={S.mesh[0]} durationInFrames={S.mesh[1]}><MeshRouting dur={S.mesh[1]} /></Sequence>
      <Sequence from={S.tui[0]} durationInFrames={S.tui[1]}><TuiScene dur={S.tui[1]} /></Sequence>
      <Sequence from={S.proof[0]} durationInFrames={S.proof[1] + XF}><Proof dur={S.proof[1]} /></Sequence>
      <Sequence from={S.feat[0]} durationInFrames={S.feat[1] + XF}><FeatureRun dur={S.feat[1]} /></Sequence>
      <Sequence from={S.remote[0]} durationInFrames={S.remote[1]}><RemoteControl dur={S.remote[1]} /></Sequence>
      <Sequence from={S.close[0]} durationInFrames={S.close[1]}><Close dur={S.close[1]} /></Sequence>
    </AbsoluteFill>
  );
};
