// Chat segment (BUILD_PLAN §6 "Chat" / §7 Batch 2 W5). Merges paginated `useHistory`
// backlog (older scrollback) with the live snapshot's short `transcript` tail + in-flight
// `streaming` partial — mirrors remote_assets/app.js's `#hist` (above) + `#tail` (live,
// below) split. Renders content only (UI_RULES.md #1 — the session shell owns the Screen).
// Action cards (permission/question/plan/diff) are explicitly OUT of scope here — Batch 3.
import * as Clipboard from "expo-clipboard";
import * as DocumentPicker from "expo-document-picker";
import * as ImagePicker from "expo-image-picker";
import AsyncStorage from "@react-native-async-storage/async-storage";
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  type FlatList,
  KeyboardAvoidingView,
  Modal,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
  Platform,
  Pressable,
  ScrollView,
  Text,
  TextInput,
  View,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { ApiError, type HistoryRow } from "../../../lib/api";
import { theme } from "../../../lib/theme";
import { useHistory, useUpload } from "../../../lib/queries";
import { useSessionCtx } from "../../../lib/sessionContext";
import {
  Badge,
  BoundedList,
  Card,
  Chip,
  EmptyState,
  ErrorText,
  Loading,
  PrimaryButton,
} from "../../../components/ui";

const OFFLINE_CAP = 20;
const COMMAND_CHIPS = ["/plan", "/compact", "/models", "/mode", "/help"];

function offlineKey(baseUrl: string | null, sessionId: string): string {
  return `forge-oq:${baseUrl ?? "none"}:${sessionId}`;
}

function formatTime(unixSeconds: number): string {
  if (!unixSeconds) return "";
  return new Date(unixSeconds * 1000).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
}

// ---------------------------------------------------------------------------
// Markdown-lite (fenced code + inline bold/italic/code/links) — DOM-free port of
// remote_assets/app.js's mdRender/inlineMd, rendered as RN Text/View trees.
// ---------------------------------------------------------------------------

type Block =
  | { type: "code"; lang: string; code: string }
  | { type: "heading"; level: number; text: string }
  | { type: "list"; ordered: boolean; items: string[] }
  | { type: "para"; text: string };

function parseBlocks(src: string): Block[] {
  const lines = src.split("\n");
  const blocks: Block[] = [];
  let para: string[] = [];
  const flushPara = () => {
    if (para.length) {
      blocks.push({ type: "para", text: para.join("\n") });
      para = [];
    }
  };
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    const fence = line.match(/^\s*```([\w+-]*)\s*$/);
    if (fence) {
      flushPara();
      const code: string[] = [];
      i++;
      while (i < lines.length && !/^\s*```\s*$/.test(lines[i])) {
        code.push(lines[i]);
        i++;
      }
      i++;
      blocks.push({ type: "code", lang: (fence[1] || "").toLowerCase(), code: code.join("\n") });
      continue;
    }
    const heading = line.match(/^(#{1,6})\s+(.*)$/);
    if (heading) {
      flushPara();
      blocks.push({ type: "heading", level: heading[1].length, text: heading[2] });
      i++;
      continue;
    }
    const listItem = line.match(/^\s*([-*]|\d+\.)\s+(.*)$/);
    if (listItem) {
      flushPara();
      const ordered = /^\d/.test(listItem[1]);
      const items: string[] = [];
      while (i < lines.length) {
        const m = lines[i].match(/^\s*([-*]|\d+\.)\s+(.*)$/);
        if (!m || /^\d/.test(m[1]) !== ordered) break;
        items.push(m[2]);
        i++;
      }
      blocks.push({ type: "list", ordered, items });
      continue;
    }
    if (!line.trim()) {
      flushPara();
      i++;
      continue;
    }
    para.push(line);
    i++;
  }
  flushPara();
  return blocks;
}

