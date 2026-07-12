// Native (iOS/Android) voice capture — DESIGN.md "Client audio formats". `AudioModule.AudioRecorder`
// is used directly (not the `useAudioRecorder` hook) because this module's start/stop/cancel
// lifecycle is driven by VoiceRecordingPill, not tied to a single component's mount lifecycle.
import {
  AudioModule,
  AudioQuality,
  IOSOutputFormat,
  RecordingPresets,
  requestRecordingPermissionsAsync,
  setAudioModeAsync,
  type RecordingOptions,
} from "expo-audio";
import { File } from "expo-file-system";
import { Platform } from "react-native";

import type { VoiceRecorder } from "./types";

const POLL_MS = 80; // ~12Hz — within DESIGN.md's 10-15Hz metering-poll spec

// expo-audio's native metering is dBFS, roughly -160 (digital silence) to 0 (full scale) — but
// ordinary speech never gets near either extreme. Anchoring the floor at -60dB instead of -160
// keeps the amplitude bars responsive to normal speaking volume instead of pinned near zero.
const METER_FLOOR_DB = -60;

function dbToRms01(db: number): number {
  if (!Number.isFinite(db)) return 0;
  return Math.max(0, Math.min(1, (db - METER_FLOOR_DB) / -METER_FLOOR_DB));
}

// iOS records .wav, Android records .m4a (see RECORDING_OPTIONS below) — the map exists so a
// surprise extension (e.g. `.caf`) is still labeled honestly instead of silently uploaded as a
// mislabeled m4a.
const MIME_BY_EXT: Record<string, string> = {
  m4a: "audio/m4a",
  caf: "audio/x-caf",
  aac: "audio/aac",
  wav: "audio/wav",
  "3gp": "audio/3gpp",
};

// iOS records raw Linear PCM .wav instead of an m4a/AAC container: CoreAudio's own m4a muxing
// has a quirk the server's symphonia (Rust) decoder can't parse ("no audio track found"), even
// though an ffmpeg-muxed m4a decodes fine — so this sidesteps AAC container parsing entirely.
// 16kHz mono 16-bit is also whisper's native input, and Android's MediaRecorder can't write wav
// at all, so Android keeps AAC/m4a, just downsampled to match.
const RECORDING_OPTIONS: RecordingOptions =
  Platform.OS === "ios"
    ? {
        extension: ".wav",
        sampleRate: 16000,
        numberOfChannels: 1,
        bitRate: 16000 * 16,
        android: RecordingPresets.HIGH_QUALITY.android,
        ios: {
          outputFormat: IOSOutputFormat.LINEARPCM,
          audioQuality: AudioQuality.MAX,
          linearPCMBitDepth: 16,
          linearPCMIsBigEndian: false,
          linearPCMIsFloat: false,
        },
        web: RecordingPresets.HIGH_QUALITY.web,
      }
    : {
        extension: ".m4a",
        sampleRate: 16000,
        numberOfChannels: 1,
        bitRate: 64000,
        android: { outputFormat: "mpeg4", audioEncoder: "aac" },
        ios: RecordingPresets.HIGH_QUALITY.ios,
        web: RecordingPresets.HIGH_QUALITY.web,
      };

let recorder: InstanceType<typeof AudioModule.AudioRecorder> | null = null;
let pollId: ReturnType<typeof setInterval> | null = null;

function stopPolling(): void {
  if (pollId != null) {
    clearInterval(pollId);
    pollId = null;
  }
}

function teardown(): void {
  stopPolling();
  recorder?.release();
  recorder = null;
}

export const voice: VoiceRecorder = {
  isSupported() {
    return Platform.OS !== "web";
  },

  async start(onLevel) {
    const perm = await requestRecordingPermissionsAsync();
    if (!perm.granted) {
      throw new Error("microphone permission denied");
    }
    await setAudioModeAsync({ allowsRecording: true, playsInSilentMode: true });

    // expo-audio's own documented API (`new AudioModule.AudioRecorder(...)`, see its README);
    // the import plugin's TS resolver doesn't see `AudioRecorder` as a member of the
    // `AudioModule` native-module class shape, but tsc — which actually type-checks it —
    // resolves it fine.
    // eslint-disable-next-line import/namespace
    recorder = new AudioModule.AudioRecorder({ ...RECORDING_OPTIONS, isMeteringEnabled: true });
    await recorder.prepareToRecordAsync();
    recorder.record();

    stopPolling();
    pollId = setInterval(() => {
      const status = recorder?.getStatus();
      onLevel(dbToRms01(status?.metering ?? METER_FLOOR_DB));
    }, POLL_MS);
  },

  async stop() {
    stopPolling();
    if (!recorder) throw new Error("not recording");
    await recorder.stop();
    // Deliberately NOT releasing the recorder here. `release()` detaches the JS object from
    // its native AVAudioRecorder/MediaRecorder counterpart (expo-modules-core's SharedObject) —
    // the recorder is the one holding the native file open, and releasing it before the caller
    // has uploaded `uri` can invalidate that file out from under an in-flight RN FormData read.
    // The caller must call `dispose()` once the upload (or discard) is done.
    let uri = recorder.uri;
    if (!uri) throw new Error("recording produced no audio");
    if (!/^[a-z][a-z0-9+.-]*:/i.test(uri)) uri = `file://${uri}`;
    const ext = /\.([a-z0-9]+)$/i.exec(uri)?.[1]?.toLowerCase() ?? "m4a";
    const name = `voice.${ext}`;
    const mime = MIME_BY_EXT[ext] ?? "audio/m4a";
    // Expo's WinterCG `fetch` (native's global fetch) rejects RN's `{uri,name,type}` FormData
    // shorthand outright — its multipart encoder only accepts a string, a real `Blob`, or an
    // object with `bytes(): Promise<Uint8Array>` (expo/src/winter/fetch/convertFormData.ts).
    // A plain adapter satisfies that contract directly, instead of relying on `File`'s own
    // `.name`/`.type` getters, which reflect the on-disk file and not our derived `name`/`mime`.
    const file = new File(uri);
    return {
      blobOrFile: { bytes: () => file.bytes(), name, type: mime },
      name,
      mime,
    };
  },

  async cancel() {
    if (recorder) {
      try {
        await recorder.stop();
      } catch {
        // best-effort — recorder may already be invalid, we're discarding either way
      }
    }
    teardown();
  },

  dispose() {
    teardown();
  },
};
