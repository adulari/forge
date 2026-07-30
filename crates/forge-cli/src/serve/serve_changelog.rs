//! Compiled release notes projection for the Serve control surface.

use axum::extract::Query;
use axum::response::Response;

use super::json_response;

/// The repository's Keep-a-Changelog file, compiled in.
///
/// Embedded rather than read from disk because the daemon's cwd is a USER project, not the Forge
/// checkout — there is no path at runtime that reliably holds the changelog for the binary that is
/// actually running, and "what's new" is only true if it matches that binary. The package-local
/// mirror makes the crates.io tarball self-contained. Only the top
/// [`CHANGELOG_DEFAULT_RELEASES`] sections are ever parsed, so the ~200 KB is static rodata, never
/// a per-request cost.
// Kept byte-identical to the repository root by test-crates-release-order.sh so the published
// crate is self-contained instead of reaching outside its tarball during compilation.
const CHANGELOG_MD: &str = include_str!("../../CHANGELOG.md");

const CHANGELOG_DEFAULT_RELEASES: usize = 10;
const CHANGELOG_MAX_RELEASES: usize = 50;

#[derive(serde::Deserialize)]
pub(super) struct ChangelogParams {
    limit: Option<usize>,
}

/// One bullet, tagged with the `### Added` / `### Changed` / `### Fixed` heading it sat under.
#[derive(serde::Serialize, PartialEq, Debug)]
struct ChangelogEntry {
    section: String,
    text: String,
}

#[derive(serde::Serialize, PartialEq, Debug)]
struct ChangelogRelease {
    version: String,
    /// `null` for `[Unreleased]`, which carries no date.
    date: Option<String>,
    entries: Vec<ChangelogEntry>,
}

/// Parse the top `limit` `## [version] - date` sections. Continuation lines of a wrapped bullet are
/// folded back into that bullet; `[Unreleased]` is emitted only when it actually has entries.
fn parse_changelog(markdown: &str, limit: usize) -> Vec<ChangelogRelease> {
    let mut releases: Vec<ChangelogRelease> = Vec::new();
    let mut current: Option<ChangelogRelease> = None;
    let mut section = String::new();
    for line in markdown.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            if let Some(release) = current.take() {
                if !release.entries.is_empty() {
                    releases.push(release);
                }
            }
            if releases.len() >= limit {
                return releases;
            }
            let (version, date) = match heading.split_once(" - ") {
                Some((version, date)) => (version, Some(date.trim().to_string())),
                None => (heading, None),
            };
            let version = version.trim().trim_matches(['[', ']']).to_string();
            // The running binary does not contain unreleased work, so `[Unreleased]` must never
            // reach its own "What's New". This used to fall out of the empty-entries check below,
            // which only held because that section happened to always be empty — the moment
            // anything landed under it, it became the feed's newest "release", dateless.
            // Leaving `current` unset also discards its bullets: entries only attach to a live
            // release.
            current = (!version.eq_ignore_ascii_case("Unreleased")).then_some(ChangelogRelease {
                version,
                date,
                entries: Vec::new(),
            });
            section.clear();
            continue;
        }
        if let Some(heading) = line.strip_prefix("### ") {
            section = heading.trim().to_string();
            continue;
        }
        let Some(release) = current.as_mut() else {
            continue;
        };
        if let Some(item) = line.strip_prefix("- ") {
            release.entries.push(ChangelogEntry {
                section: section.clone(),
                text: item.trim().to_string(),
            });
        } else if line.starts_with("  ") && !line.trim().is_empty() {
            // A wrapped bullet: Forge's changelog hard-wraps long entries at ~100 columns.
            if let Some(last) = release.entries.last_mut() {
                last.text.push(' ');
                last.text.push_str(line.trim());
            }
        }
    }
    if let Some(release) = current.take() {
        if !release.entries.is_empty() && releases.len() < limit {
            releases.push(release);
        }
    }
    releases
}

/// `GET /api/changelog?limit=<n>` — the "What's New" feed for the running binary.
pub(super) async fn changelog_page(Query(params): Query<ChangelogParams>) -> Response {
    let limit = params
        .limit
        .unwrap_or(CHANGELOG_DEFAULT_RELEASES)
        .clamp(1, CHANGELOG_MAX_RELEASES);
    json_response(&parse_changelog(CHANGELOG_MD, limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changelog_parses_the_top_releases_with_their_sections() {
        let releases = parse_changelog(CHANGELOG_MD, 3);
        assert_eq!(releases.len(), 3, "the top N releases, no more");
        assert!(
            releases.iter().all(|r| !r.entries.is_empty()),
            "sections with no entries are never emitted"
        );
        assert!(
            !releases
                .iter()
                .any(|r| r.version.eq_ignore_ascii_case("Unreleased")),
            "the running binary never advertises unreleased work"
        );
        let first = &releases[0];
        assert!(
            first
                .version
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit()),
            "version brackets are stripped: {}",
            first.version
        );
        assert!(first.date.is_some(), "a released version carries its date");
        assert!(first.entries.iter().all(|e| matches!(
            e.section.as_str(),
            "Added" | "Changed" | "Fixed" | "Removed"
        )));
    }

    // The assertion above goes vacuous whenever `[Unreleased]` happens to be empty, which is most
    // of the time — a released CHANGELOG_MD cannot prove this. Pin the behaviour on a fixture that
    // always has unreleased entries.
    #[test]
    fn a_populated_unreleased_section_never_reaches_the_whats_new_feed() {
        let markdown = "\
## [Unreleased]

### Fixed

- Something that has not shipped yet.

## [9.9.9] - 2026-01-01

### Added

- Something that has.
";
        let releases = parse_changelog(markdown, 3);
        assert_eq!(releases.len(), 1, "only the shipped release is advertised");
        assert_eq!(releases[0].version, "9.9.9");
        assert_eq!(
            releases[0].entries.len(),
            1,
            "unreleased bullets are dropped"
        );
        assert_eq!(releases[0].entries[0].text, "Something that has.");
    }

    #[test]
    fn changelog_folds_wrapped_bullets_and_skips_empty_unreleased() {
        let markdown = "# Changelog\n\n## [Unreleased]\n\n## [1.2.0] - 2026-01-02\n\n### Added\n\n- a thing\n  that wrapped\n- another\n\n### Fixed\n\n- a fix\n";
        let releases = parse_changelog(markdown, 10);
        assert_eq!(releases.len(), 1, "empty `[Unreleased]` is dropped");
        assert_eq!(releases[0].version, "1.2.0");
        assert_eq!(releases[0].date.as_deref(), Some("2026-01-02"));
        assert_eq!(
            releases[0].entries,
            vec![
                ChangelogEntry {
                    section: "Added".into(),
                    text: "a thing that wrapped".into()
                },
                ChangelogEntry {
                    section: "Added".into(),
                    text: "another".into()
                },
                ChangelogEntry {
                    section: "Fixed".into(),
                    text: "a fix".into()
                },
            ]
        );
    }
}
