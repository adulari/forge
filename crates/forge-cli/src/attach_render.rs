//! `forge attach`'s incremental terminal renderer: prints only what each new snapshot frame adds.

use std::io::Write as _;

use super::{question_is_secret, ClientOutage, ClientSetup, ClientSnapshot, Pending};

/// Incremental line-oriented renderer: the daemon re-sends a full (bounded, sliding) snapshot each
/// frame, so this prints only what's new since the last frame and never re-prints scrollback.
#[derive(Default)]
pub(super) struct Renderer {
    header_shown: bool,
    pub(super) last_revision: Option<u64>,
    pub(super) printed_transcript: Vec<String>,
    printed_notes: Vec<String>,
    shown_prompt_seq: Option<u64>,
    was_busy: bool,
    shown_setup: Option<ClientSetup>,
    shown_base: Option<String>,
    pub(super) printed_live: Vec<String>,
    pub(super) held_live: Option<String>,
    shown_outage: Option<ClientOutage>,
    /// The stored "provider down since" / "back after" notes already said so: the matching
    /// status line would print the same fact twice.
    pub(super) enter_note_printed: bool,
    pub(super) back_note_printed: bool,
}

/// The line `forge attach` prints when the outage state changes: the same wording as the local
/// status bar when a wait starts or its next probe moves, the recovery line when it ends.
pub(super) fn outage_notice(
    shown: &Option<ClientOutage>,
    now: &Option<ClientOutage>,
    now_unix: u64,
    enter_noted: bool,
    back_noted: bool,
) -> Option<String> {
    match (shown, now) {
        (None, Some(_)) if enter_noted => None,
        (Some(_), None) if back_noted => None,
        (_, Some(o)) if shown.as_ref() != Some(o) => Some(forge_tui::outage_line(
            &o.model,
            o.since_unix,
            o.next_try_unix,
            o.until_unix,
            &o.reason,
        )),
        (Some(o), None) => Some(forge_tui::recovered_line(
            &o.model,
            now_unix.saturating_sub(o.since_unix),
        )),
        _ => None,
    }
}

impl Renderer {
    pub(super) fn render(&mut self, snap: &ClientSnapshot, pending: &mut Pending) {
        // `revision` is optional for compatibility with older daemons. When present, it is the
        // authoritative ordering key; when absent, the overlap renderer below still de-duplicates.
        if let Some(next_revision) = snap.revision {
            if self
                .last_revision
                .is_some_and(|revision| next_revision <= revision)
            {
                return;
            }
            self.last_revision = Some(next_revision);
        }

        if !self.header_shown {
            let title = if snap.title.is_empty() {
                "(untitled)"
            } else {
                snap.title.as_str()
            };
            println!("── {} · {title} · {} ──", snap.session_id, snap.cwd);
            println!("   model: {}   cost: ${:.4}\n", snap.model, snap.cost_usd);
            self.header_shown = true;
        }

        for l in new_suffix(&self.printed_transcript, &snap.transcript) {
            println!("{l}");
        }
        self.printed_transcript = snap.transcript.clone();

        // A running command's rolling tail: print each line once as it arrives, indented so it
        // reads as the call's output; the finished result follows in the transcript as usual.
        // The last line may still be growing (the sampler appends the unterminated tail), so it is
        // held back until another line follows it or the call ends; only settled lines are
        // deduplicated, which keeps a redrawn last line from reprinting the whole block.
        let live = snap.live_tail.as_ref().map_or(&[][..], |t| &t.lines[..]);
        let (settled, last) = match live.split_last() {
            Some((last, settled)) => (settled, Some(last)),
            None => (&[][..], None),
        };
        for l in new_suffix(&self.printed_live, settled) {
            println!("  │ {l}");
        }
        if last.is_none() {
            if let Some(held) = self.held_live.take() {
                println!("  │ {held}");
            }
        }
        self.printed_live = settled.to_vec();
        self.held_live = last.cloned();

        if snap.worktree_base != self.shown_base {
            if let Some(base) = &snap.worktree_base {
                println!("· worktree {base}");
            }
            self.shown_base.clone_from(&snap.worktree_base);
        }

        if snap.worktree_setup != self.shown_setup {
            if let Some(setup) = &snap.worktree_setup {
                println!("{}", setup.notice());
            }
            self.shown_setup.clone_from(&snap.worktree_setup);
        }

        for n in new_suffix(&self.printed_notes, &snap.notes) {
            println!("· {n}");
            if n.contains(forge_store::OUTAGE_ENTER_MARK) && n.contains("provider down since") {
                self.enter_note_printed = true;
                self.back_note_printed = false;
            }
            if n.contains(forge_store::OUTAGE_BACK_MARK) && n.contains(" back after ") {
                self.back_note_printed = true;
            }
        }
        self.printed_notes = snap.notes.clone();

        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        if let Some(line) = outage_notice(
            &self.shown_outage,
            &snap.outage,
            now_unix,
            self.enter_note_printed,
            self.back_note_printed,
        ) {
            println!("{line}");
        }
        if snap.outage.is_none() {
            self.enter_note_printed = false;
            if self.shown_outage.is_some() {
                self.back_note_printed = false;
            }
        }
        self.shown_outage.clone_from(&snap.outage);

        // Turn boundary: announce when a turn finishes so the operator knows it's their move.
        if self.was_busy
            && !snap.busy
            && snap.permission_prompt.is_none()
            && snap.question.is_none()
        {
            println!("— turn complete (${:.4}) —", snap.cost_usd);
        }
        self.was_busy = snap.busy;

        // Permission prompt: print once per distinct prompt_seq; a new seq replaces the old.
        if let Some(body) = &snap.permission_prompt {
            if self.shown_prompt_seq != Some(snap.prompt_seq) {
                println!("\n⚠ permission needed:");
                for l in body.lines() {
                    println!("    {l}");
                }
                println!("  answer:  y  (allow)   ·   n  (deny)\n");
                self.shown_prompt_seq = Some(snap.prompt_seq);
            }
            *pending = Pending::Permission(snap.prompt_seq);
        } else if let Some(q) = &snap.question {
            if self.shown_prompt_seq != Some(snap.prompt_seq) {
                println!("\n❓ {q}");
                for (i, opt) in snap.question_options.iter().enumerate() {
                    println!("    {}. {}", i + 1, opt.label);
                }
                if question_is_secret(snap) {
                    println!("  this answer is a secret: answer it in the Forge terminal or app, not here\n");
                } else {
                    println!("  answer by typing a number or free text\n");
                }
                self.shown_prompt_seq = Some(snap.prompt_seq);
            }
            // A typed line here would be echoed into the terminal's scrollback.
            *pending = if question_is_secret(snap) {
                Pending::None
            } else {
                Pending::Question(snap.prompt_seq)
            };
        } else {
            self.shown_prompt_seq = None;
            *pending = Pending::None;
        }
        let _ = std::io::stdout().flush();
    }
}

/// The lines of `next` that follow everything already printed. Because the transcript/notes are a
/// bounded window that slides, alignment uses the longest suffix/prefix overlap. Anchoring on only
/// the last line is incorrect when common lines repeat (for example `— turn complete —`): it can
/// match the wrong occurrence and replay a large block.
pub(super) fn new_suffix(prev: &[String], next: &[String]) -> Vec<String> {
    if prev.is_empty() {
        return next.to_vec();
    }
    let max_overlap = prev.len().min(next.len());
    let overlap = (1..=max_overlap)
        .rev()
        .find(|&len| prev[prev.len() - len..] == next[..len])
        .unwrap_or(0);
    next[overlap..].to_vec()
}
