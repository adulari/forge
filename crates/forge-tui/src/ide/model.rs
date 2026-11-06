//! The change queue as plain data: which changed files need a human first. A port of the desktop
//! `changeQueueModel.ts`, with the same ordering rule (waiting on you, then conflicts, then failing
//! checks, then unreviewed, then reviewed) and the same test cases, so the two surfaces can never
//! disagree about what comes first.

use std::collections::BTreeMap;

/// Lower is more urgent; the order is the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    Waiting,
    Conflict,
    Failing,
    Unreviewed,
    Reviewed,
}

/// The daemon's verdict on a file: every hunk routine, or something to look at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    Look,
    Routine,
}

impl Risk {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "look" => Some(Risk::Look),
            "routine" => Some(Risk::Routine),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: String,
    /// Git porcelain letter: M A D R ?; "U" for an unmerged path.
    pub status: String,
    pub adds: usize,
    pub dels: usize,
    /// Identity of the file's changed content. A reviewed mark is only honoured while this holds.
    pub hash: String,
    pub conflicted: bool,
    /// A check (test, lint, build) that names this file is failing.
    pub failing: bool,
    /// `None` from a daemon that predates risk ranking.
    pub risk: Option<Risk>,
    pub risk_reason: String,
    pub look_hunks: usize,
    pub routine_hunks: usize,
}

impl ChangedFile {
    pub fn new(path: &str, status: &str, adds: usize, dels: usize, hash: &str) -> Self {
        Self {
            path: path.to_string(),
            status: status.to_string(),
            adds,
            dels,
            hash: hash.to_string(),
            conflicted: false,
            failing: false,
            risk: None,
            risk_reason: String::new(),
            look_hunks: 0,
            routine_hunks: 0,
        }
    }

    /// Sort key: files that need a look first; an unranked file counts as needing one.
    pub fn risk_rank(&self) -> u8 {
        u8::from(self.risk == Some(Risk::Routine))
    }
}

/// Hunk and file counts over the files still to review ("9 look here · 31 routine").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RiskTotals {
    /// Some file carries a verdict; without one the queue shows no risk UI.
    pub ranked: bool,
    pub look_hunks: usize,
    pub routine_hunks: usize,
    pub look_files: usize,
    pub routine_files: usize,
}

impl RiskTotals {
    pub fn line(&self) -> String {
        let mut parts = Vec::new();
        if self.look_hunks > 0 {
            parts.push(format!("{} look here", self.look_hunks));
        }
        if self.routine_hunks > 0 {
            parts.push(format!("{} routine", self.routine_hunks));
        }
        parts.join(" · ")
    }
}

