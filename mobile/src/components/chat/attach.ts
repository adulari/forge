// DESIGN_SYSTEM.md §6 Composer attach + FEATURES.md §1.1 upload: image/document pickers and
// web paste-image extraction. Pure picker/FormData helpers only — the actual POST goes through
// `useUpload()` (lib/queries.ts) from the Composer, never a raw fetch here (UI_RULES.md #3).
import * as DocumentPicker from "expo-document-picker";
import * as ImagePicker from "expo-image-picker";
import { Platform } from "react-native";

export type AttachmentState = "uploading" | "done" | "error";

/** One attach chip's render state, tracked by the Composer. */
export interface Attachment {
  id: string;
  name: string;
  image: boolean;
  state: AttachmentState;
  /** Server-relative path once uploaded (`UploadedFile.path` from api.ts). */
  path?: string;
  /** Local preview source (native `file://`/`ph://` URI, or a web `blob:` object URL) — used
   * for the composer thumbnail and carried through to the optimistic sent bubble. */
  uri?: string;
}

/** A picked file in RN's `{uri,name,type}` shape, before upload. */
export interface PickedFile {
  uri: string;
  name: string;
  mimeType: string;
  image: boolean;
  /**
   * The real web `File`, present only on `Platform.OS === "web"`. RN's `{uri,name,type}`
   * shorthand only becomes a multipart file part through React Native's OWN networking layer
   * (native iOS/Android) — a real browser's `fetch`/`FormData` has no such magic and will
   * just `String()`-coerce a plain object, uploading garbage. Web callers MUST append this
   * `File` (via `formDataFromWebFiles`) instead of the shorthand.
   */
  file?: File;
}

let nextId = 0;
export function makeAttachmentId(): string {
  return `att-${Date.now().toString(36)}-${nextId++}`;
}

/** Photo library picker (multi-select). Returns `[]` on cancel or denied permission. */
export async function pickImages(): Promise<PickedFile[]> {
  const perm = await ImagePicker.requestMediaLibraryPermissionsAsync();
  if (!perm.granted) return [];
  const result = await ImagePicker.launchImageLibraryAsync({
    mediaTypes: ["images"],
    allowsMultipleSelection: true,
    quality: 0.9,
  });
  if (result.canceled) return [];
  return result.assets.map((a) => ({
    uri: a.uri,
    name: a.fileName ?? a.uri.split("/").pop() ?? "image.jpg",
    mimeType: a.mimeType ?? "image/jpeg",
    image: true,
    file: a.file,
  }));
}

/** Any-file document picker (multi-select). Returns `[]` on cancel. */
export async function pickDocuments(): Promise<PickedFile[]> {
  const result = await DocumentPicker.getDocumentAsync({ multiple: true, copyToCacheDirectory: true });
  if (result.canceled) return [];
  return result.assets.map((a) => ({
    uri: a.uri,
    name: a.name,
    mimeType: a.mimeType ?? "application/octet-stream",
    image: (a.mimeType ?? "").startsWith("image/"),
    file: a.file,
  }));
}

/** Web-only: extract pasted image files from a DOM ClipboardEvent (composer paste handler). */
export function imagesFromClipboardEvent(e: ClipboardEvent): File[] {
  if (Platform.OS !== "web") return [];
  const items = e.clipboardData?.items;
  if (!items) return [];
  const files: File[] = [];
  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    if (item.kind === "file" && item.type.startsWith("image/")) {
      const file = item.getAsFile();
      if (file) files.push(file);
    }
  }
  return files;
}

/** Multipart body for native-picked files (RN's `{uri,name,type}` upload shape). */
export function formDataFromPicked(files: PickedFile[]): FormData {
  const form = new FormData();
  for (const f of files) {
    form.append("files", { uri: f.uri, name: f.name, type: f.mimeType } as unknown as Blob);
  }
  return form;
}

/** Multipart body for real web `File` objects (paste-image). */
export function formDataFromWebFiles(files: File[]): FormData {
  const form = new FormData();
  for (const f of files) form.append("files", f, f.name);
  return form;
}

/**
 * A successfully-uploaded attachment as it rides the optimistic "just sent" bubble
 * (MessageRow renders these directly — client-local memory of what THIS device just sent,
 * before the history refetch lands). MessageRow also builds this same shape for a persisted
 * history row's `@path` mention (files always had one; images now do too), so `AttachmentRow`
 * renders both the live and the reconstructed-from-history case identically.
 */
export interface SentAttachment {
  id: string;
  name: string;
  image: boolean;
  uri?: string;
}
