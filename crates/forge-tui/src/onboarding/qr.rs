//! A scannable QR code as terminal lines, sized to the room it gets. Dark modules are drawn as
//! filled cells, so the code scans inverted on a dark background and normally on a light one, and
//! the terminal background around it is the quiet zone.

use std::cell::RefCell;
use std::rc::Rc;

use qrcode::{Color as Module, EcLevel, QrCode};

pub(crate) enum Qr {
    Fits(Vec<String>),
    /// The smallest rendering still needs this many rows and columns.
    TooBig {
        rows: usize,
        cols: usize,
    },
}

/// The largest rendering of `payload` that fits `rows` x `cols`: half blocks (square modules,
/// two per cell) first, then sextants (six per cell) where the terminal draws them itself.
/// `None` when the payload cannot be encoded at all.
pub(crate) fn fitted(payload: &str, rows: usize, cols: usize, sextants: bool) -> Option<Qr> {
    let code = grid(payload)?;
    let half = render(&code, 1, 2, half_glyph);
    if half.len() <= rows && width(&half) <= cols {
        return Some(Qr::Fits(half));
    }
    if !sextants {
        return Some(Qr::TooBig {
            rows: half.len(),
            cols: width(&half),
        });
    }
    let small = render(&code, 2, 3, sextant_glyph);
    Some(if small.len() <= rows && width(&small) <= cols {
        Qr::Fits(small)
    } else {
        Qr::TooBig {
            rows: small.len(),
            cols: width(&small),
        }
    })
}

fn width(lines: &[String]) -> usize {
    lines.first().map_or(0, |l| l.chars().count())
}

/// The module grid of a payload, row-major.
struct Grid {
    size: usize,
    dark: Vec<bool>,
}

thread_local! {
    /// Encoding is the expensive part and the screen redraws many times a second; the link is the
    /// same for the whole sign-in.
    static LAST: RefCell<Option<(String, Option<Rc<Grid>>)>> = const { RefCell::new(None) };
}

fn grid(payload: &str) -> Option<Rc<Grid>> {
    LAST.with(|last| {
        let mut last = last.borrow_mut();
        if let Some((cached, grid)) = last.as_ref() {
            if cached == payload {
                return grid.clone();
            }
        }
        // Low error correction: the code is on a screen, not a printed label, and it saves rows.
        let grid = QrCode::with_error_correction_level(payload.as_bytes(), EcLevel::L)
            .ok()
            .map(|code| {
                Rc::new(Grid {
                    size: code.width(),
                    dark: code
                        .to_colors()
                        .into_iter()
                        .map(|c| c == Module::Dark)
                        .collect(),
                })
            });
        *last = Some((payload.to_string(), grid.clone()));
        grid
    })
}

/// One character per `cw` x `ch` block of modules; `glyph` gets the dark bits row-major from the
/// top-left.
fn render(code: &Grid, cw: usize, ch: usize, glyph: fn(u8) -> char) -> Vec<String> {
    let size = code.size;
    let dark = |x: usize, y: usize| x < size && y < size && code.dark[y * size + x];
    (0..size)
        .step_by(ch)
        .map(|y| {
            (0..size)
                .step_by(cw)
                .map(|x| {
                    let mut bits = 0u8;
                    for dy in 0..ch {
                        for dx in 0..cw {
                            if dark(x + dx, y + dy) {
                                bits |= 1 << (dy * cw + dx);
                            }
                        }
                    }
                    glyph(bits)
                })
                .collect()
        })
        .collect()
}

fn half_glyph(bits: u8) -> char {
    match bits {
        0 => ' ',
        1 => '▀',
        2 => '▄',
        _ => '█',
    }
}

/// Unicode "block sextant" characters (U+1FB00..U+1FB3B) cover every 2x3 pattern except the four
/// that already exist as older block elements.
fn sextant_glyph(bits: u8) -> char {
    match bits {
        0 => ' ',
        21 => '▌',
        42 => '▐',
        63 => '█',
        b => {
            let skipped = u32::from(b > 21) + u32::from(b > 42);
            char::from_u32(0x1FB00 + u32::from(b) - 1 - skipped).unwrap_or('█')
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://claude.com/cai/oauth/authorize?code=true&client_id=9d1c250a-e61b-44d9-88ed-5944d1962f5e&response_type=code&redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&scope=user%3Ainference&code_challenge=ggyp0-bc8nZj3BxEEhvyhGq-sNI00PD9NNGDQ7c5yCU&code_challenge_method=S256&state=rfqXDL-weMT7TvTE0bhcsqV81PbaD5E5s0ijwClgKE8";

    #[test]
    fn a_roomy_screen_gets_square_half_block_modules() {
        let Some(Qr::Fits(rows)) = fitted("https://claude.ai/x?state=abc", 40, 100, false) else {
            panic!("short link fits");
        };
        let w = width(&rows);
        assert!(rows.iter().all(|r| r.chars().count() == w));
        assert_eq!(rows.len(), w.div_ceil(2), "two module rows per line");
    }

    #[test]
    fn a_real_claude_sign_in_link_shrinks_to_fit_a_40_row_terminal() {
        let Some(Qr::Fits(rows)) = fitted(URL, 30, 80, true) else {
            panic!("the sextant rendering fits 30 rows");
        };
        assert!(rows.len() <= 30, "{}", rows.len());
        assert!(rows
            .iter()
            .flat_map(|r| r.chars())
            .any(|c| ('\u{1FB00}'..='\u{1FB3B}').contains(&c)));
    }

    #[test]
    fn without_sextants_a_too_small_screen_reports_what_it_needs() {
        match fitted(URL, 20, 200, false) {
            Some(Qr::TooBig { rows, cols }) => assert!(rows > 20 && cols > rows),
            _ => panic!("half blocks need more than 20 rows"),
        }
    }

    #[test]
    fn sextant_codes_map_onto_the_unicode_block() {
        assert_eq!(sextant_glyph(1), '\u{1FB00}');
        assert_eq!(sextant_glyph(20), '\u{1FB13}');
        assert_eq!(sextant_glyph(22), '\u{1FB14}');
        assert_eq!(sextant_glyph(62), '\u{1FB3B}');
    }

    #[test]
    fn an_oversized_payload_is_none_not_a_panic() {
        assert!(fitted(&"x".repeat(10_000), 40, 100, true).is_none());
    }
}
