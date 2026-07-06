// Root HTML document for the web static export (expo-router convention:
// https://docs.expo.dev/router/reference/static-rendering/#root-html). Only
// rendered on `npx expo export -p web` / `expo start --web` — links the PWA
// manifest + favicon so the exported site is installable (ARCHITECTURE.md §5
// PWA + Web Push parity, BUILD_ORDER.md T4.3). `public/sw.js` registers
// itself from `src/lib/push/push.web.ts`, not from a `<script>` tag here.
import { ScrollViewStyleReset } from "expo-router/html";
import React from "react";

import { darkTokens } from "../theme/tokens";

// DESIGN_SYSTEM.md §1.2 dark `bg1` — matches `public/manifest.webmanifest`'s
// `theme_color` deliberately: the browser chrome/PWA titlebar reads as the
// app's surface. tokens.ts is the only file allowed a raw hex literal
// (BUILD_ORDER.md's no-raw-hex gate), so this imports rather than repeats it.
const THEME_COLOR = darkTokens.bg1;

export default function Root({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <head>
        <meta charSet="utf-8" />
        <meta httpEquiv="X-UA-Compatible" content="IE=edge" />
        <meta name="viewport" content="width=device-width, initial-scale=1, shrink-to-fit=no, viewport-fit=cover" />
        <meta name="theme-color" content={THEME_COLOR} />
        <meta name="mobile-web-app-capable" content="yes" />
        <link rel="manifest" href="/manifest.webmanifest" />
        <link rel="icon" href="/favicon.png" />
        <link rel="apple-touch-icon" href="/icon-192.png" />
        <ScrollViewStyleReset />
      </head>
      <body>{children}</body>
    </html>
  );
}
