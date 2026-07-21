//! Microphone capture (cpal) -> 16kHz mono f32, with a live RMS level feed for UI meters.

#[cfg(feature = "microphone")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "microphone")]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tokio::sync::watch;

use crate::{Result, VoiceError};

/// Commands sent from [`RecordingHandle`] to the capture thread.
#[cfg(any(feature = "microphone", target_os = "linux"))]
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
#[cfg(any(feature = "microphone", target_os = "linux"))]
pub struct RecordingHandle {
    /// Live RMS level (0..1), updated as audio arrives. Cheap to poll — a `watch` channel only
    /// ever holds the latest value.
    pub levels: watch::Receiver<f32>,
    cmd_tx: std::sync::mpsc::Sender<Cmd>,
    done_rx: std::sync::mpsc::Receiver<Result<Vec<f32>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(any(feature = "microphone", target_os = "linux"))]
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

/// Placeholder handle used when this crate was built without a microphone backend. A value of
/// this type cannot be produced by [`Recorder::start`]; retaining the type keeps callers portable
/// while the start operation reports a normal, actionable error at runtime.
#[cfg(all(not(feature = "microphone"), not(target_os = "linux")))]
#[non_exhaustive]
pub struct RecordingHandle {
    /// Kept API-compatible with microphone-enabled handles. No receiver is created because
    /// [`Recorder::start`] always returns an error in this build.
    pub levels: watch::Receiver<f32>,
}

#[cfg(all(not(feature = "microphone"), not(target_os = "linux")))]
impl RecordingHandle {
    /// Return the same graceful unavailability error as [`Recorder::start`].
    pub fn stop(self) -> Result<Vec<f32>> {
        Err(VoiceError::MicrophoneUnavailable)
    }

    /// No-op for API compatibility; an unavailable build cannot create a live recording.
    pub fn cancel(self) {}
}

/// Starts/stops microphone recordings. Stateless — every [`Recorder::start`] call spins up its
/// own capture thread.
pub struct Recorder;

impl Recorder {
    /// Whether this build contains a local microphone capture backend.
    ///
    /// This is a compile-time capability check; it does not probe for an attached input device or
    /// operating-system permission. Call [`Recorder::start`] to perform those runtime checks.
    #[must_use]
    pub const fn is_supported() -> bool {
        cfg!(any(feature = "microphone", target_os = "linux"))
    }

    /// Start recording from the default input device. Returns immediately with a handle whose
    /// `levels` receiver starts updating as soon as the device is open.
    #[cfg(feature = "microphone")]
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

    /// Portable Linux builds use the system PipeWire recorder (or ALSA's `arecord`) at runtime,
    /// avoiding a hard link on libasound for every Forge command.
    #[cfg(all(target_os = "linux", not(feature = "microphone")))]
    pub fn start() -> Result<RecordingHandle> {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let backend = std::env::split_paths(&path)
            .find_map(|directory| select_linux_backend(&directory).ok())
            .ok_or_else(linux_backend_unavailable)?;
        start_linux_backend(backend)
    }

    /// Report that microphone capture was not included in this build.
    #[cfg(all(not(feature = "microphone"), not(target_os = "linux")))]
    pub fn start() -> Result<RecordingHandle> {
        Err(VoiceError::MicrophoneUnavailable)
    }
}

/// Target sample rate whisper.cpp expects.
#[cfg(any(feature = "microphone", test))]
const WHISPER_SAMPLE_RATE: u32 = 16_000;

#[cfg(feature = "microphone")]
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

#[cfg(all(target_os = "linux", not(feature = "microphone")))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinuxBackendKind {
    PipeWire,
    Alsa,
}

#[cfg(all(target_os = "linux", not(feature = "microphone")))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct LinuxBackend {
    kind: LinuxBackendKind,
    program: std::path::PathBuf,
}

#[cfg(all(target_os = "linux", not(feature = "microphone")))]
fn linux_backend_unavailable() -> VoiceError {
    VoiceError::Record(
        "no Linux microphone recorder found; install PipeWire tools (`pw-record`) or ALSA tools (`arecord`) and try again"
            .into(),
    )
}

#[cfg(all(target_os = "linux", not(feature = "microphone")))]
fn select_linux_backend(directory: &std::path::Path) -> Result<LinuxBackend> {
    use std::os::unix::fs::PermissionsExt;
    let executable = |path: &std::path::Path| {
        path.metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    };
    let pipewire = directory.join("pw-record");
    if executable(&pipewire) {
        return Ok(LinuxBackend {
            kind: LinuxBackendKind::PipeWire,
            program: pipewire,
        });
    }
    let alsa = directory.join("arecord");
    if executable(&alsa) {
        return Ok(LinuxBackend {
            kind: LinuxBackendKind::Alsa,
            program: alsa,
        });
    }
    Err(linux_backend_unavailable())
}

