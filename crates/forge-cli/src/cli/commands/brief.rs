//! `forge brief [--since 8h]` — the daemon's "while you were away" digest, printed.

use anyhow::Result;

use crate::attach::{resolve_base_url, resolve_token};
use crate::brief_client::{fetch_brief, parse_window, render};

pub(crate) async fn brief_cmd(
    since: String,
    json: bool,
    url: Option<String>,
    token: Option<String>,
) -> Result<()> {
    let window = parse_window(&since)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    let base = resolve_base_url(url);
    let token = resolve_token(token)?;
    let brief = fetch_brief(&base, &token, now - window).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&brief)?);
    } else {
        print!("{}", render(&brief));
    }
    Ok(())
}