pub fn risk_totals(rows: &[QueueRow]) -> RiskTotals {
    let mut totals = RiskTotals::default();
    for row in rows {
        let Some(risk) = row.file.risk else {
            continue;
        };
        totals.ranked = true;
        if row.tier == Tier::Reviewed {
            continue;
        }
        totals.look_hunks += row.file.look_hunks;
        totals.routine_hunks += row.file.routine_hunks;
        match risk {
            Risk::Look => totals.look_files += 1,
            Risk::Routine => totals.routine_files += 1,
        }
    }
    totals
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAgent {
    pub session_id: String,
    pub name: String,
    /// The agent is blocked on a permission or a question for the user.
    pub waiting: bool,
    pub busy: bool,
    pub last_activity: i64,
    pub files: Vec<ChangedFile>,
}

/// `session\0path` -> the hash that was current when the user marked the file reviewed.
pub type ReviewedMarks = BTreeMap<String, String>;

pub fn review_key(session_id: &str, path: &str) -> String {
    format!("{session_id}\0{path}")
}

pub fn is_reviewed(marks: &ReviewedMarks, session_id: &str, path: &str, hash: &str) -> bool {
    marks
        .get(&review_key(session_id, path))
        .is_some_and(|mark| mark == hash)
}

/// Marks a file reviewed at its current hash. Returns whether anything changed.
pub fn mark_reviewed(marks: &mut ReviewedMarks, session_id: &str, path: &str, hash: &str) -> bool {
    let key = review_key(session_id, path);
    if marks.get(&key).is_some_and(|mark| mark == hash) {
        return false;
    }
    marks.insert(key, hash.to_string());
    true
}

pub fn unmark_reviewed(marks: &mut ReviewedMarks, session_id: &str, path: &str) -> bool {
    marks.remove(&review_key(session_id, path)).is_some()
}

/// Drops every mark whose file changed again (hash differs) or left the change set. `live` only
/// needs the sessions that were actually loaded: a session absent from it keeps its marks, so a
/// failed fetch does not wipe the user's review progress. Returns whether anything was dropped.
pub fn prune_reviewed(
    marks: &mut ReviewedMarks,
    live: &BTreeMap<String, BTreeMap<String, String>>,
) -> bool {
    let before = marks.len();
    marks.retain(|key, hash| {
        let Some((session, path)) = key.split_once('\0') else {
            return false;
        };
        match live.get(session) {
            Some(files) => files.get(path) == Some(hash),
            None => true,
        }
    });
    marks.len() != before
}

/// The persisted shape of the marks: a flat JSON object.
pub fn marks_to_json(marks: &ReviewedMarks, cap: usize) -> String {
    let skip = marks.len().saturating_sub(cap);
    let kept: BTreeMap<&String, &String> = marks.iter().skip(skip).collect();
    serde_json::to_string(&kept).unwrap_or_else(|_| "{}".to_string())
}

pub fn marks_from_json(raw: &str) -> ReviewedMarks {
    serde_json::from_str::<BTreeMap<String, serde_json::Value>>(raw)
        .map(|map| {
            map.into_iter()
                .filter_map(|(key, value)| match value {
                    serde_json::Value::String(hash) if key.contains('\0') => Some((key, hash)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueRow {
    /// Stable row key: `session\0path`.
    pub key: String,
    pub session_id: String,
    pub path: String,
    pub file: ChangedFile,
    pub tier: Tier,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueGroup {
    pub session_id: String,
    pub name: String,
    /// The most urgent tier among the group's files; groups sort by it.
    pub tier: Tier,
    pub waiting: bool,
    pub busy: bool,
    pub adds: usize,
    pub dels: usize,
    pub to_review: usize,
    pub reviewed: usize,
    /// One plain line: what this agent left for you.
    pub summary: String,
    pub rows: Vec<QueueRow>,
}

fn tier_of(agent: &QueueAgent, file: &ChangedFile, marks: &ReviewedMarks) -> Tier {
    if agent.waiting {
        Tier::Waiting
    } else if file.conflicted {
        Tier::Conflict
    } else if file.failing {
        Tier::Failing
    } else if is_reviewed(marks, &agent.session_id, &file.path, &file.hash) {
        Tier::Reviewed
    } else {
        Tier::Unreviewed
    }
}

pub fn summarize_group(
    to_review: usize,
    reviewed: usize,
    adds: usize,
    dels: usize,
    waiting: bool,
    conflicts: usize,
) -> String {
    let total = to_review + reviewed;
    if total == 0 {
        return "No changes yet".to_string();
    }
    let files = format!("{total} {}", if total == 1 { "file" } else { "files" });
    let lead = if waiting {
        "Waiting for you".to_string()
    } else if conflicts > 0 {
        format!(
            "{conflicts} {}",
            if conflicts == 1 {
                "conflict"
            } else {
                "conflicts"
            }
        )
    } else if to_review == 0 {
        "All reviewed".to_string()
    } else {
        format!("{to_review} to review")
    };
    format!("{lead} · {files} · +{adds} −{dels}")
}

/// Groups with no changed files are left out: an empty group is not something to review. `scope`
/// limits to one agent; `None` is "All agents".
pub fn build_queue(
    agents: &[QueueAgent],
    marks: &ReviewedMarks,
    scope: Option<&str>,
) -> Vec<QueueGroup> {
    let mut groups: Vec<QueueGroup> = Vec::new();
    for agent in agents {
        if scope.is_some_and(|only| only != agent.session_id) {
            continue;
        }
        if agent.files.is_empty() {
            continue;
        }
        let mut rows: Vec<QueueRow> = agent
            .files
            .iter()
            .map(|file| QueueRow {
                key: review_key(&agent.session_id, &file.path),
                session_id: agent.session_id.clone(),
                path: file.path.clone(),
                file: file.clone(),
                tier: tier_of(agent, file, marks),
            })
            .collect();
        rows.sort_by(|a, b| {
            a.tier
                .cmp(&b.tier)
                .then_with(|| a.file.risk_rank().cmp(&b.file.risk_rank()))
                .then_with(|| (b.file.adds + b.file.dels).cmp(&(a.file.adds + a.file.dels)))
                .then_with(|| a.path.cmp(&b.path))
        });
        let (mut adds, mut dels, mut reviewed, mut conflicts) = (0, 0, 0, 0);
        for row in &rows {
            adds += row.file.adds;
            dels += row.file.dels;
            if row.tier == Tier::Reviewed {
                reviewed += 1;
            }
            if row.file.conflicted {
                conflicts += 1;
            }
        }
        let to_review = rows.len() - reviewed;
        groups.push(QueueGroup {
            session_id: agent.session_id.clone(),
            name: agent.name.clone(),
            tier: rows[0].tier,
            waiting: agent.waiting,
            busy: agent.busy,
            adds,
            dels,
            to_review,
            reviewed,
            summary: summarize_group(to_review, reviewed, adds, dels, agent.waiting, conflicts),
            rows,
        });
    }
    let activity: BTreeMap<&str, i64> = agents
        .iter()
        .map(|agent| (agent.session_id.as_str(), agent.last_activity))
        .collect();
    groups.sort_by(|a, b| {
        let last = |id: &str| activity.get(id).copied().unwrap_or(0);
        a.tier
            .cmp(&b.tier)
            .then_with(|| last(&b.session_id).cmp(&last(&a.session_id)))
            .then_with(|| a.name.cmp(&b.name))
    });
    groups
}

/// Every row in the order j/k walks them.
pub fn flatten_queue(groups: &[QueueGroup]) -> Vec<QueueRow> {
    groups
        .iter()
        .flat_map(|group| group.rows.iter().cloned())
        .collect()
}

/// The row after (`step` 1) or before (`step` -1) `current`; never wraps: j at the end stays.
pub fn step_queue<'a>(
    rows: &'a [QueueRow],
    current_key: Option<&str>,
    step: i32,
) -> Option<&'a QueueRow> {
    if rows.is_empty() {
        return None;
    }
    let index = current_key.and_then(|key| rows.iter().position(|row| row.key == key));
    match index {
        None if step >= 0 => rows.first(),
        None => rows.last(),
        Some(index) => {
            let next = (index as i64 + i64::from(step)).clamp(0, rows.len() as i64 - 1);
            rows.get(next as usize)
        }
    }
}

/// After marking `current_key` reviewed, the next row that still needs a look.
pub fn next_to_review<'a>(rows: &'a [QueueRow], current_key: &str) -> Option<&'a QueueRow> {
    let index = rows.iter().position(|row| row.key == current_key)?;
    (1..rows.len())
        .map(|step| &rows[(index + step) % rows.len()])
        .find(|row| row.tier != Tier::Reviewed && row.key != current_key)
}

/// The queue's content hash when the daemon gave none (never in practice; kept for rows that
/// carry no `content_hash`): a cheap, stable string hash (djb2).
pub fn fingerprint(parts: &[&str]) -> String {
    let mut h: i32 = 5381;
    for part in parts {
        for unit in part.encode_utf16() {
            h = h
                .wrapping_shl(5)
                .wrapping_add(h)
                .wrapping_add(i32::from(unit));
        }
        h = h.wrapping_shl(5).wrapping_add(h).wrapping_add(10);
    }
    radix36(h as u32)
}

fn radix36(mut n: u32) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(char::from_digit(n % 36, 36).unwrap_or('0'));
        n /= 36;
    }
    out.iter().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> ChangedFile {
        ChangedFile::new(path, "M", 1, 0, &format!("h-{path}"))
    }

    fn agent(id: &str, files: Vec<ChangedFile>) -> QueueAgent {
        QueueAgent {
            session_id: id.to_string(),
            name: id.to_string(),
            waiting: false,
            busy: false,
            last_activity: 0,
            files,
        }
    }

    fn marked(session: &str, path: &str, hash: &str) -> ReviewedMarks {
        let mut marks = ReviewedMarks::new();
        mark_reviewed(&mut marks, session, path, hash);
        marks
    }

    #[test]
    fn orders_files_waiting_conflict_failing_unreviewed_reviewed_across_and_within_agents() {
        let marks = marked("a", "done.ts", "h-done.ts");
        let mut fail = file("fail.ts");
        fail.failing = true;
        let mut clash = file("clash.ts");
        clash.conflicted = true;
        let mut needs = agent("b", vec![file("needs.ts")]);
        needs.waiting = true;
        let groups = build_queue(
            &[
                agent("a", vec![file("done.ts"), file("plain.ts"), fail, clash]),
                needs,
            ],
            &marks,
            None,
        );
        let ids: Vec<&str> = groups.iter().map(|g| g.session_id.as_str()).collect();
        assert_eq!(ids, ["b", "a"]);
        assert_eq!(groups[0].tier, Tier::Waiting);
        let rows: Vec<(&str, Tier)> = groups[1]
            .rows
            .iter()
            .map(|r| (r.path.as_str(), r.tier))
            .collect();
        assert_eq!(
            rows,
            [
                ("clash.ts", Tier::Conflict),
                ("fail.ts", Tier::Failing),
                ("plain.ts", Tier::Unreviewed),
                ("done.ts", Tier::Reviewed),
            ]
        );
    }

    #[test]
    fn a_waiting_agents_reviewed_file_still_leads_because_the_agent_is_the_blocker() {
        let marks = marked("w", "x.ts", "h-x.ts");
        let mut waiting = agent("w", vec![file("x.ts")]);
        waiting.waiting = true;
        let groups = build_queue(&[waiting], &marks, None);
        assert_eq!(groups[0].rows[0].tier, Tier::Waiting);
    }

    #[test]
    fn ties_inside_a_tier_break_by_churn_then_path() {
        let mut b = file("b.ts");
        b.adds = 2;
        let mut a = file("a.ts");
        a.adds = 2;
        let mut big = file("big.ts");
        big.adds = 30;
        big.dels = 4;
        let groups = build_queue(&[agent("a", vec![b, a, big])], &ReviewedMarks::new(), None);
        let paths: Vec<&str> = groups[0].rows.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(paths, ["big.ts", "a.ts", "b.ts"]);
    }

    #[test]
    fn group_ties_break_by_recent_activity_and_empty_groups_are_dropped() {
        let mut old = agent("old", vec![file("a.ts")]);
        old.last_activity = 1;
        let mut new = agent("new", vec![file("b.ts")]);
        new.last_activity = 9;
        let groups = build_queue(
            &[old, new, agent("empty", vec![])],
            &ReviewedMarks::new(),
            None,
        );
        let ids: Vec<&str> = groups.iter().map(|g| g.session_id.as_str()).collect();
        assert_eq!(ids, ["new", "old"]);
    }

    #[test]
    fn scopes_to_one_agent() {
        let groups = build_queue(
            &[
                agent("a", vec![file("a.ts")]),
                agent("b", vec![file("b.ts")]),
            ],
            &ReviewedMarks::new(),
            Some("b"),
        );
        let ids: Vec<&str> = groups.iter().map(|g| g.session_id.as_str()).collect();
        assert_eq!(ids, ["b"]);
    }

    #[test]
    fn writes_a_plain_one_line_summary() {
        let marks = marked("a", "a.ts", "h-a.ts");
        let mut one = file("a.ts");
        one.adds = 4;
        one.dels = 1;
        let mut two = file("b.ts");
        two.adds = 2;
        two.dels = 2;
        let groups = build_queue(&[agent("a", vec![one, two])], &marks, None);
        assert_eq!(groups[0].summary, "1 to review · 2 files · +6 −3");
        let done = build_queue(&[agent("a", vec![file("a.ts")])], &marks, None);
        assert!(done[0].summary.contains("All reviewed"));
    }

    #[test]
    fn honours_a_mark_only_while_the_content_hash_is_unchanged() {
        let marks = marked("s", "f.ts", "v1");
        assert!(is_reviewed(&marks, "s", "f.ts", "v1"));
        assert!(!is_reviewed(&marks, "s", "f.ts", "v2"));
        assert!(!is_reviewed(&marks, "other", "f.ts", "v1"));
    }

    #[test]
    fn a_file_edited_again_drops_back_to_unreviewed_in_the_queue() {
        let marks = marked("s", "f.ts", "v1");
        let at = |hash: &str| {
            let mut f = file("f.ts");
            f.hash = hash.to_string();
            build_queue(&[agent("s", vec![f])], &marks, None)
        };
        assert_eq!(at("v1")[0].rows[0].tier, Tier::Reviewed);
        assert_eq!(at("v2")[0].rows[0].tier, Tier::Unreviewed);
    }

    #[test]
    fn prunes_marks_whose_hash_moved_or_whose_file_left_and_keeps_unloaded_sessions() {
        let mut marks = marked("s", "kept.ts", "a");
        mark_reviewed(&mut marks, "s", "moved.ts", "a");
        mark_reviewed(&mut marks, "s", "gone.ts", "a");
        mark_reviewed(&mut marks, "unloaded", "x.ts", "a");
        let mut files = BTreeMap::new();
        files.insert("kept.ts".to_string(), "a".to_string());
        files.insert("moved.ts".to_string(), "b".to_string());
        let mut live = BTreeMap::new();
        live.insert("s".to_string(), files);
        assert!(prune_reviewed(&mut marks, &live));
        let mut keys: Vec<String> = marks.keys().cloned().collect();
        keys.sort();
        let mut want = vec![review_key("s", "kept.ts"), review_key("unloaded", "x.ts")];
        want.sort();
        assert_eq!(keys, want);
    }

    #[test]
    fn nothing_to_prune_or_unmark_reports_no_change() {
        let mut marks = marked("s", "a.ts", "a");
        let mut files = BTreeMap::new();
        files.insert("a.ts".to_string(), "a".to_string());
        let mut live = BTreeMap::new();
        live.insert("s".to_string(), files);
        assert!(!prune_reviewed(&mut marks, &live));
        assert!(!unmark_reviewed(&mut marks, "s", "nope.ts"));
        assert!(unmark_reviewed(&mut marks, "s", "a.ts"));
        assert!(marks.is_empty());
    }

    fn walk_rows() -> Vec<QueueRow> {
        let mut a1 = file("a1.ts");
        a1.adds = 9;
        let mut a2 = file("a2.ts");
        a2.adds = 5;
        let mut b = agent("b", vec![file("b1.ts")]);
        b.last_activity = -1;
        flatten_queue(&build_queue(
            &[agent("a", vec![a1, a2]), b],
            &ReviewedMarks::new(),
            None,
        ))
    }

    #[test]
    fn steps_without_wrapping_and_starts_at_an_end_when_nothing_is_open() {
        let rows = walk_rows();
        assert_eq!(
            step_queue(&rows, None, 1).map(|r| r.path.as_str()),
            Some("a1.ts")
        );
        assert_eq!(
            step_queue(&rows, None, -1).map(|r| r.path.as_str()),
            Some("b1.ts")
        );
        assert_eq!(
            step_queue(&rows, Some(&rows[0].key), -1).map(|r| r.path.as_str()),
            Some("a1.ts")
        );
        assert_eq!(
            step_queue(&rows, Some(&rows[2].key), 1).map(|r| r.path.as_str()),
            Some("b1.ts")
        );
        assert!(step_queue(&[], None, 1).is_none());
    }

    #[test]
    fn after_review_jumps_to_the_next_file_that_still_needs_a_look() {
        let marks = marked("a", "a2.ts", "h-a2.ts");
        let mut a1 = file("a1.ts");
        a1.adds = 9;
        let mut a2 = file("a2.ts");
        a2.adds = 5;
        let mut b = agent("b", vec![file("b1.ts")]);
        b.last_activity = -1;
        let rows = flatten_queue(&build_queue(&[agent("a", vec![a1, a2]), b], &marks, None));
        assert_eq!(
            next_to_review(&rows, &rows[0].key).map(|r| r.path.as_str()),
            Some("b1.ts")
        );
        assert!(next_to_review(&rows[..1], &rows[0].key).is_none());
    }

    #[test]
    fn fingerprint_is_stable_and_sensitive_to_content_and_boundaries() {
        assert_eq!(fingerprint(&["a", "b"]), fingerprint(&["a", "b"]));
        assert_ne!(fingerprint(&["ab"]), fingerprint(&["a", "b"]));
    }

    #[test]
    fn marks_round_trip_through_json_and_ignore_junk() {
        let marks = marked("s", "f.ts", "v1");
        let json = marks_to_json(&marks, 10);
        assert_eq!(marks_from_json(&json), marks);
        assert!(marks_from_json("not json").is_empty());
        assert!(marks_from_json(r#"{"no-nul":"x","a\u0000b":1}"#).is_empty());
    }

    #[test]
    fn the_persisted_blob_keeps_only_the_newest_cap() {
        let mut marks = ReviewedMarks::new();
        for n in 0..5 {
            mark_reviewed(&mut marks, "s", &format!("f{n}"), "h");
        }
        assert_eq!(marks_from_json(&marks_to_json(&marks, 2)).len(), 2);
    }
}