#[cfg(all(target_os = "linux", not(feature = "microphone")))]
fn start_linux_backend(backend: LinuxBackend) -> Result<RecordingHandle> {
    let (level_tx, level_rx) = watch::channel(0.0f32);
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Cmd>();
    let (done_tx, done_rx) = std::sync::mpsc::channel::<Result<Vec<f32>>>();
    let thread = std::thread::Builder::new()
        .name("forge-voice-record".to_string())
        .spawn(move || linux_record_thread(backend, level_tx, cmd_rx, done_tx))
        .map_err(|error| VoiceError::Record(format!("spawning capture thread: {error}")))?;
    Ok(RecordingHandle {
        levels: level_rx,
        cmd_tx,
        done_rx,
        thread: Some(thread),
    })
}

#[cfg(all(target_os = "linux", not(feature = "microphone")))]
fn linux_record_thread(
    backend: LinuxBackend,
    _level_tx: watch::Sender<f32>,
    cmd_rx: std::sync::mpsc::Receiver<Cmd>,
    done_tx: std::sync::mpsc::Sender<Result<Vec<f32>>>,
) {
    static RECORDING_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = RECORDING_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let wav = std::env::temp_dir().join(format!("forge-voice-{}-{id}.wav", std::process::id()));
    let mut command = std::process::Command::new(&backend.program);
    match backend.kind {
        LinuxBackendKind::PipeWire => {
            command.args([
                "--rate",
                "16000",
                "--channels",
                "1",
                "--format",
                "s16",
                "--container",
                "wav",
            ]);
        }
        LinuxBackendKind::Alsa => {
            command.args([
                "--quiet",
                "--file-type",
                "wav",
                "--channels",
                "1",
                "--format",
                "S16_LE",
                "--rate",
                "16000",
            ]);
        }
    }
    command
        .arg(&wav)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    let result = (|| -> Result<Vec<f32>> {
        let mut child = command.spawn().map_err(|error| {
            VoiceError::Record(format!("starting {}: {error}", backend.program.display()))
        })?;
        let keep = matches!(cmd_rx.recv(), Ok(Cmd::Stop));
        // A very fast stop/cancel can arrive while the helper is still exec'ing. Give it a
        // bounded chance to open the output so SIGINT can finalize a valid WAV, while also
        // detecting permission/device failures that exit before recording starts.
        let startup_deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while !wav.exists()
            && child.try_wait().map_err(VoiceError::Io)?.is_none()
            && std::time::Instant::now() < startup_deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let already_finished = child.try_wait().map_err(VoiceError::Io)?;
        if already_finished.is_none() {
            // SIGINT lets pw-record/arecord finish the WAV header. Reap within a bounded window;
            // a broken recorder process must not leave the TUI stuck in Transcribing forever.
            unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) };
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while child.try_wait().map_err(VoiceError::Io)?.is_none()
                && std::time::Instant::now() < deadline
            {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            if child.try_wait().map_err(VoiceError::Io)?.is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
        } else if !already_finished.is_some_and(|status| status.success()) {
            return Err(VoiceError::Record(format!(
                "{} exited before recording completed; check microphone permissions and the default input device",
                backend.program.display()
            )));
        }
        if !keep {
            return Ok(Vec::new());
        }
        let bytes = std::fs::read(&wav).map_err(|error| {
            VoiceError::Record(format!(
                "reading captured audio from {}: {error}",
                wav.display()
            ))
        })?;
        crate::decode_audio(&bytes, Some("wav"))
    })();
    let _ = std::fs::remove_file(&wav);
    let _ = done_tx.send(result);
}

