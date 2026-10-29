// Registers the terminal as a Find target: the retained scrollback is searched, every match is
// highlighted, and stepping scrolls the active one into view. Closing the bar hands the keyboard
// back to the shell.
import { type MutableRefObject, useEffect, useMemo, useRef, useState } from "react";
import { type ScrollView, type Text, type TextInput } from "react-native";

import { findController, useFindState } from "../../lib/find/findController";
import { steppedIndex } from "../../lib/find/matching";
import { type AnsiScrollback } from "./ansi";
import {
  findTerminalHits,
  highlightsByLine,
  type LineHighlight,
  relocateHit,
  type TerminalHit,
} from "./terminalFind";

interface FindView {
  hits: TerminalHit[];
  active: number;
}

// The Find bar floats over the top of the dock; matches are kept clear of it.
const BAR_INSET = 52;

const NO_VIEW: FindView = { hits: [], active: -1 };

export interface TerminalFindOptions {
  buffer: MutableRefObject<AnsiScrollback>;
  scroll: MutableRefObject<ScrollView | null>;
  input: MutableRefObject<TextInput | null>;
  /** Cleared while a match is on screen so new output does not drag the view back to the end. */
  sticky: MutableRefObject<boolean>;
  lineHeight: number;
}

export function useTerminalFind({ buffer, scroll, input, sticky, lineHeight }: TerminalFindOptions): {
  highlights: ReadonlyMap<number, LineHighlight[]>;
  activeKey: number | null;
  activeLineRef: MutableRefObject<Text | null>;
} {
  const find = useFindState();
  const [view, setView] = useState<FindView>(NO_VIEW);
  const activeLineRef = useRef<Text | null>(null);

  useEffect(() => {
    let hits: TerminalHit[] = [];
    let active = -1;
    let capped = false;
    let query = "";
    let caseSensitive = false;
    const publish = () => {
      setView({ hits, active });
      return { count: hits.length, index: active, capped };
    };
    const rescan = () => {
      ({ hits, capped } = findTerminalHits(buffer.current.lines(), query, caseSensitive));
    };
    return findController.register({
      id: "terminal",
      label: "terminal",
      search(nextQuery, nextCase) {
        query = nextQuery;
        caseSensitive = nextCase;
        rescan();
        active = hits.length - 1;
        return publish();
      },
      step(direction) {
        const previous = hits[active];
        rescan();
        active = steppedIndex(relocateHit(hits, previous), hits.length, direction);
        return publish();
      },
      clear() {
        hits = [];
        active = -1;
        setView(NO_VIEW);
      },
    });
  }, [buffer]);

  const highlights = useMemo(() => highlightsByLine(view.hits, view.active), [view]);
  const activeKey = view.hits[view.active]?.key ?? null;

  useEffect(() => {
    const line = activeLineRef.current;
    if (activeKey == null || !line) return;
    sticky.current = false;
    const pane = scroll.current;
    if (!pane) return;
    const dom = (pane as unknown as { getScrollableNode?: () => HTMLElement }).getScrollableNode?.();
    const element = line as unknown as HTMLElement;
    if (dom && typeof element.getBoundingClientRect === "function") {
      const box = element.getBoundingClientRect();
      const frame = dom.getBoundingClientRect();
      if (box.top >= frame.top + BAR_INSET && box.bottom <= frame.bottom - lineHeight) return;
      dom.scrollTop += box.top - frame.top - BAR_INSET - (frame.height - BAR_INSET - box.height) / 2;
      return;
    }
    line.measureLayout(
      pane as never,
      (_x, y) => pane.scrollTo({ y: Math.max(0, y - BAR_INSET - lineHeight), animated: false }),
      () => undefined,
    );
  }, [view, activeKey, scroll, sticky, lineHeight]);

  const wasOpen = useRef(false);
  const open = find.open && find.targetId === "terminal";
  useEffect(() => {
    if (wasOpen.current && !open && !find.open) input.current?.focus();
    wasOpen.current = open;
  }, [open, find.open, input]);

  return { highlights, activeKey, activeLineRef };
}