function renderInline(text: string): React.ReactNode[] {
  const re = /(`([^`]+)`)|(\*\*([^*]+)\*\*)|(\*([^*\s][^*]*)\*)|(\[([^\]]+)\]\(([^)]+)\))/g;
  const nodes: React.ReactNode[] = [];
  let last = 0;
  let m: RegExpExecArray | null;
  let key = 0;
  while ((m = re.exec(text))) {
    if (m.index > last) nodes.push(text.slice(last, m.index));
    if (m[2] !== undefined) {
      nodes.push(
        <Text key={key++} className="text-ink bg-chipBg text-[13px]">
          {m[2]}
        </Text>,
      );
    } else if (m[4] !== undefined) {
      nodes.push(
        <Text key={key++} className="text-ink font-bold">
          {m[4]}
        </Text>,
      );
    } else if (m[6] !== undefined) {
      nodes.push(
        <Text key={key++} className="text-ink italic">
          {m[6]}
        </Text>,
      );
    } else if (m[8] !== undefined) {
      nodes.push(m[8]);
    }
    last = re.lastIndex;
  }
  if (last < text.length) nodes.push(text.slice(last));
  return nodes;
}

function CodeBlock({ code }: { code: string }) {
  const [copied, setCopied] = useState(false);
  const onCopy = useCallback(() => {
    Clipboard.setStringAsync(code).catch(() => {});
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  }, [code]);
  return (
    <View className="bg-codeBg rounded-md border border-borderSoft overflow-hidden">
      <View className="flex-row justify-end px-6 pt-4">
        <Pressable
          onPress={onCopy}
          hitSlop={8}
          style={{ minHeight: 32, minWidth: 44, alignItems: "center", justifyContent: "center" }}
        >
          <Text className="text-dim text-[11px]">{copied ? "copied" : "copy"}</Text>
        </Pressable>
      </View>
      <ScrollView horizontal showsHorizontalScrollIndicator={false} className="px-8 pb-8">
        <Text
          className="text-ink text-[12px]"
          style={{
            fontFamily: Platform.select({ ios: "Menlo", android: "monospace", default: "ui-monospace" }),
            lineHeight: 18,
          }}
        >
          {code}
        </Text>
      </ScrollView>
    </View>
  );
}

const MessageContent = React.memo(function MessageContent({ text }: { text: string }) {
  const blocks = useMemo(() => parseBlocks(text), [text]);
  return (
    <View className="gap-6">
      {blocks.map((b, i) => {
        if (b.type === "code") return <CodeBlock key={i} code={b.code} />;
        if (b.type === "heading") {
          return (
            <Text key={i} className="text-ink font-bold text-[15px]">
              {renderInline(b.text)}
            </Text>
          );
        }
        if (b.type === "list") {
          return (
            <View key={i} className="gap-2">
              {b.items.map((it, j) => (
                <View key={j} className="flex-row gap-6">
                  <Text className="text-dim text-[14px]">{b.ordered ? `${j + 1}.` : "•"}</Text>
                  <Text className="flex-1 text-ink text-[14px] leading-[21px]">{renderInline(it)}</Text>
                </View>
              ))}
            </View>
          );
        }
        return (
          <Text key={i} className="text-ink text-[14px] leading-[21px]">
            {renderInline(b.text)}
          </Text>
        );
      })}
    </View>
  );
});

// ---------------------------------------------------------------------------
// Row types — history (paginated, persisted) + tail lines + streaming (both live,
// from the snapshot only).
// ---------------------------------------------------------------------------

type ChatRow =
  | { kind: "streaming"; id: string; text: string }
  | { kind: "tail"; id: string; text: string }
  | { kind: "history"; id: string; row: HistoryRow };

function HistoryMessageRow({ row }: { row: HistoryRow }) {
  const isNote = row.visibility === "ui";
  const isUser = row.role === "user";
  const label = isNote ? "note" : isUser ? "you" : "forge";
  const labelClass = isNote ? "text-bannerInk" : isUser ? "text-accent" : "text-ok";
  return (
    <View className="px-12 py-8 border-b border-histBorder">
      <View className="flex-row items-center gap-6 mb-4">
        <Text className={`text-[12px] font-semibold ${labelClass}`}>{label}</Text>
        {row.model ? <Text className="text-dim text-[11px]">· {row.model}</Text> : null}
        <View className="flex-1" />
        <Text className="text-footer text-[11px]" style={{ fontVariant: ["tabular-nums"] }}>
          {formatTime(row.created_at)}
        </Text>
      </View>
      <MessageContent text={row.content} />
    </View>
  );
}

function TailLineRow({ text }: { text: string }) {
  return (
    <View className="px-12 py-4">
      <Text className="text-ink text-[14px] leading-[21px]">{text}</Text>
    </View>
  );
}

function StreamingRow({ text }: { text: string }) {
  return (
    <View className="px-12 py-8">
      <Text className="text-stream text-[14px] italic leading-[21px]">{text}</Text>
    </View>
  );
}

function ChatRowItem({ item }: { item: ChatRow }) {
  if (item.kind === "streaming") return <StreamingRow text={item.text} />;
  if (item.kind === "tail") return <TailLineRow text={item.text} />;
  return <HistoryMessageRow row={item.row} />;
}
const ChatRowItemMemo = React.memo(ChatRowItem, (prev, next) => {
  const a = prev.item;
  const b = next.item;
  if (a.kind !== b.kind || a.id !== b.id) return false;
  if (a.kind === "history" && b.kind === "history") {
    return (
      a.row.seq === b.row.seq &&
      a.row.content === b.row.content &&
      a.row.model === b.row.model &&
      a.row.visibility === b.row.visibility
    );
  }
  if (a.kind === "tail" && b.kind === "tail") return a.text === b.text;
  if (a.kind === "streaming" && b.kind === "streaming") return a.text === b.text;
  return false;
});

// ---------------------------------------------------------------------------
// Upload attach sheet (document/image picker → POST /api/upload)
// ---------------------------------------------------------------------------

interface AttachSheetProps {
  visible: boolean;
  onClose: () => void;
  onPickImage: () => void;
  onPickDocument: () => void;
}

function AttachSheet({ visible, onClose, onPickImage, onPickDocument }: AttachSheetProps) {
  return (
    <Modal visible={visible} transparent animationType="fade" onRequestClose={onClose}>
      <Pressable className="flex-1 bg-black/60 justify-end" onPress={onClose}>
        <Pressable onPress={() => undefined}>
          <Card variant="feature" className="mx-12 mb-16 gap-8">
            <Text className="text-ink text-[15px] font-bold">Attach</Text>
            <Pressable onPress={onPickImage} style={{ minHeight: 44, justifyContent: "center" }}>
              <Text className="text-ink text-[15px]">Photo library</Text>
            </Pressable>
            <Pressable onPress={onPickDocument} style={{ minHeight: 44, justifyContent: "center" }}>
              <Text className="text-ink text-[15px]">File</Text>
            </Pressable>
            <Pressable onPress={onClose} style={{ minHeight: 44, justifyContent: "center" }}>
              <Text className="text-dim text-[15px]">Cancel</Text>
            </Pressable>
          </Card>
        </Pressable>
      </Pressable>
    </Modal>
  );
}

interface UploadChip {
  id: number;
  name: string;
  state: string;
  failed: boolean;
}

// ---------------------------------------------------------------------------
// Chat screen
// ---------------------------------------------------------------------------

export default function ChatScreen() {
  const { sessionId, baseUrl, snapshot, connectionState, send } = useSessionCtx();
  const historyQuery = useHistory(sessionId);
  const uploadMutation = useUpload();
  const insets = useSafeAreaInsets();

  const [text, setText] = useState("");
  const [nearBottom, setNearBottom] = useState(true);
  const listRef = useRef<FlatList<ChatRow>>(null);
  const [offlineQueue, setOfflineQueue] = useState<string[]>([]);
  const [offlineDropped, setOfflineDropped] = useState(0);
  const [uploads, setUploads] = useState<UploadChip[]>([]);
  const [attachOpen, setAttachOpen] = useState(false);
  const uploadIdRef = useRef(0);

  // --- offline prompt queue: persisted per server+session, flushed in order on reconnect ---
  useEffect(() => {
    let cancelled = false;
    AsyncStorage.getItem(offlineKey(baseUrl, sessionId))
      .then((raw) => {
        if (cancelled || !raw) return;
        const parsed = JSON.parse(raw) as unknown;
        if (Array.isArray(parsed)) setOfflineQueue(parsed as string[]);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [baseUrl, sessionId]);

  const persistOfflineQueue = useCallback(
    (q: string[]) => {
      AsyncStorage.setItem(offlineKey(baseUrl, sessionId), JSON.stringify(q)).catch(() => {});
    },
    [baseUrl, sessionId],
  );

  const prevConnRef = useRef(connectionState);
  useEffect(() => {
    const prev = prevConnRef.current;
    prevConnRef.current = connectionState;
    if (connectionState === "open" && prev !== "open" && offlineQueue.length) {
      offlineQueue.forEach((t) => send({ kind: "prompt", text: t }));
      setOfflineQueue([]);
      persistOfflineQueue([]);
      setOfflineDropped(0);
    }
  }, [connectionState, offlineQueue, send, persistOfflineQueue]);

  const sendPrompt = useCallback(
    (value: string) => {
      const trimmed = value.trim();
      if (!trimmed) return;
      if (Platform.OS !== "web") {
        import("expo-haptics")
          .then((H) => H.impactAsync(H.ImpactFeedbackStyle.Light))
          .catch(() => {});
      }
      if (connectionState === "open") {
        send({ kind: "prompt", text: trimmed });
      } else {
        setOfflineQueue((q) => {
          if (q.length >= OFFLINE_CAP) {
            setOfflineDropped((d) => d + 1);
            return q;
          }
          const next = [...q, trimmed];
          persistOfflineQueue(next);
          return next;
        });
      }
    },
    [connectionState, send, persistOfflineQueue],
  );

  const onSubmit = useCallback(() => {
    if (!text.trim()) return;
    sendPrompt(text);
    setText("");
    setUploads([]);
  }, [text, sendPrompt]);

  // --- uploads ---
  const appendFileToForm = useCallback(
    async (form: FormData, uri: string, name: string, mimeType: string) => {
      if (Platform.OS === "web") {
        const res = await fetch(uri);
        const blob = await res.blob();
        form.append("file", blob, name);
      } else {
        form.append("file", { uri, name, type: mimeType } as unknown as Blob);
      }
    },
    [],
  );

  const doUpload = useCallback(
    async (uri: string, name: string, mimeType: string) => {
      const uid = ++uploadIdRef.current;
      setUploads((u) => [...u, { id: uid, name, state: "uploading…", failed: false }]);
      try {
        const form = new FormData();
        await appendFileToForm(form, uri, name, mimeType);
        const res = await uploadMutation.mutateAsync({ sessionId, form });
        const isImage = res.files[0]?.image ?? false;
        setUploads((u) =>
          u.map((x) =>
            x.id === uid
              ? {
                  ...x,
                  state: isImage
                    ? "image attached — rides the next prompt"
                    : "attached — rides the next prompt",
                }
              : x,
          ),
        );
      } catch (err) {
        const message = err instanceof ApiError ? err.message : "upload failed";
        setUploads((u) => u.map((x) => (x.id === uid ? { ...x, failed: true, state: message } : x)));
      }
    },
    [appendFileToForm, uploadMutation, sessionId],
  );

  const pickImage = useCallback(async () => {
    setAttachOpen(false);
    const result = await ImagePicker.launchImageLibraryAsync({
      mediaTypes: ["images"],
      quality: 0.8,
    });
    if (result.canceled) return;
    const a = result.assets[0];
    if (!a) return;
    await doUpload(a.uri, a.fileName ?? "image.jpg", a.mimeType ?? "image/jpeg");
  }, [doUpload]);

  const pickDocument = useCallback(async () => {
    setAttachOpen(false);
    const result = await DocumentPicker.getDocumentAsync({ multiple: false, copyToCacheDirectory: true });
    if (result.canceled) return;
    const a = result.assets[0];
    if (!a) return;
    await doUpload(a.uri, a.name, a.mimeType ?? "application/octet-stream");
  }, [doUpload]);

  // --- transcript: history (older, paginated) + live tail + streaming (BUILD_PLAN §6 Chat) ---
  const combined = useMemo<ChatRow[]>(() => {
    const rows: ChatRow[] = [];
    if (snapshot?.streaming) {
      rows.push({ kind: "streaming", id: "streaming", text: snapshot.streaming });
    }
    const tail = snapshot?.transcript ?? [];
    for (let i = tail.length - 1; i >= 0; i--) {
      rows.push({ kind: "tail", id: `tail-${i}`, text: tail[i] });
    }
    const pages = historyQuery.data?.pages ?? [];
    for (const page of pages) {
      for (const row of page) {
        rows.push({ kind: "history", id: `history-${row.seq}`, row });
      }
    }
    return rows;
  }, [snapshot?.streaming, snapshot?.transcript, historyQuery.data]);

  const keyExtractor = useCallback((item: ChatRow) => item.id, []);
  const renderItem = useCallback(({ item }: { item: ChatRow }) => <ChatRowItemMemo item={item} />, []);
  const emptyComponent = useMemo(
    () => <EmptyState title="No messages yet — say something to get started." />,
    [],
  );

  const onEndReached = useCallback(() => {
    if (historyQuery.hasNextPage && !historyQuery.isFetchingNextPage) {
      historyQuery.fetchNextPage();
    }
  }, [historyQuery]);

  const onScroll = useCallback((e: NativeSyntheticEvent<NativeScrollEvent>) => {
    setNearBottom(e.nativeEvent.contentOffset.y < 80);
  }, []);

  const jumpToLatest = useCallback(() => {
    listRef.current?.scrollToOffset({ offset: 0, animated: true });
  }, []);

  const queued = snapshot?.queued ?? [];
  const showLoading = historyQuery.isLoading && combined.length === 0;
  const showError = historyQuery.isError && combined.length === 0;

  return (
    <View className="flex-1">
      <KeyboardAvoidingView
        className="flex-1"
        behavior={Platform.OS === "ios" ? "padding" : undefined}
        keyboardVerticalOffset={Platform.OS === "ios" ? 90 : 0}
      >
        <View className="flex-1">
          {showLoading ? (
            <Loading label="Connecting to session…" />
          ) : showError ? (
            <ErrorText
              message={historyQuery.error instanceof ApiError ? historyQuery.error.message : "server unreachable"}
              onRetry={() => historyQuery.refetch()}
            />
          ) : (
            <BoundedList
              ref={listRef}
              data={combined}
              keyExtractor={keyExtractor}
              renderItem={renderItem}
              ListEmptyComponent={emptyComponent}
              inverted
              onEndReached={onEndReached}
              onEndReachedThreshold={0.4}
              onScroll={onScroll}
              scrollEventThrottle={32}
              keyboardShouldPersistTaps="handled"
            />
          )}
          {!nearBottom && combined.length > 0 ? (
            <Pressable
              onPress={jumpToLatest}
              className="absolute bottom-10 right-12 bg-accent rounded-pill px-16 items-center justify-center"
              style={{ minHeight: 44 }}
            >
              <Text className="text-panel text-[13px] font-bold">Jump to latest ↓</Text>
            </Pressable>
          ) : null}
        </View>

        <View
          className="border-t border-borderSoft bg-panel px-12 pt-8 gap-8"
          style={{ paddingBottom: 8 + insets.bottom }}
        >
          {uploads.length ? (
            <View className="flex-row flex-wrap gap-6">
              {uploads.map((u) => (
                <Badge key={u.id} label={`${u.failed ? "⚠ " : "📎 "}${u.name} · ${u.state}`} tone={u.failed ? "no" : "default"} />
              ))}
            </View>
          ) : null}
          {queued.length ? (
            <View className="gap-2">
              {queued.map((q, i) => (
                <Text key={i} numberOfLines={1} className="text-dim text-[12px]">
                  ⏳ queued: {q}
                </Text>
              ))}
            </View>
          ) : null}
          {offlineQueue.length || offlineDropped ? (
            <View className="gap-2">
              {offlineQueue.map((t, i) => (
                <Text key={i} numberOfLines={1} className="text-dim text-[12px]">
                  📴 queued (offline): {t}
                </Text>
              ))}
              {offlineDropped ? (
                <Text className="text-no text-[12px]">
                  ⚠ offline queue full ({OFFLINE_CAP}) — {offlineDropped} dropped
                </Text>
              ) : null}
            </View>
          ) : null}

          <ScrollView horizontal showsHorizontalScrollIndicator={false}>
            <View className="flex-row gap-6">
              {COMMAND_CHIPS.map((c) => (
                <Chip key={c} label={c} onPress={() => sendPrompt(c)} />
              ))}
            </View>
          </ScrollView>

          <View className="flex-row items-end gap-8">
            <Pressable
              onPress={() => setAttachOpen(true)}
              hitSlop={8}
              accessibilityRole="button"
              accessibilityLabel="Attach a file or photo"
              style={{ minWidth: 44, minHeight: 44, alignItems: "center", justifyContent: "center" }}
            >
              <Text className="text-dim text-[20px]">📎</Text>
            </Pressable>
            <TextInput
              value={text}
              onChangeText={setText}
              placeholder="type a task or /command…"
              placeholderTextColor={theme.colors.dim}
              multiline
              returnKeyType="send"
              className="flex-1 bg-panelDeep border border-border rounded-md px-10 py-8 text-ink text-[15px]"
              style={{ maxHeight: 120 }}
            />
            <PrimaryButton label="Send" onPress={onSubmit} fullWidth={false} disabled={!text.trim()} />
          </View>
        </View>
      </KeyboardAvoidingView>

      <AttachSheet
        visible={attachOpen}
        onClose={() => setAttachOpen(false)}
        onPickImage={pickImage}
        onPickDocument={pickDocument}
      />
    </View>
  );
}
