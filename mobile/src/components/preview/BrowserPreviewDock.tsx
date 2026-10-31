import {
  ArrowLeft,
  ArrowRight,
  ExternalLink,
  Minus,
  MousePointer2,
  Plus,
  RefreshCw,
  ScrollText,
  SquareTerminal,
} from "lucide-react-native";
import React, { useCallback, useEffect, useRef, useState } from "react";
import {
  ActivityIndicator,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
  type LayoutChangeEvent,
} from "react-native";

import {
  getPreviewPreferences,
  hideBrowserPreview,
  listenBrowserPreview,
  moveBrowserPreviewHistory,
  navigateBrowserPreview,
  normalizePreviewUrl,
  openBrowserPreview,
  previewLabel,
  previewViewportBounds,
  reloadBrowserPreview,
  setBrowserPreviewPicker,
  setPreviewPreferences,
  zoomBrowserPreview,
  type PreviewBounds,
  type PreviewViewport,
} from "../../lib/browserPreview";
import { useAuth } from "../../lib/auth";
import { isTauri } from "../../lib/platform";
import { appendConsoleEntries, consoleCounts, useConsoleEntries } from "../../lib/previewConsole";
import { isLoopbackUrl, loadRecents, rememberRecent } from "../../lib/previewRecents";
import { insertIntoTerminal } from "../../lib/terminalInsert";
import { addVisualAnnotation, type VisualAnnotation } from "../../lib/visualAnnotations";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { IconButton } from "../ds/IconButton";
import { useToast } from "../ds/ToastHost";
import { useWorkbench } from "../workbench/WorkbenchProvider";
import { type WorkbenchSurface } from "../workbench/model";
import { PreviewConsolePanel } from "./PreviewConsolePanel";
import { PreviewTabStrip } from "./PreviewTabStrip";
import { RecentUrls } from "./RecentUrls";

interface BrowserPreviewDockProps {
  sessionId: string;
  surface: WorkbenchSurface;
}

const VIEWPORTS: readonly { value: PreviewViewport; label: string }[] = [
  { value: "fill", label: "Fit" },
  { value: "mobile", label: "390" },
  { value: "tablet", label: "768" },
];
const PICKER_NEEDS_DESKTOP = "Element picking needs the desktop app";
const MIN_ZOOM = 0.5;
const MAX_ZOOM = 2;
const ZOOM_STEP = 0.1;

function roundZoom(value: number): number {
  return Math.round(value * 10) / 10;
}

