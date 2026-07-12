//! Decode an uploaded audio file's raw bytes into 16kHz mono f32 — whisper's input format.
//!
//! WAV is decoded via `hound` (small, exact); everything else (m4a/aac/mp4 — what mobile clients
//! actually upload) goes through `symphonia`.

use std::io::Cursor;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, Track, TrackType};
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
        .map_err(|e| {
            VoiceError::Decode(format!(
                "unrecognized audio format (hint={}, first-bytes={}): {e}",
                hint.unwrap_or("none"),
                hex_prefix(bytes, 12),
            ))
        })?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| VoiceError::Decode(no_audio_track_error(format.as_ref())))?
        .clone();
    let codec_params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| VoiceError::Decode(no_audio_track_error(format.as_ref())))?;
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
        .map_err(|e| {
            VoiceError::Decode(format!(
                "unsupported codec ({}): {e}",
                describe_track(&track)
            ))
        })?;

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

/// Build a forensic "no decodable audio track" message: container name, track count, and a
/// per-track codec summary. Mobile uploads (iOS CoreAudio `.m4a` in particular) sometimes probe
/// fine but leave every track with an unrecognized or null codec — this is the detail needed to
/// tell "wrong container" apart from "container's audio track uses a codec/box layout we don't
/// parse".
fn no_audio_track_error(format: &dyn FormatReader) -> String {
    let container = format.format_info().short_name;
    let details: Vec<String> = format.tracks().iter().map(describe_track).collect();
    format_no_track_message(container, &details)
}

/// Pure formatting half of [`no_audio_track_error`], split out so it's testable without having to
/// construct a real `FormatReader`.
fn format_no_track_message(container: &str, track_descs: &[String]) -> String {
    format!(
        "no decodable audio track: container={container}, {} track{}: [{}]",
        track_descs.len(),
        if track_descs.len() == 1 { "" } else { "s" },
        track_descs.join(", ")
    )
}

/// One-line codec summary for a track, e.g. `codec=aac 44100Hz 2ch` or `codec=0x1610
/// (unsupported)` for a codec ID this build has no decoder registered for.
fn describe_track(track: &Track) -> String {
    match track.codec_params.as_ref() {
        Some(CodecParameters::Audio(p)) => {
            let name = symphonia::default::get_codecs()
                .get_audio_decoder(p.codec)
                .map(|d| d.codec.info.short_name);
            let rate = p.sample_rate.map(|r| format!(" {r}Hz")).unwrap_or_default();
            let channels = p
                .channels
                .as_ref()
                .map(|c| format!(" {}ch", c.count()))
                .unwrap_or_default();
            match name {
                Some(name) => format!("codec={name}{rate}{channels}"),
                None => format!("codec={} (unsupported){rate}{channels}", p.codec),
            }
        }
        Some(CodecParameters::Video(p)) => format!("codec={} (video track, not audio)", p.codec),
        Some(CodecParameters::Subtitle(p)) => {
            format!("codec={} (subtitle track, not audio)", p.codec)
        }
        Some(_) => "codec=unknown (unrecognized track type)".to_string(),
        None => "codec=none (unparsed codec parameters)".to_string(),
    }
}

/// Hex-dump the first `n` bytes of `bytes` (space-separated, e.g. `"66 74 79 70 6d 70 34 32"`) —
/// container magic bytes, useful in probe-failure errors when the format couldn't even be
/// guessed.
fn hex_prefix(bytes: &[u8], n: usize) -> String {
    bytes
        .iter()
        .take(n)
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
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
        let bytes = b"not an audio file at all";
        let err = decode_audio(bytes, None).unwrap_err();
        let msg = err.to_string();
        // Probe failure must carry the hint and the raw magic bytes for forensics.
        assert!(
            msg.contains("hint=none"),
            "expected hint in error, got: {msg}"
        );
        let expected_magic = hex_prefix(bytes, 12);
        assert!(
            msg.contains(&expected_magic),
            "expected first-bytes magic {expected_magic:?} in error, got: {msg}"
        );
    }

    #[test]
    fn hex_prefix_formats_lowercase_space_separated_bytes() {
        assert_eq!(hex_prefix(&[0x00, 0x10, 0xff], 12), "00 10 ff");
        assert_eq!(hex_prefix(&[0xab, 0xcd, 0xef, 0x01], 2), "ab cd");
        assert_eq!(hex_prefix(&[], 12), "");
    }

    #[test]
    fn describe_track_reports_known_codec_with_rate_and_channels() {
        use symphonia::core::audio::Channels;
        use symphonia::core::codecs::audio::well_known::CODEC_ID_AAC;
        use symphonia::core::codecs::audio::AudioCodecParameters;
        use symphonia::core::codecs::CodecParameters;

        let mut track = Track::new(1);
        track.with_codec_params(CodecParameters::Audio(AudioCodecParameters {
            codec: CODEC_ID_AAC,
            sample_rate: Some(44_100),
            channels: Some(Channels::Discrete(2)),
            ..Default::default()
        }));

        let desc = describe_track(&track);
        assert_eq!(desc, "codec=aac 44100Hz 2ch");
    }

    #[test]
    fn describe_track_flags_codec_with_no_registered_decoder() {
        use symphonia::core::codecs::audio::well_known::CODEC_ID_MP3;
        use symphonia::core::codecs::audio::AudioCodecParameters;
        use symphonia::core::codecs::CodecParameters;

        // This build only registers aac + pcm decoders (see Cargo.toml features), so MP3 is a
        // codec ID symphonia knows the *name* of but has no decoder for here.
        let mut track = Track::new(1);
        track.with_codec_params(CodecParameters::Audio(AudioCodecParameters {
            codec: CODEC_ID_MP3,
            sample_rate: Some(48_000),
            ..Default::default()
        }));

        let desc = describe_track(&track);
        assert!(
            desc.contains("(unsupported)"),
            "expected unsupported marker, got: {desc}"
        );
        assert!(
            desc.contains("48000Hz"),
            "expected sample rate, got: {desc}"
        );
    }

    #[test]
    fn describe_track_reports_missing_codec_params() {
        let track = Track::new(1);
        assert_eq!(
            describe_track(&track),
            "codec=none (unparsed codec parameters)"
        );
    }

    #[test]
    fn format_no_track_message_includes_container_and_track_count() {
        let msg = format_no_track_message(
            "isomp4",
            &[
                "codec=0x1610 (unsupported)".to_string(),
                "codec=aac 44100Hz 2ch".to_string(),
            ],
        );
        assert_eq!(
            msg,
            "no decodable audio track: container=isomp4, 2 tracks: \
             [codec=0x1610 (unsupported), codec=aac 44100Hz 2ch]"
        );
    }

    #[test]
    fn format_no_track_message_singular_track_count() {
        let msg = format_no_track_message(
            "wav",
            &["codec=none (unparsed codec parameters)".to_string()],
        );
        assert!(
            msg.contains("1 track:"),
            "expected singular 'track', got: {msg}"
        );
    }
}
