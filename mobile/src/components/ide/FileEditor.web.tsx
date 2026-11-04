// The light editor: a transparent <textarea> over a highlighted layer that paints only the lines in
// view. The browser owns typing, selection, scrolling and undo; this file owns colour, the line
// gutter, the current-line band, Tab indent, Mod+S, and the "agent changed this file" conflict.
// Nothing here re-renders per scroll tick: the layers follow the textarea through refs, and React
// only re-renders when the visible window of lines actually moves or the text changes.
import { AlertTriangle } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { highlightTokens, type HighlightToken, type TokenKind } from "../../lib/highlightTokens";
import { useWorkspaceFile, useWriteWorkspaceFile } from "../../lib/queries";
import { useTheme } from "../../theme/ThemeProvider";
import { type ColorTokens, space } from "../../theme/tokens";
import { monoFamily, sansFamilyFor } from "../../theme/typography";
import { EmptyState } from "../ds/EmptyState";
import { Skeleton } from "../ds/Skeleton";
import { useToast } from "../ds/ToastHost";
import { languageForPath } from "../git/diffHighlight";
import { highlightLanguage } from "../workspace/fileKind";
import { EDITOR_LINE_HEIGHT as LH, classifySaveError, externalChange, indentSelection, lineOfOffset, lineWindow, scrollTopFor } from "./editorModel";
import { setFileDirty, useEditorFocusRequest, useReveal } from "./ideSignals";
import { IdeTextButton } from "./ideUi";
import { useLatest } from "./useLatest";

const MAX_BYTES = 1_000_000;
const PAD_Y = 6;
const PAD_X = 10;
const FONT_SIZE = 12;
const POLL_MS = 5000;
const CACHE_LIMIT = 3000;

const tokenCache = new Map<string, HighlightToken[]>();
function tokensFor(lang: string, text: string): HighlightToken[] {
  const key = `${lang}\0${text}`;
  const hit = tokenCache.get(key);
  if (hit) return hit;
  if (tokenCache.size > CACHE_LIMIT) tokenCache.clear();
  const tokens = text.length > 600 ? [{ kind: "plain" as TokenKind, text }] : highlightTokens(text, lang);
  tokenCache.set(key, tokens);
  return tokens;
}

function colorOf(kind: TokenKind, tokens: ColorTokens, keyword: string): string | undefined {
  switch (kind) {
    case "keyword":
      return keyword;
    case "string":
      return tokens.warn;
    case "number":
      return tokens.info;
    case "comment":
      return tokens.ink3;
    default:
      return undefined;
  }
}

export function FileEditor({ sessionId, path }: { sessionId: string; path: string }) {
  const file = useWorkspaceFile(sessionId, path);
  const { refetch } = file;
  if (file.isPending) {
    return (
      <View style={styles.skeleton}>
        {[60, 42, 74, 38, 55, 66].map((w, i) => (
          <Skeleton key={i} width={`${w}%`} height={10} />
        ))}
      </View>
    );
  }
  if (file.error || !file.data) {
    return <EmptyState icon={AlertTriangle} message={`Could not open this file. ${file.error?.message ?? ""}`.trim()} />;
  }
  if (file.data.size > MAX_BYTES) {
    return <EmptyState icon={AlertTriangle} message="This file is too large to edit here. Ask the agent, or open it in your own editor." />;
  }
  return <Editor key={`${sessionId}:${path}`} sessionId={sessionId} path={path} loaded={file.data} refetch={refetch} />;
}