export function BrowserPreviewDock({ sessionId, surface }: BrowserPreviewDockProps) {
  const tokens = useTokens();
  const toast = useToast();
  const workbench = useWorkbench();
  const { baseUrl } = useAuth();
  const isWebFrame = Platform.OS === "web" && !isTauri;
  const label = previewLabel(surface.id);
  const initial = getPreviewPreferences(label);
  const bodyRef = useRef<View>(null);
  const currentUrlRef = useRef(initial.url);
  const [url, setUrl] = useState(initial.url);
  const [draftUrl, setDraftUrl] = useState(initial.url);
  const [viewport, setViewport] = useState<PreviewViewport>(initial.viewport);
  const [zoom, setZoom] = useState(initial.zoom);
  const [layoutVersion, setLayoutVersion] = useState(0);
  const [loading, setLoading] = useState(false);
  const [pickerActive, setPickerActive] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [recents, setRecents] = useState<string[]>([]);
  const [recentsOpen, setRecentsOpen] = useState(false);
  const [consoleOpen, setConsoleOpen] = useState(false);
  const [lastPick, setLastPick] = useState<VisualAnnotation | null>(null);
  const [frameKey, setFrameKey] = useState(0);
  const consoleEntries = useConsoleEntries(label);
  const problems = consoleCounts(consoleEntries);
  const blurTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    let disposed = false;
    void loadRecents().then((stored) => {
      if (!disposed) setRecents(stored);
    });
    return () => {
      disposed = true;
      if (blurTimer.current) clearTimeout(blurTimer.current);
    };
  }, []);

  const measureBounds = useCallback(
    () =>
      new Promise<PreviewBounds | null>((resolve) => {
        const node = bodyRef.current;
        if (!node) return resolve(null);
        node.measureInWindow((x, y, width, height) => {
          if (width < 1 || height < 1) return resolve(null);
          resolve(previewViewportBounds({ x, y, width, height }, viewport));
        });
      }),
    [viewport],
  );

  const showAtCurrentBounds = useCallback(
    async (nextUrl: string) => {
      const bounds = await measureBounds();
      if (!bounds) throw new Error("Preview panel has not finished laying out.");
      await openBrowserPreview(label, nextUrl, bounds);
      if (zoom !== 1) await zoomBrowserPreview(label, zoom);
    },
    [label, measureBounds, zoom],
  );

  useEffect(() => {
    currentUrlRef.current = url;
  }, [url]);

  useEffect(() => {
    let disposed = false;
    let unlisten: () => void = () => undefined;
    void listenBrowserPreview(
      (event) => {
        if (disposed || event.label !== label) return;
        setUrl(event.url);
        setDraftUrl(event.url);
        setPreviewPreferences(label, { url: event.url });
        setLoading(!event.loaded);
        setError(null);
      },
      (event) => {
        if (disposed || event.label !== label) return;
        setLastPick(addVisualAnnotation(sessionId, event.annotation));
        setPickerActive(false);
        toast.show("Element added to the composer", { tone: "success" });
      },
      (event) => {
        if (!disposed && event.label === label) appendConsoleEntries(label, event.entries);
      },
    )
      .then((cleanup) => {
        if (disposed) cleanup();
        else unlisten = cleanup;
      })
      .catch((cause) => {
        if (!disposed) setError(cause instanceof Error ? cause.message : String(cause));
      });
    return () => {
      disposed = true;
      unlisten();
    };
  }, [label, sessionId, toast]);

  useEffect(() => {
    if (!isTauri || !url) return;
    let cancelled = false;
    void showAtCurrentBounds(url).catch((cause) => {
      if (!cancelled) {
        setLoading(false);
        setError(cause instanceof Error ? cause.message : String(cause));
      }
    });
    return () => {
      cancelled = true;
    };
  }, [layoutVersion, showAtCurrentBounds, url, viewport]);

  useEffect(
    () => () => {
      if (currentUrlRef.current) void hideBrowserPreview(label).catch(() => undefined);
    },
    [label],
  );

  const submitUrl = async () => {
    try {
      const normalized = normalizePreviewUrl(draftUrl);
      if (isWebFrame && !isLoopbackUrl(normalized)) {
        throw new Error("The web app previews local servers only. Open this address in the desktop app.");
      }
      setError(null);
      setLoading(!isWebFrame);
      setUrl(normalized);
      setDraftUrl(normalized);
      setRecentsOpen(false);
      setPreviewPreferences(label, { url: normalized });
      void rememberRecent(normalized).then(setRecents);
      if (isWebFrame) setFrameKey((key) => key + 1);
      else if (normalized === currentUrlRef.current) await navigateBrowserPreview(label, normalized);
    } catch (cause) {
      setLoading(false);
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const run = async (operation: () => Promise<void>) => {
    try {
      setError(null);
      await operation();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const changeZoom = (delta: number) => {
    const next = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, roundZoom(zoom + delta)));
    setZoom(next);
    setPreviewPreferences(label, { zoom: next });
  };

  const changeViewport = (next: PreviewViewport) => {
    setViewport(next);
    setPreviewPreferences(label, { viewport: next });
  };

  const pickRecent = (recent: string) => {
    setDraftUrl(recent);
    setRecentsOpen(false);
    setError(null);
    setLoading(!isWebFrame);
    setUrl(recent);
    setPreviewPreferences(label, { url: recent });
    void rememberRecent(recent).then(setRecents);
    if (isWebFrame) setFrameKey((key) => key + 1);
    else if (recent === currentUrlRef.current) void run(() => navigateBrowserPreview(label, recent));
  };

  const insertPickIntoTerminal = async () => {
    if (!lastPick || !baseUrl) return;
    try {
      const typed = await insertIntoTerminal(baseUrl, sessionId, lastPick.selector);
      toast.show(typed ? "Selector typed into the terminal" : "No running terminal in this session", {
        tone: typed ? "success" : "neutral",
      });
    } catch (cause) {
      toast.show(cause instanceof Error ? cause.message : "Could not reach the terminal.", { tone: "danger" });
    }
  };

  const addTab = () => {
    const count = workbench.state.right.tabs.filter((tab) => tab.kind === "preview").length + 1;
    workbench.openSurface({
      kind: "preview",
      sessionId,
      resourceId: `tab-${Date.now().toString(36)}`,
      title: `Preview ${count}`,
    });
  };

  const openExternal = async () => {
    if (!url) return;
    try {
      const { openUrl } = await import("@tauri-apps/plugin-opener");
      await openUrl(url);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const previewTabs = workbench.state.right.tabs.filter(
    (tab) => tab.kind === "preview" && tab.sessionId === sessionId,
  );

  const onBodyLayout = (_event: LayoutChangeEvent) => setLayoutVersion((version) => version + 1);

  if (!isTauri && !isWebFrame) {
    return (
      <View style={styles.unavailable}>
        <Text style={[typeScale.bodyBold, { color: tokens.ink }]}>Desktop browser preview</Text>
        <Text style={[typeScale.sub, styles.unavailableCopy, { color: tokens.ink3 }]}>
          Embedded previews use Forge&apos;s isolated native webview and are available in the
          desktop app. Web and mobile clients never load arbitrary sites inside the app.
        </Text>
      </View>
    );
  }

  return (
    <View style={[styles.root, { backgroundColor: tokens.bg1 }]}>
      <View style={[styles.chrome, { borderBottomColor: tokens.border, backgroundColor: tokens.bg1 }]}>
        <View style={styles.navigation}>
          <IconButton
            icon={<ArrowLeft size={16} color={tokens.ink2} />}
            onPress={() => void run(() => moveBrowserPreviewHistory(label, -1))}
            disabled={!url || isWebFrame}
            accessibilityLabel="Go back in preview"
            style={styles.compactButton}
          />
          <IconButton
            icon={<ArrowRight size={16} color={tokens.ink2} />}
            onPress={() => void run(() => moveBrowserPreviewHistory(label, 1))}
            disabled={!url || isWebFrame}
            accessibilityLabel="Go forward in preview"
            style={styles.compactButton}
          />
          <IconButton
            icon={<RefreshCw size={15} color={tokens.ink2} />}
            onPress={() => (isWebFrame ? setFrameKey((key) => key + 1) : void run(() => reloadBrowserPreview(label)))}
            disabled={!url}
            accessibilityLabel="Reload preview"
            style={styles.compactButton}
          />
        </View>

        <View style={[styles.address, { borderColor: tokens.border, backgroundColor: tokens.bg0 }]}>
          {loading ? <ActivityIndicator size="small" color={tokens.accent} /> : null}
          <TextInput
            value={draftUrl}
            onChangeText={setDraftUrl}
            onSubmitEditing={() => void submitUrl()}
            onFocus={() => {
              if (blurTimer.current) clearTimeout(blurTimer.current);
              setRecentsOpen(true);
            }}
            onBlur={() => {
              blurTimer.current = setTimeout(() => setRecentsOpen(false), 160);
            }}
            autoCapitalize="none"
            autoCorrect={false}
            spellCheck={false}
            keyboardType="url"
            placeholder="URL or local port (for example 5173)"
            placeholderTextColor={tokens.ink4}
            selectionColor={tokens.accent}
            accessibilityLabel="Preview URL"
            style={[typeScale.monoMeta, styles.addressInput, { color: tokens.ink }]}
          />
        </View>

        <IconButton
          icon={<ExternalLink size={15} color={tokens.ink2} />}
          onPress={() => void openExternal()}
          disabled={!url}
          accessibilityLabel="Open preview in default browser"
          style={styles.compactButton}
        />
        <IconButton
          icon={
            <MousePointer2
              size={16}
              color={pickerActive ? tokens.accent : tokens.ink2}
              fill={pickerActive ? tokens.selection : "transparent"}
            />
          }
          onPress={() => {
            const next = !pickerActive;
            setPickerActive(next);
            void run(() => setBrowserPreviewPicker(label, next));
          }}
          disabled={!url || isWebFrame}
          accessibilityLabel={
            isWebFrame ? PICKER_NEEDS_DESKTOP : pickerActive ? "Cancel element picker" : "Pick element for prompt"
          }
          style={[
            styles.compactButton,
            pickerActive && { backgroundColor: tokens.selection },
          ]}
        />
        <IconButton
          icon={<SquareTerminal size={16} color={lastPick ? tokens.ink2 : tokens.ink4} />}
          onPress={() => void insertPickIntoTerminal()}
          disabled={!lastPick || isWebFrame}
          accessibilityLabel="Insert picked element into terminal"
          style={styles.compactButton}
        />
        <IconButton
          icon={
            <ScrollText
              size={16}
              color={consoleOpen ? tokens.accent : problems.error > 0 ? tokens.danger : tokens.ink2}
            />
          }
          onPress={() => setConsoleOpen((open) => !open)}
          disabled={isWebFrame}
          accessibilityLabel={
            isWebFrame
              ? "Console needs the desktop app"
              : consoleOpen
                ? "Hide console"
                : `Show console${problems.error > 0 ? `, ${problems.error} errors` : ""}`
          }
          style={[styles.compactButton, consoleOpen && { backgroundColor: tokens.selection }]}
        />
        {previewTabs.length < 2 ? (
          <IconButton
            icon={<Plus size={16} color={tokens.ink2} />}
            onPress={addTab}
            accessibilityLabel="Add browser preview tab"
            style={styles.compactButton}
          />
        ) : null}
      </View>

      {previewTabs.length >= 2 ? (
        <PreviewTabStrip
          tabs={previewTabs}
          activeId={surface.id}
          onActivate={(id) => workbench.activateSurface("right", id)}
          onClose={(id) => workbench.closeSurface("right", id)}
          onAdd={addTab}
        />
      ) : null}
      {recentsOpen ? <RecentUrls urls={recents} onPick={pickRecent} /> : null}

      <View style={[styles.tools, { borderBottomColor: tokens.border }]}>
        <View style={styles.viewportOptions} accessibilityRole="tablist">
          {VIEWPORTS.map((option) => {
            const selected = option.value === viewport;
            return (
              <Pressable
                key={option.value}
                onPress={() => changeViewport(option.value)}
                accessibilityRole="tab"
                accessibilityState={{ selected }}
                accessibilityLabel={`${option.label} preview viewport`}
                style={[
                  styles.viewportOption,
                  {
                    borderColor: selected ? tokens.accent : tokens.border,
                    backgroundColor: selected ? tokens.selection : tokens.bg0,
                  },
                ]}
              >
                <Text
                  style={[
                    typeScale.monoMeta,
                    { color: selected ? tokens.accent : tokens.ink3 },
                  ]}
                >
                  {option.label}
                </Text>
              </Pressable>
            );
          })}
        </View>
        <View style={styles.toolSpacer} />
        {isWebFrame ? (
          <Text style={[typeScale.meta, { color: tokens.ink4 }]} numberOfLines={1}>
            {PICKER_NEEDS_DESKTOP}
          </Text>
        ) : null}
        {isWebFrame ? null : (<>
        <IconButton
          icon={<Minus size={14} color={tokens.ink3} />}
          onPress={() => changeZoom(-ZOOM_STEP)}
          disabled={!url || zoom <= MIN_ZOOM}
          accessibilityLabel="Zoom preview out"
          style={styles.tinyButton}
        />
        <Pressable
          onPress={() => {
            setZoom(1);
            setPreviewPreferences(label, { zoom: 1 });
          }}
          disabled={!url}
          accessibilityRole="button"
          accessibilityLabel="Reset preview zoom"
          style={styles.zoomLabel}
        >
          <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>
            {Math.round(zoom * 100)}%
          </Text>
        </Pressable>
        <IconButton
          icon={<Plus size={14} color={tokens.ink3} />}
          onPress={() => changeZoom(ZOOM_STEP)}
          disabled={!url || zoom >= MAX_ZOOM}
          accessibilityLabel="Zoom preview in"
          style={styles.tinyButton}
        />
        </>)}
      </View>

      {error ? (
        <View style={[styles.error, { backgroundColor: tokens.dangerBg }]}>
          <Text style={[typeScale.meta, { color: tokens.danger }]}>{error}</Text>
        </View>
      ) : null}

      <View
        ref={bodyRef}
        onLayout={onBodyLayout}
        collapsable={false}
        style={[styles.body, { backgroundColor: tokens.bg0 }]}
      >
        {!url ? (
          <View style={styles.empty}>
            <Text style={[typeScale.bodyBold, { color: tokens.ink }]}>Open a local app or URL</Text>
            <Text style={[typeScale.sub, styles.emptyCopy, { color: tokens.ink3 }]}>
              {isWebFrame
                ? "Enter a development port such as 5173 or a localhost address. The web app frames local servers only; the desktop app opens any site and can pick elements."
                : "Enter a development port such as 5173, localhost address, or HTTPS URL. Use the pointer tool to attach an exact DOM element to your next Forge prompt."}
            </Text>
            {recents.length > 0 ? <RecentUrls urls={recents.slice(0, 5)} onPick={pickRecent} /> : null}
          </View>
        ) : isWebFrame ? (
          <View style={[styles.frameStage, { backgroundColor: tokens.bg1 }]}>
            <View style={[styles.frameBox, viewportWidth(viewport)]}>
              {React.createElement("iframe", {
                key: `${url}:${frameKey}`,
                src: url,
                title: "Browser preview",
                onLoad: () => setLoading(false),
                style: { width: "100%", height: "100%", border: 0, background: "#fff" },
              })}
            </View>
          </View>
        ) : null}
      </View>
      {consoleOpen && !isWebFrame ? <PreviewConsolePanel label={label} /> : null}
    </View>
  );
}

function viewportWidth(viewport: PreviewViewport): { width: number | "100%" } {
  return { width: viewport === "mobile" ? 390 : viewport === "tablet" ? 768 : "100%" };
}

const styles = StyleSheet.create({
  frameStage: { flex: 1, alignItems: "center" },
  frameBox: { height: "100%", maxWidth: "100%" },
  root: { flex: 1, minHeight: 0 },
  chrome: {
    minHeight: 42,
    flexShrink: 0,
    borderBottomWidth: StyleSheet.hairlineWidth,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space4,
    paddingHorizontal: space.space8,
  },
  navigation: { flexDirection: "row", alignItems: "center" },
  compactButton: { width: 30, height: 30 },
  tinyButton: { width: 26, height: 26 },
  address: {
    flex: 1,
    minWidth: 120,
    height: 30,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius4,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space4,
    paddingHorizontal: space.space8,
  },
  addressInput: { flex: 1, minWidth: 0, paddingVertical: 0 },
  tools: {
    height: 34,
    flexShrink: 0,
    borderBottomWidth: StyleSheet.hairlineWidth,
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: space.space8,
    gap: space.space4,
  },
  viewportOptions: { flexDirection: "row", gap: space.space4 },
  viewportOption: {
    minWidth: 38,
    height: 24,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius4,
    paddingHorizontal: 6,
  },
  toolSpacer: { flex: 1 },
  zoomLabel: { minWidth: 42, height: 26, alignItems: "center", justifyContent: "center" },
  error: { flexShrink: 0, paddingHorizontal: space.space12, paddingVertical: space.space8 },
  body: { flex: 1, minHeight: 0 },
  empty: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    gap: space.space8,
    padding: space.space24,
  },
  emptyCopy: { maxWidth: 430, textAlign: "center" },
  unavailable: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    gap: space.space8,
    padding: space.space24,
  },
  unavailableCopy: { maxWidth: 420, textAlign: "center" },
});
