import React from "react";
import { Composition } from "remotion";
import { Promo, PROMO_DURATION } from "./Promo";
import { Teaser, TEASER_DURATION } from "./Teaser";
import { Vertical, VERTICAL_DURATION } from "./Vertical";

export const RemotionRoot: React.FC = () => {
  return (
    <>
      <Composition
        id="Promo"
        component={Promo}
        durationInFrames={PROMO_DURATION}
        fps={30}
        width={1920}
        height={1080}
      />
      <Composition
        id="Teaser"
        component={Teaser}
        durationInFrames={TEASER_DURATION}
        fps={30}
        width={1920}
        height={1080}
      />
      <Composition
        id="Vertical"
        component={Vertical}
        durationInFrames={VERTICAL_DURATION}
        fps={30}
        width={1080}
        height={1920}
      />
    </>
  );
};
