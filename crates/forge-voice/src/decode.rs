//! Decode an uploaded audio file's raw bytes into 16kHz mono f32 — whisper's input format.
//!
//! WAV is decoded via `hound` (small, exact); everything else (m4a/aac/mp4 — what mobile clients
//! actually upload) goes through `symphonia`.

use std::io::Cursor;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use crate::record::{downmix, resample_linear};
use crate::{Result, VoiceError};

const WHISPER_SAMPLE_RATE: u32 = 16_000;

/// Decode `bytes` (a whole audio file, e.g. an HTTP multipart upload body) into 16kHz mono f32
/// samples. `hint` is the uploaded file name or MIME type, if known — it steers symphonia's
/// format probe for containers that don't self-identify unambiguously; a `None` hint still works,
/// just with a slightly wider (slower) format probe.
pub fn decode_audio(bytes: &[u8], hint: Option<&str>) -> Result<Vec<f32>> {
    if is_wav(bytes) {
        decode_wav(bytes)
    } else {
        decode_with_symphonia(bytes, hint)
    }
}

fn is_wav(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE"
}

fn decode_wav(bytes: &[u8]) -> Result<Vec<f32>> {
    let mut reader = hound::WavReader::new(Cursor::new(bytes))
        .map_err(|e| VoiceError::Decode(format!("wav: {e}")))?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| VoiceError::Decode(format!("wav: {e}")))?,
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / max))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| VoiceError::Decode(format!("wav: {e}")))?
        }
    };
    let mono = downmix(&samples, spec.channels as usize);
    Ok(resample_linear(
        &mono,
        spec.sample_rate,
        WHISPER_SAMPLE_RATE,
    ))
}

fn decode_with_symphonia(bytes: &[u8], hint: Option<&str>) -> Result<Vec<f32>> {
    let mut format_hint = Hint::new();
    if let Some(h) = hint {
        // Accept either a bare/compound extension ("m4a", "audio.m4a") or a MIME type
        // ("audio/mp4") — extract whichever token symphonia's probe understands (a file
        // extension), falling back to passing the hint through as-is.
        let ext = h.rsplit(['.', '/']).next().unwrap_or(h);
        format_hint.with_extension(ext);
    }

    let source = Box::new(Cursor::new(bytes.to_vec()));
    let mss = MediaSourceStream::new(source, Default::default());
    let fmt_opts = FormatOptions::default();
    let meta_opts = MetadataOptions::default();
    let dec_opts = AudioDecoderOptions::default();

    let mut format = symphonia::default::get_probe()
        .probe(&format_hint, mss, fmt_opts, meta_opts)
        .map_err(|e| VoiceError::Decode(format!("unrecognized audio format: {e}")))?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| VoiceError::Decode("no audio track found".to_string()))?
        .clone();
    let codec_params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| VoiceError::Decode("no audio codec parameters".to_string()))?;
    let sample_rate = codec_params
        .sample_rate
        .ok_or_else(|| VoiceError::Decode("unknown sample rate".to_string()))?;
    let channels = codec_params
        .channels
        .as_ref()
        .map(|c| c.count())
        .unwrap_or(1)
        .max(1);

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(codec_params, &dec_opts)
        .map_err(|e| VoiceError::Decode(format!("unsupported codec: {e}")))?;

    let track_id = track.id;
    let mut interleaved: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(symphonia::core::errors::Error::IoError(_)) => break,
            Err(e) => return Err(VoiceError::Decode(format!("reading packet: {e}"))),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(audio_buf) => {
                let start = interleaved.len();
                interleaved.resize(start + audio_buf.samples_interleaved(), 0.0);
                audio_buf.copy_to_slice_interleaved(&mut interleaved[start..]);
            }
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(VoiceError::Decode(format!("decoding audio: {e}"))),
        }
    }

    let mono = downmix(&interleaved, channels);
    Ok(resample_linear(&mono, sample_rate, WHISPER_SAMPLE_RATE))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_wav(samples_i16: &[i16], channels: u16, sample_rate: u32) -> Vec<u8> {
        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
            for &s in samples_i16 {
                writer.write_sample(s).unwrap();
            }
            writer.finalize().unwrap();
        }
        cursor.into_inner()
    }

    #[test]
    fn decodes_mono_wav_at_target_rate() {
        // Already 16kHz mono: decode should be a straight int->float conversion, no resampling.
        let samples = [0i16, i16::MAX / 2, -(i16::MAX / 2), i16::MIN + 1];
        let wav = make_wav(&samples, 1, WHISPER_SAMPLE_RATE);
        let out = decode_audio(&wav, Some("clip.wav")).unwrap();
        assert_eq!(out.len(), samples.len());
        assert!((out[0]).abs() < 1e-6);
        assert!(out[1] > 0.4 && out[1] < 0.6);
    }

    #[test]
    fn decodes_and_resamples_wav() {
        let samples: Vec<i16> = (0..800).map(|i| (i % 100) as i16 * 300).collect();
        let wav = make_wav(&samples, 1, 8_000);
        let out = decode_audio(&wav, None).unwrap();
        // 8kHz -> 16kHz should roughly double the sample count.
        assert!(
            out.len() > samples.len(),
            "expected upsampling to grow the sample count"
        );
    }

    #[test]
    fn decodes_stereo_wav_downmixed_to_mono() {
        // Interleaved L/R: constant +1.0 on the left, -1.0 on the right -> mono average 0.0.
        let mut samples = Vec::new();
        for _ in 0..50 {
            samples.push(i16::MAX);
            samples.push(i16::MIN + 1);
        }
        let wav = make_wav(&samples, 2, WHISPER_SAMPLE_RATE);
        let out = decode_audio(&wav, None).unwrap();
        assert_eq!(
            out.len(),
            50,
            "stereo frames collapse to one mono sample each"
        );
        for s in out {
            assert!(s.abs() < 0.01, "L/R should cancel out to ~silence, got {s}");
        }
    }

    #[test]
    fn rejects_garbage_bytes() {
        let err = decode_audio(b"not an audio file at all", None);
        assert!(err.is_err());
    }
}
