// DESIGN.md "Mobile/desktop (V3)" — the seam ARCHITECTURE.md §2 and FEATURES.md reserve for
// voice input. `voice.ts` (native, expo-audio) and `voice.web.ts` (web + Tauri desktop webview,
// WebAudio) both implement this contract; Metro's platform extensions pick the right one.
export interface VoiceRecorder {
  /** Begin capturing. `onLevel` fires repeatedly with a 0..1 amplitude for the recording pill. */
  start(onLevel: (rms01: number) => void): Promise<void>;
  /** Stop capturing and return the encoded audio, ready to append to a FormData upload. */
  stop(): Promise<{ blobOrFile: unknown; name: string; mime: string }>;
  /** Abort capturing and discard whatever was recorded so far. */
  cancel(): Promise<void>;
  /** Whether this platform/environment can record at all — gates the Composer's mic button. */
  isSupported(): boolean;
}
