//! A scannable QR code as terminal lines, two modules per cell with half blocks.

/// The code for `payload`, or `None` when it cannot be encoded. Dark modules are drawn as filled
/// cells, which scans on a dark terminal background and on a light one.
pub(crate) fn lines(payload: &str) -> Option<Vec<String>> {
    let code = qrcode::QrCode::new(payload.as_bytes()).ok()?;
    let width = code.width();
    let mut out = Vec::with_capacity(width.div_ceil(2) + 2);
    let quiet = " ".repeat(width + 4);
    out.push(quiet.clone());
    for y in (0..width).step_by(2) {
        let mut row = String::from("  ");
        for x in 0..width {
            let top_light = code[(x, y)].select(false, true);
            let bottom_light = y + 1 >= width || code[(x, y + 1)].select(false, true);
            row.push(match (top_light, bottom_light) {
                (true, true) => ' ',
                (true, false) => '▄',
                (false, true) => '▀',
                (false, false) => '█',
            });
        }
        row.push_str("  ");
        out.push(row);
    }
    out.push(quiet);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_becomes_a_square_ish_block_of_half_block_rows() {
        let rows = lines("https://claude.ai/oauth/authorize?state=abc").unwrap();
        assert!(rows.len() > 10 && rows.len() < 40, "{}", rows.len());
        let width = rows[0].chars().count();
        assert!(rows.iter().all(|r| r.chars().count() == width));
        assert!(rows.iter().any(|r| r.contains('█')));
    }

    #[test]
    fn an_oversized_payload_is_none_not_a_panic() {
        assert!(lines(&"x".repeat(10_000)).is_none());
    }
}
