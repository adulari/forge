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

/// Gap between meter samples on the portable Linux path. The [`RecordingHandle::levels`] contract
/// promises roughly 30 updates/sec, which is also about one TUI frame — polling finer would only
/// re-read the growing file more often for the same waveform.
#[cfg(all(target_os = "linux", not(feature = "microphone")))]
const LEVEL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(33);

/// Byte offset of the `data` chunk's payload in a RIFF/WAVE stream, or `None` while the header is
/// still too short to name it. pw-record and arecord both happen to write the canonical 44-byte
/// header, but neither promises it — `LIST`/`fact` chunks are legal before `data` — so walk the
/// chunk list instead of assuming a fixed size.
#[cfg(all(target_os = "linux", not(feature = "microphone")))]
fn wav_data_offset(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut at = 12;
    while at + 8 <= bytes.len() {
        if &bytes[at..at + 4] == b"data" {
            return Some(at + 8);
        }
        let size = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]])
            as usize;
        // Chunks are word-aligned: an odd payload is followed by a pad byte.
        at += 8 + size + (size & 1);
    }
    None
}

/// A read cursor over the WAV the recorder helper is still writing, so each poll measures only the
/// frames appended since the last one instead of the whole recording.
///
/// pw-record/arecord hand us no sample stream — the file IS the capture — so the meter is derived
/// from the very bytes that will be transcribed. A flat waveform therefore means no audio is
/// reaching the file, which is the thing a user needs to be told.
#[cfg(all(target_os = "linux", not(feature = "microphone")))]
#[derive(Default)]
struct WavLevelTail {
    file: Option<std::fs::File>,
    /// Absolute offset already measured. Stays 0 until the header names the `data` payload, so a
    /// header that is still being written is simply re-read on the next poll.
    pos: u64,
    in_data: bool,
}

#[cfg(all(target_os = "linux", not(feature = "microphone")))]
impl WavLevelTail {
    /// RMS of the frames appended since the previous call, or `None` when there are none yet.
    /// Assumes the mono s16 the backends are launched with (see [`linux_record_thread`]).
    fn poll(&mut self, wav: &std::path::Path) -> Option<f32> {
        use std::io::{Read, Seek, SeekFrom};
        // 32 KiB is one second of 16 kHz mono s16 — a poll that gets descheduled catches up on the
        // next one rather than falling permanently behind the recorder.
        let mut buf = [0u8; 32 * 1024];
        if self.file.is_none() {
            self.file = std::fs::File::open(wav).ok();
        }
        let file = self.file.as_mut()?;
        file.seek(SeekFrom::Start(self.pos)).ok()?;
        let read = file.read(&mut buf).ok()?;
        let fresh = if self.in_data {
            &buf[..read]
        } else {
            let start = wav_data_offset(&buf[..read])?;
            self.in_data = true;
            self.pos = start as u64;
            &buf[start..read]
        };
        // An odd trailing byte is half a sample: leave it for the next poll to pair up.
        let whole = fresh.len() & !1;
        if whole == 0 {
            return None;
        }
        self.pos += whole as u64;
        // Scale by 2^15 to match `decode`'s integer path, so the bars mean the same thing as the
        // samples whisper ends up seeing (and the same as the cpal backend's meter).
        let samples: Vec<f32> = fresh[..whole]
            .chunks_exact(2)
            .map(|s| f32::from(i16::from_le_bytes([s[0], s[1]])) / 32_768.0)
            .collect();
        Some(rms(&samples))
    }
}

/// Block until the handle stops or cancels the recording, publishing a level for whatever the helper
/// appended to `wav` in the meantime. `None` means the handle was dropped without either, which
/// [`linux_record_thread`] treats like a cancel.
#[cfg(all(target_os = "linux", not(feature = "microphone")))]
fn wait_for_stop(
    cmd_rx: &std::sync::mpsc::Receiver<Cmd>,
    wav: &std::path::Path,
    level_tx: &watch::Sender<f32>,
) -> Option<Cmd> {
    let mut tail = WavLevelTail::default();
    loop {
        match cmd_rx.recv_timeout(LEVEL_POLL_INTERVAL) {
            Ok(cmd) => return Some(cmd),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return None,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if let Some(level) = tail.poll(wav) {
                    let _ = level_tx.send(level);
                }
            }
        }
    }
}

