//! `[harness] read_shaping` for `search`: matches grouped per file and shown 20 files to a page,
//! with a footer naming the next page. Pure rendering over the per-file groups.

pub(crate) const FILES_PER_PAGE: usize = 20;

/// `groups` holds one rendered block per matching file, in walk order; the walk stops one file
/// past the requested page, so a group beyond the page means more exist.
pub(crate) fn render_page(query: &str, groups: &[String], page: usize) -> String {
    if groups.is_empty() {
        return format!("no matches for '{query}'");
    }
    let start = (page - 1) * FILES_PER_PAGE;
    if start >= groups.len() {
        return format!(
            "page {page} is past the end: only {} file(s) match '{query}'",
            groups.len()
        );
    }
    let end = (start + FILES_PER_PAGE).min(groups.len());
    let mut out = groups[start..end].join("\n--\n");
    let more = groups.len() > end;
    out.push_str(&format!("\n[files {}-{end}", start + 1));
    if more {
        out.push_str(&format!(
            "; more files match. Next page: repeat the search with `page: {}`]",
            page + 1
        ));
    } else {
        out.push(']');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn groups(n: usize) -> Vec<String> {
        (1..=n).map(|i| format!("f{i}.rs:1: hit")).collect()
    }

    #[test]
    fn first_page_has_twenty_files_and_a_next_page_hint() {
        let out = render_page("hit", &groups(21), 1);
        assert!(out.contains("f20.rs") && !out.contains("f21.rs"), "{out}");
        assert!(out.ends_with("`page: 2`]"), "{out}");
    }

    #[test]
    fn last_page_has_no_hint_and_an_overshoot_is_explained() {
        let out = render_page("hit", &groups(21), 2);
        assert!(out.starts_with("f21.rs"), "{out}");
        assert!(out.ends_with("[files 21-21]"), "{out}");
        assert!(render_page("hit", &groups(3), 2).contains("past the end"));
        assert_eq!(render_page("zz", &[], 1), "no matches for 'zz'");
    }
}
