import { useEffect, useState } from "react";
import { Image } from "react-native";

/** Width over height of the image at `uri`, once known. React Native Web's onLoad does not report
 * a size, so the thumbnail frame measures it this way. */
export function useImageRatio(uri: string, fallback: number): number {
  const [ratio, setRatio] = useState(fallback);
  useEffect(() => {
    let live = true;
    Image.getSize(
      uri,
      (width, height) => {
        if (live && height > 0) setRatio(width / height);
      },
      () => {},
    );
    return () => {
      live = false;
    };
  }, [uri]);
  return ratio;
}
