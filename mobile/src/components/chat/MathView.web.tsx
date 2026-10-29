// Typeset TeX in the browser and the desktop webview. KaTeX loads on first use as its own chunk, so
// a chat without math never pays for it, and renders MathML: the engine's own math layout, no
// stylesheet or fonts to ship, and it inherits the surrounding colour and size.
import React, { useEffect, useState } from "react";
import { Text } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { monoFamily, type } from "../../theme/typography";

interface Katex {
  renderToString(tex: string, options: Record<string, unknown>): string;
}

let loaded: Katex | null = null;
let loading: Promise<Katex> | null = null;

function loadKatex(): Promise<Katex> {
  loading ??= import("katex").then((module) => {
    loaded = (module.default ?? module) as unknown as Katex;
    return loaded;
  });
  return loading;
}

function typeset(katex: Katex, tex: string, display: boolean): string | null {
  try {
    return katex.renderToString(tex, {
      displayMode: display,
      output: "mathml",
      throwOnError: false,
      strict: "ignore",
      trust: false,
      maxExpand: 500,
    });
  } catch {
    return null;
  }
}

export function MathView({ tex, display }: { tex: string; display: boolean }) {
  const tokens = useTokens();
  const [html, setHtml] = useState<string | null>(() => (loaded ? typeset(loaded, tex, display) : null));

  useEffect(() => {
    let live = true;
    if (loaded) {
      setHtml(typeset(loaded, tex, display));
      return;
    }
    loadKatex()
      .then((katex) => live && setHtml(typeset(katex, tex, display)))
      .catch(() => live && setHtml(null));
    return () => {
      live = false;
    };
  }, [tex, display]);

  if (html == null) {
    return (
      <Text selectable style={[type.codeSmall, { color: tokens.ink3, fontFamily: monoFamily.regular }]}>
        {display ? tex : `$${tex}$`}
      </Text>
    );
  }
  return React.createElement(display ? "div" : "span", {
    role: "math",
    "aria-label": tex,
    dangerouslySetInnerHTML: { __html: html },
    style: display
      ? { display: "block", overflowX: "auto", overflowY: "hidden", padding: "6px 0", margin: "4px 0", color: tokens.ink }
      : { color: tokens.ink },
  });
}
