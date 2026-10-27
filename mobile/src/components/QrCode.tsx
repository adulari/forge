import React, { useMemo } from "react";
import Svg, { Path, Rect } from "react-native-svg";

import { qrMatrix, qrSvgPath } from "../lib/qr";
import { lightTokens } from "../theme/tokens";

const QUIET_ZONE = 4;

export interface QrCodeProps {
  value: string;
  size?: number;
  accessibilityLabel?: string;
}

/** Always dark-on-light, in both themes: scanners need the contrast and the quiet zone. */
export function QrCode({ value, size = 168, accessibilityLabel = "QR code" }: QrCodeProps) {
  const { path, modules } = useMemo(() => {
    const matrix = qrMatrix(value);
    return { path: qrSvgPath(matrix, QUIET_ZONE), modules: matrix.length + QUIET_ZONE * 2 };
  }, [value]);
  return (
    <Svg
      width={size}
      height={size}
      viewBox={`0 0 ${modules} ${modules}`}
      accessibilityRole="image"
      accessibilityLabel={accessibilityLabel}
    >
      <Rect width={modules} height={modules} fill={lightTokens.bg0} />
      <Path d={path} fill={lightTokens.ink} />
    </Svg>
  );
}
