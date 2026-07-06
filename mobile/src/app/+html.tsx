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

// Native-feel CSS for the PWA/web target: no rubber-band chaining to the page, no grey
// tap flash / long-press callout on touch devices, no Safari font-boosting, faster taps.
// Deliberately does NOT set a global `user-select: none` — chat/code content stays
// selectable; per-component `overscroll-behavior: contain` on scroll surfaces (Screen,
// BoundedList) is what actually stops rubber-band, `overscroll-behavior: none` here is
// the page-level backstop.
const GLOBAL_WEB_CSS = `
html, body, #root {
  height: 100%;
  height: 100vh;
  height: 100dvh;
  margin: 0;
}
body { overscroll-behavior: none; -webkit-overflow-scrolling: touch; touch-action: manipulation; }
* { -webkit-tap-highlight-color: transparent; -webkit-touch-callout: none; }
html { -webkit-text-size-adjust: 100%; text-size-adjust: 100%; }
`;

export default function Root({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <head>
        <meta charSet="utf-8" />
        <meta httpEquiv="X-UA-Compatible" content="IE=edge" />
        {/* Kills pinch/double-tap zoom + page mis-scaling; viewport-fit=cover enables
            safe-area insets; interactive-widget=resizes-content keeps the layout (not
            just the visual viewport) resizing when the iOS keyboard opens. */}
        <meta
          name="viewport"
          content="width=device-width, initial-scale=1, maximum-scale=1, user-scalable=no, viewport-fit=cover, interactive-widget=resizes-content"
        />
        <meta name="theme-color" content={THEME_COLOR} />
        {/* Apple PWA / "Add to Home Screen" chromeless launch. */}
        <meta name="apple-mobile-web-app-capable" content="yes" />
        <meta name="apple-mobile-web-app-status-bar-style" content="black-translucent" />
        <meta name="apple-mobile-web-app-title" content="Forge" />
        <meta name="mobile-web-app-capable" content="yes" />
        <link rel="manifest" href="/manifest.webmanifest" />
        <link rel="icon" href="/favicon.png" />
        <link rel="apple-touch-icon" href="/icon-192.png" />
        <style dangerouslySetInnerHTML={{ __html: GLOBAL_WEB_CSS }} />
        <ScrollViewStyleReset />
      </head>
      <body>{children}</body>
    </html>
  );
}