/// How long to keep retrying a spawn that fails `ETXTBSY`, and how long to wait between tries.
/// Short: the holder of the write descriptor is always about to close it, so anything still busy
/// after this is a genuine failure rather than a race worth waiting on.
#[cfg(all(target_os = "linux", not(feature = "microphone")))]
const BUSY_RETRY_WINDOW: std::time::Duration = std::time::Duration::from_millis(200);
#[cfg(all(target_os = "linux", not(feature = "microphone")))]
const BUSY_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);

/// Spawn, retrying briefly while the OS reports the program as busy.
///
/// `ETXTBSY` means something still holds a writable descriptor for the executable, so the kernel
/// refuses to exec it. It is transient by nature and has two real causes here: a package manager or
/// installer rewriting `pw-record`/`arecord` while Forge starts one, and — inside the test suite —
/// a concurrent fork inheriting the descriptor of a fake recorder the tests just wrote. Failing the
/// recording outright for a condition that clears in microseconds is the wrong trade; the user sees
/// "starting pw-record: Text file busy" and loses the take.
#[cfg(all(target_os = "linux", not(feature = "microphone")))]
fn spawn_retrying_while_busy(
    command: &mut std::process::Command,
) -> std::io::Result<std::process::Child> {
    let deadline = std::time::Instant::now() + BUSY_RETRY_WINDOW;
    loop {
        match command.spawn() {
            Err(error)
                if error.kind() == std::io::ErrorKind::ExecutableFileBusy
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(BUSY_RETRY_INTERVAL);
            }
            other => return other,
        }
    }
}

