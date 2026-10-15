//! Loading images for vision input: from a file path (`/image <path>`, `@path`, a dropped path),
//! raw bytes, or the OS clipboard (paste). Everything funnels through [`prepare_image`], which
//! decides the real format from magic bytes (never the extension), converts HEIC/HEIF to JPEG,
//! and downscales big images so a 12-megapixel screenshot does not blow a provider's size limit.

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use forge_types::ImageAttachment;

/// Hard ceiling on an input file's size, before any resizing.
const MAX_BYTES: usize = 20 * 1024 * 1024;
/// Longest side we send. Providers downsample beyond roughly this anyway.
const MAX_SIDE: u32 = 2000;
/// Largest encoded image sent unchanged; base64 of this stays under common 5 MB provider limits.
const PASSTHROUGH_BYTES: usize = 3 * 1024 * 1024;
const JPEG_QUALITY: u8 = 85;
/// More distinct colours than this in a sample means a photograph rather than a screenshot.
const PHOTO_COLOURS: usize = 2048;

fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sniffed {
    Png,
    Jpeg,
    Gif,
    Webp,
    Heic,
}

impl Sniffed {
    fn media_type(self) -> &'static str {
        match self {
            Sniffed::Png => "image/png",
            Sniffed::Jpeg | Sniffed::Heic => "image/jpeg",
            Sniffed::Gif => "image/gif",
            Sniffed::Webp => "image/webp",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Sniffed::Png => "PNG",
            Sniffed::Jpeg => "JPEG",
            Sniffed::Gif => "GIF",
            Sniffed::Webp => "WEBP",
            Sniffed::Heic => "HEIC",
        }
    }
}

pub fn sniff(bytes: &[u8]) -> Option<Sniffed> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(Sniffed::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(Sniffed::Jpeg)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(Sniffed::Gif)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(Sniffed::Webp)
    } else if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        const BRANDS: [&[u8]; 10] = [
            b"heic", b"heix", b"heim", b"heis", b"hevc", b"hevx", b"hevm", b"hevs", b"mif1",
            b"msf1",
        ];
        BRANDS.contains(&&bytes[8..12]).then_some(Sniffed::Heic)
    } else {
        None
    }
}

pub fn sniff_media_type(bytes: &[u8]) -> Option<&'static str> {
    sniff(bytes).map(Sniffed::media_type)
}

/// Runs `program args…` and reports whether it exited successfully. Faked in tests.
pub trait Converter {
    fn run(&self, program: &str, args: &[String]) -> bool;
}

struct SystemConverter;

impl Converter for SystemConverter {
    fn run(&self, program: &str, args: &[String]) -> bool {
        let Ok(mut child) = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            return false;
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return status.success(),
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(40));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
            }
        }
    }
}

/// Converter invocations to try, in order; `{in}`/`{out}` are the temp file paths.
const HEIC_CONVERTERS: [(&str, &[&str]); 4] = [
    ("heif-convert", &["-q", "85", "{in}", "{out}"]),
    ("magick", &["{in}", "-quality", "85", "{out}"]),
    (
        "sips",
        &[
            "-s",
            "format",
            "jpeg",
            "-s",
            "formatOptions",
            "85",
            "{in}",
            "--out",
            "{out}",
        ],
    ),
    ("convert", &["{in}", "-quality", "85", "{out}"]),
];

