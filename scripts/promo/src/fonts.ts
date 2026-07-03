import { staticFile, continueRender, delayRender } from "remotion";

// Bundled JetBrains Mono — loaded from public/fonts (no CDN).
const faces = [
  { weight: 400, file: "JetBrainsMono-Regular.woff2" },
  { weight: 500, file: "JetBrainsMono-Medium.woff2" },
  { weight: 700, file: "JetBrainsMono-Bold.woff2" },
  { weight: 800, file: "JetBrainsMono-ExtraBold.woff2" },
];

let loaded = false;

export const loadFonts = () => {
  if (loaded || typeof document === "undefined") return;
  loaded = true;
  const handle = delayRender("Loading JetBrains Mono");
  Promise.all(
    faces.map(async ({ weight, file }) => {
      const font = new FontFace(
        "JetBrains Mono",
        `url(${staticFile("fonts/" + file)}) format("woff2")`,
        { weight: String(weight), style: "normal" },
      );
      await font.load();
      (document.fonts as FontFaceSet).add(font);
    }),
  )
    .then(() => continueRender(handle))
    .catch(() => continueRender(handle));
};