#[cfg(all(target_os = "linux", not(feature = "microphone")))]
fn linux_record_thread(
    backend: LinuxBackend,
    level_tx: watch::Sender<f32>,
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
        let mut child = spawn_retrying_while_busy(&mut command).map_err(|error| {
            VoiceError::Record(format!("starting {}: {error}", backend.program.display()))
        })?;
        let keep = matches!(wait_for_stop(&cmd_rx, &wav, &level_tx), Some(Cmd::Stop));
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
/// in -1.0..1.0 range). Shared by both capture backends so a given bar height means the same
/// loudness whichever one is recording.
#[cfg(any(feature = "microphone", target_os = "linux"))]
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

    /// A held writable descriptor makes exec fail `ETXTBSY` deterministically — the same condition a
    /// concurrent fork or a package manager rewriting the recorder produces by accident. The spawn
    /// must ride it out rather than losing the take, and must still surface a real failure.
    #[cfg(all(target_os = "linux", not(feature = "microphone")))]
    #[test]
    fn a_busy_recorder_binary_is_retried_not_surfaced() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let program = root.path().join("pw-record");
        let mut handle = std::fs::File::create(&program).unwrap();
        {
            use std::io::Write;
            handle.write_all(b"#!/bin/sh\nexit 0\n").unwrap();
            handle.flush().unwrap();
        }
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();

        // Still holding the write descriptor: exec is refused right now.
        let blocked = std::process::Command::new(&program).spawn();
        assert_eq!(
            blocked.err().map(|e| e.kind()),
            Some(std::io::ErrorKind::ExecutableFileBusy),
            "the setup must actually reproduce ETXTBSY, or this test proves nothing"
        );

        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(40));
            drop(handle);
        });
        let mut command = std::process::Command::new(&program);
        spawn_retrying_while_busy(&mut command)
            .expect("the spawn must retry until the descriptor closes")
            .wait()
            .unwrap();

        // A binary that is missing rather than busy still fails immediately.
        let mut missing = std::process::Command::new(root.path().join("absent"));
        assert!(spawn_retrying_while_busy(&mut missing).is_err());
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

    #[cfg(all(target_os = "linux", not(feature = "microphone")))]
    #[test]
    fn wav_data_offset_walks_past_extra_chunks() {
        let mut wav = b"RIFF\x00\x00\x00\x00WAVE".to_vec();
        wav.extend_from_slice(b"fmt \x10\x00\x00\x00");
        wav.extend_from_slice(&[0u8; 16]);
        // An odd-sized LIST chunk plus its pad byte must not throw the walk off by one.
        wav.extend_from_slice(b"LIST\x03\x00\x00\x00abc\x00");
        let header = wav.len();
        wav.extend_from_slice(b"data\x00\x00\x00\x00");
        wav.extend_from_slice(&[1u8, 2, 3, 4]);
        assert_eq!(wav_data_offset(&wav), Some(header + 8));
        // A header that is still being written yields nothing rather than a wrong offset.
        assert_eq!(wav_data_offset(&wav[..header]), None);
        assert_eq!(wav_data_offset(b"RIFF"), None);
        assert_eq!(wav_data_offset(b"not a wav at all"), None);
    }

    #[cfg(all(target_os = "linux", not(feature = "microphone")))]
    #[test]
    fn wav_level_tail_measures_only_newly_appended_frames() {
        use std::io::Write;
        let root = tempfile::tempdir().unwrap();
        let wav = root.path().join("growing.wav");
        let mut tail = WavLevelTail::default();
        // Nothing to read yet: no file, then a header with no frames behind it.
        assert_eq!(tail.poll(&wav), None);
        let mut file = std::fs::File::create(&wav).unwrap();
        file.write_all(b"RIFF\x00\x00\x00\x00WAVEfmt \x10\x00\x00\x00")
            .unwrap();
        file.write_all(&[0u8; 16]).unwrap();
        file.write_all(b"data\x00\x00\x00\x00").unwrap();
        file.flush().unwrap();
        assert_eq!(tail.poll(&wav), None, "header alone carries no audio");

        // Full-scale square wave → RMS 1.0; a second poll sees nothing new.
        for _ in 0..64 {
            file.write_all(&i16::MIN.to_le_bytes()).unwrap();
            file.write_all(&i16::MAX.to_le_bytes()).unwrap();
        }
        file.flush().unwrap();
        let loud = tail.poll(&wav).expect("frames appended");
        assert!(loud > 0.9, "full-scale audio reads near 1.0, got {loud}");
        assert_eq!(
            tail.poll(&wav),
            None,
            "already-measured frames aren't reread"
        );

        // Silence afterwards must pull the meter back down, not average with the loud window.
        file.write_all(&[0u8; 256]).unwrap();
        file.flush().unwrap();
        assert_eq!(tail.poll(&wav), Some(0.0));

        // A frame split across two polls is carried, not counted as a half sample.
        file.write_all(&[0xff]).unwrap();
        file.flush().unwrap();
        assert_eq!(tail.poll(&wav), None, "half a sample is not a frame");
        file.write_all(&[0x7f]).unwrap();
        file.flush().unwrap();
        let split = tail.poll(&wav).expect("the split frame completes");
        assert!(
            split > 0.9,
            "carried bytes form one full-scale sample: {split}"
        );
    }

    /// The portable backend's meter is wired end-to-end: `levels` must move while the helper writes,
    /// which is what the `/voice` waveform reads. It was bound as `_level_tx` and never published,
    /// leaving the overlay's waveform flat for the whole recording.
    #[cfg(all(target_os = "linux", not(feature = "microphone")))]
    #[test]
    fn portable_linux_recorder_publishes_live_levels() {
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
        // The fake writes the fixture immediately, so the first polls see it; bounded so a broken
        // meter fails instead of hanging.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut level = 0.0f32;
        while level == 0.0 && std::time::Instant::now() < deadline {
            level = *handle.levels.borrow();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            level > 0.0,
            "meter stayed flat while audio was being written"
        );
        let samples = handle.stop().unwrap();
        assert_eq!(samples.len(), 4, "levels must not consume the recording");
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