fn heic_to_jpeg(bytes: &[u8], conv: &dyn Converter) -> Result<Vec<u8>> {
    let dir = std::env::temp_dir().join(format!(
        "forge-heic-{}-{:08x}",
        std::process::id(),
        rand::random::<u32>()
    ));
    std::fs::create_dir_all(&dir)?;
    let input = dir.join("in.heic");
    let output = dir.join("out.jpg");
    let result = (|| {
        std::fs::write(&input, bytes)?;
        for (program, template) in HEIC_CONVERTERS {
            let args: Vec<String> = template
                .iter()
                .map(|a| match *a {
                    "{in}" => input.display().to_string(),
                    "{out}" => output.display().to_string(),
                    other => other.to_string(),
                })
                .collect();
            if conv.run(program, &args) {
                if let Ok(jpeg) = std::fs::read(&output) {
                    if sniff(&jpeg) == Some(Sniffed::Jpeg) {
                        return Ok(jpeg);
                    }
                }
            }
        }
        Err(anyhow!(
            "HEIC/HEIF needs a converter: install libheif (`heif-convert`) or ImageMagick, \
             or export the picture as JPEG/PNG"
        ))
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// Decode, apply the EXIF orientation, shrink to [`MAX_SIDE`], and encode as PNG (few colours or
/// transparency: screenshots, diagrams) or JPEG (photographs).
fn reencode(bytes: &[u8]) -> Result<(Vec<u8>, &'static str, u32, u32)> {
    use image::{ImageDecoder as _, ImageReader};
    let reader = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .context("reading image")?;
    let mut decoder = reader.into_decoder().context("decoding image")?;
    let orientation = decoder.orientation().ok();
    let mut img = image::DynamicImage::from_decoder(decoder).context("decoding image")?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    if img.width().max(img.height()) > MAX_SIDE {
        img = img.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Lanczos3);
    }
    let (w, h) = (img.width(), img.height());
    let mut out = std::io::Cursor::new(Vec::new());
    if looks_like_screenshot(&img) {
        img.write_to(&mut out, image::ImageFormat::Png)?;
        Ok((out.into_inner(), "image/png", w, h))
    } else {
        let rgb = img.to_rgb8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
            .encode_image(&rgb)?;
        Ok((out.into_inner(), "image/jpeg", w, h))
    }
}

fn looks_like_screenshot(img: &image::DynamicImage) -> bool {
    if img.color().has_alpha() {
        let rgba = img.to_rgba8();
        if rgba.pixels().any(|p| p.0[3] < 255) {
            return true;
        }
    }
    let rgb = img.to_rgb8();
    let total = (rgb.width() as usize) * (rgb.height() as usize);
    let step = (total / 65_536).max(1);
    let mut seen = std::collections::HashSet::new();
    for p in rgb.pixels().step_by(step) {
        seen.insert(p.0);
        if seen.len() > PHOTO_COLOURS {
            return false;
        }
    }
    true
}

fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

/// Turn raw image bytes of any supported format into a provider-ready attachment plus a short
/// label ("PNG 800x600 120KB").
pub fn prepare_image(bytes: &[u8]) -> Result<(ImageAttachment, String)> {
    prepare_image_with(bytes, &SystemConverter)
}

pub fn prepare_image_with(bytes: &[u8], conv: &dyn Converter) -> Result<(ImageAttachment, String)> {
    if bytes.is_empty() {
        return Err(anyhow!("image is empty"));
    }
    if bytes.len() > MAX_BYTES {
        return Err(anyhow!(
            "image too large ({} MB; max 20 MB)",
            bytes.len() / 1_048_576
        ));
    }
    let kind = sniff(bytes).ok_or_else(|| {
        anyhow!("not a supported image (png/jpeg/gif/webp/heic) — checked the file contents")
    })?;
    let converted;
    let (bytes, kind) = if kind == Sniffed::Heic {
        converted = heic_to_jpeg(bytes, conv)?;
        (converted.as_slice(), Sniffed::Jpeg)
    } else {
        (bytes, kind)
    };

    let dims = dimensions(bytes);
    let fits = dims.is_some_and(|(w, h)| w.max(h) <= MAX_SIDE);
    if fits && bytes.len() <= PASSTHROUGH_BYTES {
        let (w, h) = dims.unwrap_or_default();
        return Ok((
            ImageAttachment {
                media_type: kind.media_type().to_string(),
                data_base64: encode(bytes),
            },
            format!("{} {w}x{h} {}KB", kind.label(), bytes.len().div_ceil(1024)),
        ));
    }
    let (out, media_type, w, h) = reencode(bytes)?;
    if out.len() > MAX_BYTES {
        return Err(anyhow!("image still too large after resizing"));
    }
    let label = format!(
        "{} {w}x{h} {}KB",
        if media_type == "image/png" {
            "PNG"
        } else {
            "JPEG"
        },
        out.len().div_ceil(1024)
    );
    Ok((
        ImageAttachment {
            media_type: media_type.to_string(),
            data_base64: encode(&out),
        },
        label,
    ))
}

/// Read an image file (quotes, `~`, `file://` and backslash-escapes in `path` are undone).
pub fn load_image_file(path: &str) -> Result<(ImageAttachment, String)> {
    let p = crate::attach_paths::normalize_path_arg(path);
    let meta = std::fs::metadata(&p).with_context(|| format!("reading {}", p.display()))?;
    if meta.len() as usize > MAX_BYTES {
        return Err(anyhow!(
            "{} too large ({} MB; max 20 MB)",
            p.display(),
            meta.len() / 1_048_576
        ));
    }
    let bytes = std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?;
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("image");
    let (att, label) = prepare_image(&bytes).map_err(|e| anyhow!("{}: {e}", p.display()))?;
    Ok((att, format!("{name} {label}")))
}

pub enum ClipboardImage {
    Found(ImageAttachment, String),
    /// Nothing attached; the text says why (shown to the user as a dim note).
    None(String),
}

/// Read an image from the clipboard. Blocking, bounded by per-backend timeouts.
pub fn clipboard_image() -> ClipboardImage {
    clipboard_image_with(
        &crate::clipboard_image::Env::current(),
        &crate::clipboard_image::SystemRunner,
    )
}

/// What `Ctrl+V` does: attach a clipboard image, or (no image) fall back to voice input.
pub enum CtrlV {
    Image(ImageAttachment, String),
    Voice,
}

#[cfg(test)]
pub fn ctrl_v_with(
    env: &crate::clipboard_image::Env,
    runner: &dyn crate::clipboard_image::Runner,
) -> CtrlV {
    match clipboard_image_with(env, runner) {
        ClipboardImage::Found(att, label) => CtrlV::Image(att, label),
        ClipboardImage::None(_) => CtrlV::Voice,
    }
}

pub async fn ctrl_v() -> CtrlV {
    match clipboard_image_async().await {
        ClipboardImage::Found(att, label) => CtrlV::Image(att, label),
        ClipboardImage::None(_) => CtrlV::Voice,
    }
}

pub async fn clipboard_image_async() -> ClipboardImage {
    tokio::task::spawn_blocking(clipboard_image)
        .await
        .unwrap_or_else(|_| ClipboardImage::None("clipboard read failed".to_string()))
}

pub fn clipboard_image_with(
    env: &crate::clipboard_image::Env,
    runner: &dyn crate::clipboard_image::Runner,
) -> ClipboardImage {
    use crate::clipboard_image::{read_image, Outcome};
    match read_image(env, runner) {
        Outcome::Found { bytes, backend } => match prepare_image(&bytes) {
            Ok((att, label)) => ClipboardImage::Found(att, label),
            Err(e) => ClipboardImage::None(format!("clipboard image from {backend} unusable: {e}")),
        },
        Outcome::NoImage { why, .. } => ClipboardImage::None(why),
    }
}

#[cfg(test)]
pub(crate) fn test_png() -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::new(2, 2))
        .write_to(&mut buf, image::ImageFormat::Png)
        .unwrap();
    buf.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(img: image::DynamicImage, fmt: image::ImageFormat) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, fmt).unwrap();
        buf.into_inner()
    }

    fn flat(w: u32, h: u32) -> image::DynamicImage {
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(w, h, image::Rgb([20, 40, 60])))
    }

    fn noisy(w: u32, h: u32) -> image::DynamicImage {
        let mut seed = 12345u32;
        image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |_, _| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            image::Rgb([(seed >> 8) as u8, (seed >> 16) as u8, (seed >> 24) as u8])
        }))
    }

    #[test]
    fn sniffs_formats_by_magic_bytes() {
        assert_eq!(
            sniff(&encoded(flat(2, 2), image::ImageFormat::Png)),
            Some(Sniffed::Png)
        );
        assert_eq!(
            sniff(&encoded(flat(2, 2), image::ImageFormat::Jpeg)),
            Some(Sniffed::Jpeg)
        );
        assert_eq!(
            sniff(&encoded(flat(2, 2), image::ImageFormat::Gif)),
            Some(Sniffed::Gif)
        );
        assert_eq!(sniff(b"RIFF\x00\x00\x00\x00WEBPVP8 "), Some(Sniffed::Webp));
        assert_eq!(
            sniff(b"\x00\x00\x00\x18ftypheic\x00\x00\x00\x00"),
            Some(Sniffed::Heic)
        );
        assert_eq!(
            sniff(b"\x00\x00\x00\x18ftypmif1\x00\x00\x00\x00"),
            Some(Sniffed::Heic)
        );
        assert_eq!(sniff(b"\x00\x00\x00\x18ftypmp42\x00\x00\x00\x00"), None);
        assert_eq!(sniff(b"hello"), None);
    }

    #[test]
    fn extension_is_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("really-a-png.jpg");
        std::fs::write(&path, encoded(flat(4, 4), image::ImageFormat::Png)).unwrap();
        let (att, label) = load_image_file(path.to_str().unwrap()).unwrap();
        assert_eq!(att.media_type, "image/png");
        assert!(
            label.contains("really-a-png.jpg") && label.contains("PNG 4x4"),
            "{label}"
        );

        let text = dir.path().join("fake.png");
        std::fs::write(&text, "not an image").unwrap();
        assert!(load_image_file(text.to_str().unwrap()).is_err());
    }

    #[test]
    fn small_images_pass_through_untouched() {
        let png = encoded(flat(10, 10), image::ImageFormat::Png);
        let (att, _) = prepare_image(&png).unwrap();
        assert_eq!(att.data_base64, encode(&png));
    }

    #[test]
    fn large_screenshot_is_downscaled_and_stays_png() {
        let big = encoded(flat(3000, 1500), image::ImageFormat::Png);
        let (att, label) = prepare_image(&big).unwrap();
        assert_eq!(att.media_type, "image/png");
        assert!(label.contains("2000x1000"), "{label}");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(att.data_base64)
            .unwrap();
        assert_eq!(dimensions(&bytes), Some((2000, 1000)));
    }

    #[test]
    fn large_photo_becomes_jpeg() {
        let big = encoded(noisy(2400, 300), image::ImageFormat::Png);
        let (att, label) = prepare_image(&big).unwrap();
        assert_eq!(att.media_type, "image/jpeg");
        assert!(label.contains("2000x250"), "{label}");
    }

    #[test]
    fn tall_image_is_bounded_on_the_longest_side() {
        let big = encoded(flat(500, 4000), image::ImageFormat::Png);
        let (_, label) = prepare_image(&big).unwrap();
        assert!(label.contains("250x2000"), "{label}");
    }

    #[test]
    fn path_forms_all_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("my shot.png");
        std::fs::write(&path, encoded(flat(3, 3), image::ImageFormat::Png)).unwrap();
        let p = path.display().to_string();
        for form in [
            format!("'{p}'"),
            format!("\"{p}\""),
            p.replace(' ', "\\ "),
            format!("file://{}", p.replace(' ', "%20")),
        ] {
            assert!(load_image_file(&form).is_ok(), "{form}");
        }
    }

    struct FakeConv {
        jpeg: Option<Vec<u8>>,
        calls: std::cell::RefCell<Vec<String>>,
    }

    impl Converter for FakeConv {
        fn run(&self, program: &str, args: &[String]) -> bool {
            self.calls.borrow_mut().push(program.to_string());
            match (&self.jpeg, program) {
                (Some(j), "magick") => {
                    std::fs::write(&args[args.len() - 1], j).unwrap();
                    true
                }
                _ => false,
            }
        }
    }

    #[test]
    fn heic_is_converted_by_the_first_working_tool() {
        let heic = b"\x00\x00\x00\x18ftypheic\x00\x00\x00\x00moreheicbytes".to_vec();
        let jpeg = encoded(flat(8, 8), image::ImageFormat::Jpeg);
        let conv = FakeConv {
            jpeg: Some(jpeg),
            calls: Default::default(),
        };
        let (att, label) = prepare_image_with(&heic, &conv).unwrap();
        assert_eq!(att.media_type, "image/jpeg");
        assert!(label.starts_with("JPEG 8x8"), "{label}");
        assert_eq!(*conv.calls.borrow(), ["heif-convert", "magick"]);
    }

    #[test]
    fn heic_without_a_converter_says_what_to_install() {
        let heic = b"\x00\x00\x00\x18ftypheic\x00\x00\x00\x00moreheicbytes".to_vec();
        let conv = FakeConv {
            jpeg: None,
            calls: Default::default(),
        };
        let err = prepare_image_with(&heic, &conv).unwrap_err().to_string();
        assert!(
            err.contains("heif-convert") && err.contains("ImageMagick"),
            "{err}"
        );
        assert_eq!(conv.calls.borrow().len(), 4);
    }

    #[test]
    fn clipboard_bytes_are_prepared_and_failures_explained() {
        use crate::clipboard_image::{Env, RunError, Runner};
        struct R(Vec<u8>);
        impl Runner for R {
            fn run(&self, p: &str, a: &[&str], _: &[(&str, &str)]) -> Result<Vec<u8>, RunError> {
                match p {
                    "xclip" if a.contains(&"TARGETS") => Ok(b"image/png\n".to_vec()),
                    "xclip" => Ok(self.0.clone()),
                    _ => Err(RunError::NotFound),
                }
            }
            fn native_png(&self) -> Option<Vec<u8>> {
                None
            }
        }
        let env = Env {
            os: "linux",
            display: Some(":0".into()),
            ..Default::default()
        };
        let png = encoded(flat(5, 5), image::ImageFormat::Png);
        match clipboard_image_with(&env, &R(png)) {
            ClipboardImage::Found(att, label) => {
                assert_eq!(att.media_type, "image/png");
                assert!(label.contains("5x5"));
            }
            ClipboardImage::None(why) => panic!("{why}"),
        }
        assert!(matches!(
            ctrl_v_with(&env, &R(encoded(flat(2, 2), image::ImageFormat::Png))),
            CtrlV::Image(..)
        ));
        assert!(matches!(ctrl_v_with(&env, &R(Vec::new())), CtrlV::Voice));
        assert!(matches!(
            ctrl_v_with(&Env::default(), &R(Vec::new())),
            CtrlV::Voice
        ));
        match clipboard_image_with(&env, &R(b"garbage".to_vec())) {
            ClipboardImage::None(why) => assert!(why.contains("unusable"), "{why}"),
            ClipboardImage::Found(..) => panic!("garbage accepted"),
        }
    }
}
