// Web + Tauri desktop webview voice capture — DESIGN.md "Client audio formats": deliberately
// NOT MediaRecorder, which produces webm/opus that the server's symphonia decoder can't read.
// Captures raw PCM via getUserMedia + WebAudio, downsamples to 16kHz mono, and encodes a WAV
// Blob client-side instead. ScriptProcessorNode (not an AudioWorklet) is used on purpose: an
// AudioWorklet needs its processor module served from a fetchable URL, which Metro's static web
// export and the Tauri-bundled dist don't provide a clean seam for, and the deprecated-but-
// universal ScriptProcessorNode works identically across Chrome/Safari/Firefox/WebKitGTK/WebView2.
import type { VoiceRecorder } from "./types";

const TARGET_SAMPLE_RATE = 16000;
const BUFFER_SIZE = 4096;
// Raw PCM RMS for typical speech sits well under 1.0 (full scale is a rare clipped peak) — this
// gain brings normal talking volume up into a visually useful 0..1 range for the amplitude bars.
const LEVEL_GAIN = 4;

let audioContext: AudioContext | null = null;
let stream: MediaStream | null = null;
let source: MediaStreamAudioSourceNode | null = null;
let processor: ScriptProcessorNode | null = null;
let silence: GainNode | null = null;
let chunks: Float32Array[] = [];
let inputSampleRate = 48000;

function cleanup(): void {
  processor?.disconnect();
  source?.disconnect();
  silence?.disconnect();
  stream?.getTracks().forEach((t) => t.stop());
  void audioContext?.close().catch(() => {});
  processor = null;
  source = null;
  silence = null;
  stream = null;
  audioContext = null;
  chunks = [];
}

function rmsOf(buf: Float32Array): number {
  let sum = 0;
  for (let i = 0; i < buf.length; i++) sum += buf[i] * buf[i];
  return Math.sqrt(sum / buf.length);
}

// Linear-interpolation resample — good enough for speech-to-text input. A proper windowed-sinc
// resampler would sound better for music but adds real complexity for no benefit at whisper's
// quality floor.
function resampleLinear(input: Float32Array, fromRate: number, toRate: number): Float32Array {
  if (fromRate === toRate) return input;
  const ratio = fromRate / toRate;
  const outLength = Math.max(1, Math.round(input.length / ratio));
  const out = new Float32Array(outLength);
  for (let i = 0; i < outLength; i++) {
    const srcIndex = i * ratio;
    const i0 = Math.floor(srcIndex);
    const i1 = Math.min(i0 + 1, input.length - 1);
    const frac = srcIndex - i0;
    out[i] = input[i0] * (1 - frac) + input[i1] * frac;
  }
  return out;
}

function encodeWav16kMono(samples: Float32Array, sampleRate: number): Blob {
  const bytesPerSample = 2;
  const buffer = new ArrayBuffer(44 + samples.length * bytesPerSample);
  const view = new DataView(buffer);

  const writeString = (offset: number, str: string) => {
    for (let i = 0; i < str.length; i++) view.setUint8(offset + i, str.charCodeAt(i));
  };

  writeString(0, "RIFF");
  view.setUint32(4, 36 + samples.length * bytesPerSample, true);
  writeString(8, "WAVE");
  writeString(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true); // PCM
  view.setUint16(22, 1, true); // mono
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * bytesPerSample, true);
  view.setUint16(32, bytesPerSample, true);
  view.setUint16(34, 16, true); // bits per sample
  writeString(36, "data");
  view.setUint32(40, samples.length * bytesPerSample, true);

  let offset = 44;
  for (let i = 0; i < samples.length; i++) {
    const s = Math.max(-1, Math.min(1, samples[i]));
    view.setInt16(offset, s < 0 ? s * 0x8000 : s * 0x7fff, true);
    offset += bytesPerSample;
  }

  return new Blob([buffer], { type: "audio/wav" });
}

export const voice: VoiceRecorder = {
  isSupported() {
    return (
      typeof navigator !== "undefined" &&
      !!navigator.mediaDevices?.getUserMedia &&
      typeof AudioContext !== "undefined"
    );
  },

  async start(onLevel) {
    stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    audioContext = new AudioContext();
    inputSampleRate = audioContext.sampleRate;
    source = audioContext.createMediaStreamSource(stream);
    processor = audioContext.createScriptProcessor(BUFFER_SIZE, 1, 1);
    chunks = [];

    processor.onaudioprocess = (e) => {
      const input = e.inputBuffer.getChannelData(0);
      chunks.push(new Float32Array(input));
      onLevel(Math.min(1, rmsOf(input) * LEVEL_GAIN));
    };

    // ScriptProcessorNode only fires `onaudioprocess` while connected through to a destination —
    // route through a silent gain node so nothing is audibly played back while recording.
    silence = audioContext.createGain();
    silence.gain.value = 0;
    source.connect(processor);
    processor.connect(silence);
    silence.connect(audioContext.destination);
  },

  async stop() {
    if (!audioContext || !processor) throw new Error("not recording");
    const sampleRate = inputSampleRate;
    const collected = chunks;
    cleanup();

    const totalLength = collected.reduce((n, c) => n + c.length, 0);
    const merged = new Float32Array(totalLength);
    let offset = 0;
    for (const c of collected) {
      merged.set(c, offset);
      offset += c.length;
    }

    const resampled = resampleLinear(merged, sampleRate, TARGET_SAMPLE_RATE);
    const blob = encodeWav16kMono(resampled, TARGET_SAMPLE_RATE);
    return { blobOrFile: blob, name: "voice.wav", mime: "audio/wav" };
  },

  async cancel() {
    cleanup();
  },
};
