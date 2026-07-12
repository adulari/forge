//! Microphone capture (cpal) -> 16kHz mono f32, with a live RMS level feed for UI meters.

use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tokio::sync::watch;

use crate::{Result, VoiceError};

/// Commands sent from [`RecordingHandle`] to the capture thread.
enum Cmd {
    /// Stop capturing and return the recorded (resampled, mono) samples.
    Stop,
    /// Stop capturing and discard whatever was recorded.
    Cancel,
}

/// A live recording in progress. `levels` publishes an approximate RMS amplitude (0..1) roughly
/// 30x/sec, for a waveform/meter UI. The actual microphone capture happens on a dedicated OS
/// thread (cpal's `Stream` is not `Send`), so this handle is a thin, `Send`-safe remote control
/// for it.
pub struct RecordingHandle {
    /// Live RMS level (0..1), updated as audio arrives. Cheap to poll — a `watch` channel only
    /// ever holds the latest value.
    pub levels: watch::Receiver<f32>,
    cmd_tx: std::sync::mpsc::Sender<Cmd>,
    done_rx: std::sync::mpsc::Receiver<Result<Vec<f32>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl RecordingHandle {
    /// Stop capturing and return the recorded audio as 16kHz mono f32 samples.
    pub fn stop(mut self) -> Result<Vec<f32>> {
        let _ = self.cmd_tx.send(Cmd::Stop);
        let result = self.done_rx.recv().unwrap_or(Ok(Vec::new()));
        self.join();
        result
    }

    /// Stop capturing and discard the recording — no partial transcript, no leftover state.
    pub fn cancel(mut self) {
        let _ = self.cmd_tx.send(Cmd::Cancel);
        self.join();
    }

    fn join(&mut self) {
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Starts/stops microphone recordings. Stateless — every [`Recorder::start`] call spins up its
/// own capture thread.
pub struct Recorder;

impl Recorder {
    /// Start recording from the default input device. Returns immediately with a handle whose
    /// `levels` receiver starts updating as soon as the device is open.
    pub fn start() -> Result<RecordingHandle> {
        let (level_tx, level_rx) = watch::channel(0.0f32);
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Cmd>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<Result<Vec<f32>>>();

        // cpal's `Stream` holds platform audio handles that are not `Send`; it must be built and
        // dropped on the same thread. That thread also owns the raw sample buffer the audio
        // callback (itself running on a cpal-managed realtime thread) writes into.
        let thread = std::thread::Builder::new()
            .name("forge-voice-record".to_string())
            .spawn(move || record_thread(level_tx, cmd_rx, done_tx))
            .map_err(|e| VoiceError::Record(format!("spawning capture thread: {e}")))?;

        Ok(RecordingHandle {
            levels: level_rx,
            cmd_tx,
            done_rx,
            thread: Some(thread),
        })
    }
}

/// Target sample rate whisper.cpp expects.
const WHISPER_SAMPLE_RATE: u32 = 16_000;

fn record_thread(
    level_tx: watch::Sender<f32>,
    cmd_rx: std::sync::mpsc::Receiver<Cmd>,
    done_tx: std::sync::mpsc::Sender<Result<Vec<f32>>>,
) {
    let outcome = (|| -> Result<(Vec<f32>, u32)> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or(VoiceError::NoInputDevice)?;
        let supported = device
            .default_input_config()
            .map_err(|e| VoiceError::Record(format!("querying input device: {e}")))?;
        let sample_format = supported.sample_format();
        let stream_config: cpal::StreamConfig = supported.config();
        let channels = stream_config.channels as usize;
        let device_rate = stream_config.sample_rate;

        let buf: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
        let err_fn = |err: cpal::Error| {
            // The callback has no way to report this synchronously; surfacing it as a partial
            // recording is friendlier than crashing the capture thread mid-session.
            eprintln!("forge-voice: input stream error: {err}");
        };

        let stream = match sample_format {
            cpal::SampleFormat::F32 => {
                let buf = buf.clone();
                let level_tx = level_tx.clone();
                device.build_input_stream(
                    stream_config,
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        on_input(data, channels, &buf, &level_tx)
                    },
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let buf = buf.clone();
                let level_tx = level_tx.clone();
                device.build_input_stream(
                    stream_config,
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        let floats: Vec<f32> =
                            data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                        on_input(&floats, channels, &buf, &level_tx)
                    },
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::U16 => {
                let buf = buf.clone();
                let level_tx = level_tx.clone();
                device.build_input_stream(
                    stream_config,
                    move |data: &[u16], _: &cpal::InputCallbackInfo| {
                        let floats: Vec<f32> = data
                            .iter()
                            .map(|&s| (s as f32 - 32_768.0) / 32_768.0)
                            .collect();
                        on_input(&floats, channels, &buf, &level_tx)
                    },
                    err_fn,
                    None,
                )
            }
            other => {
                return Err(VoiceError::Record(format!(
                    "unsupported input sample format: {other:?}"
                )))
            }
        }
        .map_err(|e| VoiceError::Record(format!("opening input stream: {e}")))?;

        stream
            .play()
            .map_err(|e| VoiceError::Record(format!("starting input stream: {e}")))?;

        // Block until the handle tells us to stop/cancel, or is dropped without either (treated
        // like cancel: stop capturing and discard).
        let keep = matches!(cmd_rx.recv(), Ok(Cmd::Stop));
        drop(stream); // stops capture

        let raw = std::mem::take(&mut *buf.lock().unwrap_or_else(|p| p.into_inner()));
        if keep {
            Ok((raw, device_rate))
        } else {
            Ok((Vec::new(), device_rate))
        }
    })();

    let result =
        outcome.map(|(mono, device_rate)| resample_linear(&mono, device_rate, WHISPER_SAMPLE_RATE));
    let _ = done_tx.send(result);
}

/// Downmix an interleaved multi-channel buffer to mono by averaging each frame's channels, then
/// publish an RMS level and append to the shared recording buffer. Runs on cpal's realtime audio
/// callback thread — no allocation beyond the per-callback downmix buffer, no locking beyond the
/// single buffer append.
fn on_input(data: &[f32], channels: usize, buf: &Mutex<Vec<f32>>, level_tx: &watch::Sender<f32>) {
    let mono = downmix(data, channels);
    let level = rms(&mono);
    let _ = level_tx.send(level);
    if let Ok(mut b) = buf.lock() {
        b.extend_from_slice(&mono);
    }
}

/// Average interleaved `channels`-wide frames down to mono. A no-op copy when already mono.
/// `pub(crate)` so [`decode`](crate::decode) can reuse it for file-based stereo downmix.
pub(crate) fn downmix(data: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return data.to_vec();
    }
    data.chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect()
}

/// Root-mean-square amplitude of `samples`, clamped to 0..1 (samples are expected to already be
/// in -1.0..1.0 range).
fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt().min(1.0)
}

/// Linear-interpolation resampler shared by [`record`](crate::record) and
/// [`decode`](crate::decode). Not a high-quality resampler (no anti-aliasing filter) — fine for
/// speech-to-text, where whisper.cpp itself is far more forgiving than the quality bar for
/// music/production audio.
pub fn resample_linear(input: &[f32], from_hz: u32, to_hz: u32) -> Vec<f32> {
    if input.is_empty() || from_hz == 0 || to_hz == 0 {
        return Vec::new();
    }
    if from_hz == to_hz {
        return input.to_vec();
    }
    let ratio = from_hz as f64 / to_hz as f64;
    let out_len = ((input.len() as f64) / ratio).round().max(1.0) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_pos = i as f64 * ratio;
        let idx0 = src_pos.floor() as usize;
        if idx0 >= input.len() {
            break;
        }
        let frac = (src_pos - idx0 as f64) as f32;
        let s0 = input[idx0];
        let s1 = input.get(idx0 + 1).copied().unwrap_or(s0);
        out.push(s0 + (s1 - s0) * frac);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_passthrough_when_rates_match() {
        let input = vec![0.1, 0.2, 0.3, 0.4];
        assert_eq!(resample_linear(&input, 16_000, 16_000), input);
    }

    #[test]
    fn resample_downsamples_by_half() {
        let input: Vec<f32> = (0..1000).map(|i| i as f32).collect();
        let out = resample_linear(&input, 32_000, 16_000);
        // Half the sample rate => roughly half the samples.
        assert!(
            out.len() >= 490 && out.len() <= 510,
            "expected ~500 samples, got {}",
            out.len()
        );
        // Downsampling by exactly 2x with linear interpolation lands exactly on source samples.
        assert_eq!(out[0], 0.0);
        assert_eq!(out[10], 20.0);
    }

    #[test]
    fn resample_upsamples_by_double() {
        let input = vec![0.0, 10.0, 20.0];
        let out = resample_linear(&input, 8_000, 16_000);
        assert!(out.len() >= 5 && out.len() <= 7, "got {}", out.len());
        assert_eq!(out[0], 0.0);
    }

    #[test]
    fn resample_empty_input_is_empty() {
        assert!(resample_linear(&[], 16_000, 16_000).is_empty());
        assert!(resample_linear(&[1.0, 2.0], 0, 16_000).is_empty());
    }

    #[test]
    fn downmix_stereo_averages_channels() {
        // L, R, L, R
        let stereo = vec![1.0, -1.0, 0.5, 0.5];
        let mono = downmix(&stereo, 2);
        assert_eq!(mono, vec![0.0, 0.5]);
    }

    #[test]
    fn downmix_mono_is_passthrough() {
        let mono_in = vec![0.1, 0.2, 0.3];
        assert_eq!(downmix(&mono_in, 1), mono_in);
    }

    #[test]
    fn rms_of_silence_is_zero() {
        assert_eq!(rms(&[0.0, 0.0, 0.0]), 0.0);
    }

    #[test]
    fn rms_of_full_scale_is_one() {
        assert_eq!(rms(&[1.0, -1.0, 1.0, -1.0]), 1.0);
    }
}