/// Downmix an interleaved multi-channel buffer to mono by averaging each frame's channels, then
/// publish an RMS level and append to the shared recording buffer. Runs on cpal's realtime audio
/// callback thread — no allocation beyond the per-callback downmix buffer, no locking beyond the
/// single buffer append.
#[cfg(feature = "microphone")]
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
#[cfg(feature = "microphone")]
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
    fn recorder_reports_compile_time_capability() {
        assert_eq!(
            Recorder::is_supported(),
            cfg!(any(feature = "microphone", target_os = "linux"))
        );
    }

    #[cfg(all(not(feature = "microphone"), not(target_os = "linux")))]
    #[test]
    fn recorder_fails_gracefully_without_capture_backend() {
        match Recorder::start() {
            Err(VoiceError::MicrophoneUnavailable) => {}
            Err(other) => panic!("expected microphone-unavailable error, got {other}"),
            Ok(_) => panic!("capture-disabled build unexpectedly started a recording"),
        }
    }

    #[cfg(all(target_os = "linux", not(feature = "microphone")))]
    #[test]
    fn portable_linux_backend_prefers_pipewire_then_falls_back_to_alsa() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let pw = root.path().join("pw-record");
        let alsa = root.path().join("arecord");
        std::fs::write(&pw, "").unwrap();
        std::fs::write(&alsa, "").unwrap();
        std::fs::set_permissions(&pw, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&alsa, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(select_linux_backend(root.path()).unwrap().program, pw);
        std::fs::remove_file(&pw).unwrap();
        assert_eq!(select_linux_backend(root.path()).unwrap().program, alsa);
        std::fs::write(&pw, "").unwrap();
        assert_eq!(
            select_linux_backend(root.path()).unwrap().program,
            alsa,
            "a non-executable pw-record must not mask a working arecord"
        );
        std::fs::remove_file(&alsa).unwrap();
        std::fs::remove_file(&pw).unwrap();
        let error = select_linux_backend(root.path()).unwrap_err().to_string();
        assert!(error.contains("pw-record"), "{error}");
        assert!(error.contains("arecord"), "{error}");
    }

    #[cfg(all(target_os = "linux", not(feature = "microphone")))]
    fn write_fake_recorder(
        directory: &std::path::Path,
        name: &str,
        wav: &std::path::Path,
        pid_file: &std::path::Path,
    ) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let program = directory.join(name);
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\nfor output_path do :; done\ncp '{}' \"$output_path\"\necho $$ > '{}'\ntrap 'exit 0' INT TERM\nwhile :; do sleep 0.05; done\n",
                wav.display(),
                pid_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        program
    }

    #[cfg(all(target_os = "linux", not(feature = "microphone")))]
    fn write_test_wav(path: &std::path::Path) {
        let mut writer = hound::WavWriter::create(
            path,
            hound::WavSpec {
                channels: 1,
                sample_rate: WHISPER_SAMPLE_RATE,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for sample in [0i16, 1000, -1000, 500] {
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();
    }

    #[cfg(all(target_os = "linux", not(feature = "microphone")))]
    #[test]
    fn portable_linux_recorder_stops_decodes_and_reaps_the_process() {
        let root = tempfile::tempdir().unwrap();
        let wav = root.path().join("fixture.wav");
        let pid_file = root.path().join("pid");
        write_test_wav(&wav);
        let program = write_fake_recorder(root.path(), "pw-record", &wav, &pid_file);
        let handle = start_linux_backend(LinuxBackend {
            kind: LinuxBackendKind::PipeWire,
            program,
        })
        .unwrap();
        let samples = handle.stop().unwrap();
        assert_eq!(samples.len(), 4);
        let pid: libc::pid_t = std::fs::read_to_string(pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_ne!(
            unsafe { libc::kill(pid, 0) },
            0,
            "recorder process was not reaped"
        );
    }

    #[cfg(all(target_os = "linux", not(feature = "microphone")))]
    #[test]
    fn portable_linux_recorder_cancel_reaps_without_returning_audio() {
        let root = tempfile::tempdir().unwrap();
        let wav = root.path().join("fixture.wav");
        let pid_file = root.path().join("pid");
        write_test_wav(&wav);
        let program = write_fake_recorder(root.path(), "arecord", &wav, &pid_file);
        let handle = start_linux_backend(LinuxBackend {
            kind: LinuxBackendKind::Alsa,
            program,
        })
        .unwrap();
        handle.cancel();
        let pid: libc::pid_t = std::fs::read_to_string(pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_ne!(
            unsafe { libc::kill(pid, 0) },
            0,
            "cancel left recorder running"
        );
    }

    #[cfg(all(target_os = "linux", not(feature = "microphone")))]
    #[test]
    fn portable_linux_recorder_surfaces_early_backend_failure() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let program = root.path().join("pw-record");
        std::fs::write(&program, "#!/bin/sh\nexit 17\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let handle = start_linux_backend(LinuxBackend {
            kind: LinuxBackendKind::PipeWire,
            program,
        })
        .unwrap();
        let error = handle.stop().unwrap_err().to_string();
        assert!(
            error.contains("exited before recording completed"),
            "{error}"
        );
        assert!(error.contains("microphone permissions"), "{error}");
    }

    /// Hardware acceptance probe, intentionally opt-in for local/release verification.
    #[cfg(all(target_os = "linux", not(feature = "microphone")))]
    #[test]
    #[ignore = "requires a real Linux microphone and pw-record/arecord"]
    fn portable_linux_real_microphone_capture() {
        let handle = Recorder::start().expect("start the system microphone recorder");
        std::thread::sleep(std::time::Duration::from_secs(2));
        let samples = handle
            .stop()
            .expect("stop and decode the microphone recording");
        assert!(
            samples.len() >= WHISPER_SAMPLE_RATE as usize,
            "expected at least one second of decoded audio, got {} samples",
            samples.len()
        );
    }

    #[cfg(feature = "microphone")]
    #[test]
    fn rms_of_silence_is_zero() {
        assert_eq!(rms(&[0.0, 0.0, 0.0]), 0.0);
    }

    #[cfg(feature = "microphone")]
    #[test]
    fn rms_of_full_scale_is_one() {
        assert_eq!(rms(&[1.0, -1.0, 1.0, -1.0]), 1.0);
    }
}