function Editor({
  sessionId,
  path,
  loaded,
  refetch,
}: {
  sessionId: string;
  path: string;
  loaded: { content: string; hash: string; path: string; read_only?: boolean };
  refetch: () => Promise<{ data?: { content: string; hash: string; path: string; read_only?: boolean } }>;
}) {
  const { tokens, scheme } = useTheme();
  const toast = useToast();
  const write = useWriteWorkspaceFile();
  const lang = useMemo(() => languageForPath(path) ?? highlightLanguage(path), [path]);
  const keyword = scheme === "dark" ? tokens.ember.ember300 : tokens.ember.ember600;
  const readOnly = !!loaded.read_only;

  const [draft, setDraft] = useState(loaded.content);
  const [base, setBase] = useState(loaded.content);
  const hashRef = useRef(loaded.hash);
  const [conflict, setConflict] = useState(false);
  const [saving, setSaving] = useState(false);
  const dirty = draft !== base;

  const taRef = useRef<HTMLTextAreaElement | null>(null);
  const layerRef = useRef<HTMLDivElement | null>(null);
  const gutterRef = useRef<HTMLDivElement | null>(null);
  const [win, setWin] = useState({ first: 0, last: 60 });
  const [caretLine, setCaretLine] = useState(0);

  const lines = useMemo(() => draft.split("\n"), [draft]);
  const total = lines.length;
  const gutterWidth = Math.max(36, String(total).length * 8 + 20);

  useEffect(() => {
    setFileDirty(sessionId, path, dirty);
  }, [sessionId, path, dirty]);
  useEffect(() => () => setFileDirty(sessionId, path, false), [sessionId, path]);

  const adopt = useCallback((next: { content: string; hash: string }) => {
    setBase(next.content);
    hashRef.current = next.hash;
    setDraft(next.content);
    setConflict(false);
  }, []);

  // The agent edits files while they are open: a clean buffer follows it, a dirty one is flagged.
  useEffect(() => {
    const change = externalChange(hashRef.current, loaded.hash, draft !== base);
    if (change === "reload") adopt(loaded);
    else if (change === "conflict") setConflict(true);
  }, [loaded, draft, base, adopt]);
  useEffect(() => {
    const timer = setInterval(() => {
      if (typeof document !== "undefined" && document.visibilityState === "hidden") return;
      void refetch();
    }, POLL_MS);
    return () => clearInterval(timer);
  }, [refetch]);

  const sync = useCallback(() => {
    const ta = taRef.current;
    if (!ta) return;
    // Follow the textarea's scroll without a React render.
    if (layerRef.current) layerRef.current.style.transform = `translate(${-ta.scrollLeft + PAD_X}px, ${-ta.scrollTop + PAD_Y}px)`;
    if (gutterRef.current) gutterRef.current.style.transform = `translateY(${-ta.scrollTop + PAD_Y}px)`;
    const next = lineWindow(ta.scrollTop, ta.clientHeight, ta.value === "" ? 1 : lineCountFast(ta.value));
    setWin((current) => (current.first === next.first && current.last === next.last ? current : next));
  }, []);

  const updateCaret = useCallback(() => {
    const ta = taRef.current;
    if (!ta) return;
    const line = lineOfOffset(ta.value, ta.selectionStart);
    setCaretLine((current) => (current === line ? current : line));
  }, []);

  useEffect(() => {
    sync();
  }, [sync, total]);

  const save = useCallback(
    async (overwrite = false) => {
      if (!dirty || readOnly || saving) return;
      setSaving(true);
      try {
        let hash = hashRef.current;
        if (overwrite) {
          const fresh = await refetch();
          if (fresh.data) hash = fresh.data.hash;
        }
        const saved = await write.mutateAsync({ session: sessionId, path: loaded.path, content: draft, expected_hash: hash });
        setBase(draft);
        hashRef.current = saved.hash;
        setConflict(false);
        toast.show(`Saved ${path.slice(path.lastIndexOf("/") + 1)}`, { tone: "success" });
      } catch (error) {
        const kind = classifySaveError(error);
        if (kind === "conflict") setConflict(true);
        else toast.show(kind === "too_large" ? "The file is too large to save here." : error instanceof Error ? error.message : "Could not save.", { tone: "danger" });
      } finally {
        setSaving(false);
      }
    },
    [dirty, readOnly, saving, refetch, write, sessionId, loaded.path, draft, path, toast],
  );
  const saveRef = useLatest(save);

  const reload = useCallback(async () => {
    const fresh = await refetch();
    if (fresh.data) adopt(fresh.data);
  }, [refetch, adopt]);

  const onKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
      const ta = event.currentTarget;
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
        event.preventDefault();
        void saveRef.current();
        return;
      }
      if (event.key === "Escape") {
        ta.blur();
        return;
      }
      if (event.key === "Tab" && !event.metaKey && !event.ctrlKey && !event.altKey) {
        event.preventDefault();
        if (ta.selectionStart === ta.selectionEnd && !event.shiftKey && typeof document.execCommand === "function") {
          // Keeps the browser's undo stack intact for the common case.
          if (document.execCommand("insertText", false, "  ")) return;
        }
        const result = indentSelection(ta.value, ta.selectionStart, ta.selectionEnd, event.shiftKey);
        setDraft(result.value);
        requestAnimationFrame(() => {
          ta.selectionStart = result.start;
          ta.selectionEnd = result.end;
        });
      }
    },
    [saveRef],
  );

  // "Reveal line" from the checks panel or a timeline click, and "e" to start typing.
  const reveal = useReveal();
  useEffect(() => {
    if (!reveal || reveal.sessionId !== sessionId || reveal.path !== path) return;
    const ta = taRef.current;
    if (!ta) return;
    ta.scrollTop = scrollTopFor(reveal.line - 1, ta.clientHeight, total);
    let offset = 0;
    for (let i = 0; i < reveal.line - 1 && i < lines.length; i += 1) offset += lines[i].length + 1;
    ta.setSelectionRange(offset, offset);
    sync();
    updateCaret();
    // A reveal is consumed once per request.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reveal?.nonce]);
  const focusRequest = useEditorFocusRequest();
  useEffect(() => {
    if (focusRequest > 0) taRef.current?.focus();
  }, [focusRequest]);

  const visible = [];
  for (let index = win.first; index < Math.min(win.last, total); index += 1) {
    const text = lines[index];
    visible.push(
      <div key={index} style={{ position: "absolute", top: index * LH, left: 0, height: LH, whiteSpace: "pre" }}>
        {text.length === 0
          ? "​"
          : tokensFor(lang, text).map((token, i) => (
              <span key={i} style={{ color: colorOf(token.kind, tokens, keyword), fontStyle: token.kind === "comment" ? "italic" : undefined }}>
                {token.text}
              </span>
            ))}
      </div>,
    );
  }
  const numbers = [];
  for (let index = win.first; index < Math.min(win.last, total); index += 1) {
    numbers.push(
      <div
        key={index}
        style={{ position: "absolute", top: index * LH, right: 10, height: LH, color: index === caretLine ? tokens.ink : tokens.ink4 }}
      >
        {index + 1}
      </div>,
    );
  }

  const font = { fontFamily: `${monoFamily.regular}, ui-monospace, monospace`, fontSize: FONT_SIZE, lineHeight: `${LH}px`, tabSize: 2 } as const;

  return (
    <View style={styles.root}>
      {conflict ? (
        <View style={[styles.banner, { backgroundColor: tokens.warnBg, borderBottomColor: tokens.border }]}>
          <AlertTriangle size={13} strokeWidth={1.75} color={tokens.warnBgInk} />
          <Text style={[styles.bannerText, { color: tokens.warnBgInk }]}>The agent changed this file after you opened it.</Text>
          <View style={styles.grow} />
          <IdeTextButton label="Reload (lose my edits)" onPress={() => void reload()} />
          <IdeTextButton label="Overwrite" tone="danger" onPress={() => void save(true)} loading={saving} />
        </View>
      ) : readOnly ? (
        <View style={[styles.banner, { borderBottomColor: tokens.border }]}>
          <Text style={[styles.bannerText, { color: tokens.ink3 }]}>Read only. This file is outside the workspace.</Text>
        </View>
      ) : (
        <View style={[styles.status, { borderBottomColor: tokens.hairline }]}>
          <Text style={[styles.statusText, { color: dirty ? tokens.ink : tokens.ink4 }]}>{dirty ? "Unsaved changes" : "Click in and type to edit. Anything bigger, ask the agent."}</Text>
          <View style={styles.grow} />
          {dirty ? <IdeTextButton label="Save" keyHint="⌘S" onPress={() => void save()} loading={saving} /> : null}
        </View>
      )}
      <div
        data-focus-zone="editor"
        style={{ position: "relative", flex: 1, minHeight: 0, display: "flex", overflow: "hidden", background: tokens.bg0, color: tokens.ink, ...font }}
      >
        <div style={{ position: "relative", width: gutterWidth, flexShrink: 0, overflow: "hidden", borderRight: `1px solid ${tokens.hairline}`, textAlign: "right", userSelect: "none", fontSize: 10.5 }}>
          <div ref={gutterRef} style={{ position: "absolute", top: 0, left: 0, right: 0, height: total * LH, willChange: "transform" }}>
            {numbers}
          </div>
        </div>
        <div style={{ position: "relative", flex: 1, minWidth: 0 }}>
          <div style={{ position: "absolute", inset: 0, overflow: "hidden", pointerEvents: "none" }}>
            <div ref={layerRef} style={{ position: "absolute", top: 0, left: 0, minWidth: "100%", height: total * LH, willChange: "transform" }}>
              <div style={{ position: "absolute", top: caretLine * LH, left: -PAD_X, right: 0, width: "200vw", height: LH, background: tokens.bg2 }} />
              {visible}
            </div>
          </div>
          <textarea
            ref={taRef}
            value={draft}
            readOnly={readOnly}
            spellCheck={false}
            autoCapitalize="off"
            autoCorrect="off"
            wrap="off"
            aria-label={`Edit ${path}`}
            onChange={(event) => setDraft(event.target.value)}
            onScroll={sync}
            onKeyDown={onKeyDown}
            onKeyUp={updateCaret}
            onClick={updateCaret}
            onSelect={updateCaret}
            onFocus={updateCaret}
            style={{
              position: "absolute",
              inset: 0,
              width: "100%",
              height: "100%",
              margin: 0,
              padding: `${PAD_Y}px ${PAD_X}px`,
              border: 0,
              outline: "none",
              resize: "none",
              overflow: "auto",
              whiteSpace: "pre",
              background: "transparent",
              color: "transparent",
              caretColor: tokens.ink,
              boxSizing: "border-box",
              ...font,
            }}
          />
        </div>
      </div>
    </View>
  );
}

function lineCountFast(value: string): number {
  let count = 1;
  for (let i = value.indexOf("\n"); i !== -1; i = value.indexOf("\n", i + 1)) count += 1;
  return count;
}


const styles = StyleSheet.create({
  root: { flex: 1, minHeight: 0 },
  skeleton: { padding: space.space16, gap: space.space12 },
  banner: { minHeight: 32, flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space12, paddingVertical: 4, borderBottomWidth: StyleSheet.hairlineWidth },
  bannerText: { fontFamily: sansFamilyFor(500), fontSize: 11.5 },
  status: { minHeight: 28, flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space12, borderBottomWidth: StyleSheet.hairlineWidth },
  statusText: { fontFamily: sansFamilyFor(400), fontSize: 11 },
  grow: { flex: 1 },
});
