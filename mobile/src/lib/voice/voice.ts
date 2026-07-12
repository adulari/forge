// Native (iOS/Android) voice capture — DESIGN.md "Client audio formats": records m4a/aac via
// expo-audio's HIGH_QUALITY preset (m4a container on both platforms), which the server decodes
// with symphonia. `AudioModule.AudioRecorder` is used directly (not the `useAudioRecorder` hook)
// because this module's start/stop/cancel lifecycle is driven by VoiceRecordingPill, not tied to
// a single component's mount lifecycle.
import { AudioModule, RecordingPresets, requestRecordingPermissionsAsync, setAudioModeAsync } from "expo-audio";
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
    recorder = new AudioModule.AudioRecorder({ ...RecordingPresets.HIGH_QUALITY, isMeteringEnabled: true });
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
    const uri = recorder.uri;
    recorder.release();
    recorder = null;
    if (!uri) throw new Error("recording produced no audio");
    return {
      blobOrFile: { uri, name: "voice.m4a", type: "audio/m4a" },
      name: "voice.m4a",
      mime: "audio/m4a",
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
};
