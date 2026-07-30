import {
  ArrowLeft,
  ArrowRight,
  ExternalLink,
  Minus,
  MousePointer2,
  Plus,
  RefreshCw,
} from "lucide-react-native";
import React, { useCallback, useEffect, useRef, useState } from "react";
import {
  ActivityIndicator,
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
import { isTauri } from "../../lib/platform";
import { addVisualAnnotation } from "../../lib/visualAnnotations";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { IconButton } from "../ds/IconButton";
import { useToast } from "../ds/ToastHost";
import { useWorkbench } from "../workbench/WorkbenchProvider";
import { type WorkbenchSurface } from "../workbench/model";

interface BrowserPreviewDockProps {
  sessionId: string;
  surface: WorkbenchSurface;
}

const VIEWPORTS: readonly { value: PreviewViewport; label: string }[] = [
  { value: "fill", label: "Fit" },
  { value: "mobile", label: "390" },
  { value: "tablet", label: "768" },
];
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
        addVisualAnnotation(sessionId, event.annotation);
        setPickerActive(false);
        toast.show("Element added to the composer", { tone: "success" });
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
      setError(null);
      setLoading(true);
      setUrl(normalized);
      setDraftUrl(normalized);
      setPreviewPreferences(label, { url: normalized });
      if (normalized === currentUrlRef.current) await navigateBrowserPreview(label, normalized);
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

  const onBodyLayout = (_event: LayoutChangeEvent) => setLayoutVersion((version) => version + 1);

  if (!isTauri) {
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
            disabled={!url}
            accessibilityLabel="Go back in preview"
            style={styles.compactButton}
          />
          <IconButton
            icon={<ArrowRight size={16} color={tokens.ink2} />}
            onPress={() => void run(() => moveBrowserPreviewHistory(label, 1))}
            disabled={!url}
            accessibilityLabel="Go forward in preview"
            style={styles.compactButton}
          />
          <IconButton
            icon={<RefreshCw size={15} color={tokens.ink2} />}
            onPress={() => void run(() => reloadBrowserPreview(label))}
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
          disabled={!url}
          accessibilityLabel={pickerActive ? "Cancel element picker" : "Pick element for prompt"}
          style={[
            styles.compactButton,
            pickerActive && { backgroundColor: tokens.selection },
          ]}
        />
        <IconButton
          icon={<Plus size={16} color={tokens.ink2} />}
          onPress={addTab}
          accessibilityLabel="Add browser preview tab"
          style={styles.compactButton}
        />
      </View>

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
              Enter a development port such as 5173, localhost address, or HTTPS URL. Use the
              pointer tool to attach an exact DOM element to your next Forge prompt.
            </Text>
          </View>
        ) : null}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
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
