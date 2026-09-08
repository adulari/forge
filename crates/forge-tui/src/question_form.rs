//! The interactive `ask_user` form: one or more questions answered in place with arrow keys,
//! checkboxes / radio buttons, an "Other…" free-text row and an optional note per question
//! (docs/features/ask-user-question.md).
//!
//! Pure state: no terminal, no channels. The render module draws it and the key loops feed it
//! [`KeyKind`]s through [`QuestionForm::key`]; the same code serves the TUI, the daemon's
//! remote-key router and the direct `TuiPresenter`.

use std::collections::BTreeSet;

use forge_types::{Answer, Question};

use crate::app::{handle_key, InputOutcome, KeyKind};

/// What the user is typing into, when the input line is armed by the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Editing {
    Other,
    Note,
}

/// Which row of the current question the cursor is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Option(usize),
    Other,
    Note,
}

/// The user's in-progress answer to one question.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Draft {
    pub selected: BTreeSet<usize>,
    pub other: String,
    pub note: String,
}

impl Draft {
    fn is_answered(&self) -> bool {
        !self.selected.is_empty() || !self.other.trim().is_empty()
    }

    fn to_answer(&self, q: &Question) -> Answer {
        Answer {
            selected: self
                .selected
                .iter()
                .filter_map(|&i| q.options.get(i).map(|o| o.label.clone()))
                .collect(),
            other: Some(self.other.trim().to_string()).filter(|t| !t.is_empty()),
            note: Some(self.note.trim().to_string()).filter(|t| !t.is_empty()),
        }
    }
}

/// What a key did to the form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormOutcome {
    /// Still open (redraw).
    Open,
    /// Every question answered — deliver these.
    Submit(Vec<Answer>),
    /// The user dismissed the form (Esc): no answers.
    Cancel,
}

#[derive(Debug, Clone)]
pub struct QuestionForm {
    pub questions: Vec<Question>,
    pub drafts: Vec<Draft>,
    pub current: usize,
    pub cursor: Row,
    pub editing: Option<Editing>,
    /// Text of the row being edited (the app's input line is left alone so a queued prompt the
    /// user was typing survives the question).
    pub buffer: String,
    pub buffer_cursor: usize,
    /// A one-frame hint after a rejected key ("choose at least one option").
    pub flash: Option<&'static str>,
}

impl QuestionForm {
    pub fn new(questions: Vec<Question>) -> Self {
        let drafts = vec![Draft::default(); questions.len()];
        let mut form = Self {
            questions,
            drafts,
            current: 0,
            cursor: Row::Option(0),
            editing: None,
            buffer: String::new(),
            buffer_cursor: 0,
            flash: None,
        };
        form.cursor = form.first_row();
        form
    }

    pub fn question(&self) -> &Question {
        &self.questions[self.current]
    }

    pub fn draft(&self) -> &Draft {
        &self.drafts[self.current]
    }

    pub fn len(&self) -> usize {
        self.questions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.questions.is_empty()
    }

    /// Whether question `i` has an answer (for the tab strip ✓ marks).
    pub fn answered(&self, i: usize) -> bool {
        self.drafts.get(i).is_some_and(Draft::is_answered)
    }

    /// The rows of the current question in display order.
    pub fn rows(&self) -> Vec<Row> {
        let q = self.question();
        let mut rows: Vec<Row> = (0..q.options.len()).map(Row::Option).collect();
        if q.allow_other {
            rows.push(Row::Other);
        }
        if q.allow_note {
            rows.push(Row::Note);
        }
        rows
    }

    fn first_row(&self) -> Row {
        self.rows().first().copied().unwrap_or(Row::Other)
    }

    fn move_cursor(&mut self, delta: isize) {
        let rows = self.rows();
        let Some(pos) = rows.iter().position(|r| *r == self.cursor) else {
            self.cursor = self.first_row();
            return;
        };
        let n = rows.len() as isize;
        let next = (pos as isize + delta).rem_euclid(n.max(1));
        self.cursor = rows[next as usize];
    }

    fn goto(&mut self, index: usize) {
        self.current = index.min(self.questions.len().saturating_sub(1));
        self.editing = None;
        self.flash = None;
        // Land on the first chosen option so re-visiting a question shows its answer.
        self.cursor = self
            .draft()
            .selected
            .iter()
            .next()
            .map(|&i| Row::Option(i))
            .unwrap_or_else(|| self.first_row());
    }

    /// Select option `i` on a single-select question, or toggle it on a multi-select one.
    fn choose(&mut self, i: usize) {
        let multi = self.question().multi;
        let draft = &mut self.drafts[self.current];
        if multi {
            if !draft.selected.remove(&i) {
                draft.selected.insert(i);
            }
        } else {
            draft.selected.clear();
            draft.selected.insert(i);
            draft.other.clear();
        }
        self.cursor = Row::Option(i);
    }

    /// Move to the next question, or submit after the last one.
    fn advance(&mut self) -> FormOutcome {
        if self.current + 1 < self.questions.len() {
            self.goto(self.current + 1);
            return FormOutcome::Open;
        }
        // Submitting from the last question: park on the first unanswered one instead of sending
        // a half-empty form the user did not notice.
        if let Some(missing) = (0..self.questions.len()).find(|&i| !self.answered(i)) {
            if missing != self.current {
                self.goto(missing);
                self.flash = Some("this question still needs an answer");
                return FormOutcome::Open;
            }
        }
        FormOutcome::Submit(self.answers())
    }

    pub fn answers(&self) -> Vec<Answer> {
        self.questions
            .iter()
            .zip(&self.drafts)
            .map(|(q, d)| d.to_answer(q))
            .collect()
    }

    fn start_editing(&mut self, what: Editing) {
        let draft = self.draft();
        self.buffer = match what {
            Editing::Other => draft.other.clone(),
            Editing::Note => draft.note.clone(),
        };
        self.buffer_cursor = self.buffer.len();
        self.editing = Some(what);
        self.cursor = match what {
            Editing::Other => Row::Other,
            Editing::Note => Row::Note,
        };
    }

    fn commit_edit(&mut self) -> FormOutcome {
        let Some(what) = self.editing.take() else {
            return FormOutcome::Open;
        };
        let text = std::mem::take(&mut self.buffer);
        self.buffer_cursor = 0;
        let single = !self.question().multi;
        let draft = &mut self.drafts[self.current];
        match what {
            Editing::Other => {
                draft.other = text;
                if single && !draft.other.trim().is_empty() {
                    draft.selected.clear();
                }
                if draft.is_answered() {
                    return self.advance();
                }
                FormOutcome::Open
            }
            Editing::Note => {
                draft.note = text;
                FormOutcome::Open
            }
        }
    }

    /// Feed one key. Returns what happened; the caller redraws on `Open` and tears the form
    /// down on `Submit`/`Cancel`.
    pub fn key(&mut self, key: KeyKind) -> FormOutcome {
        self.flash = None;
        if self.editing.is_some() {
            return match key {
                KeyKind::Esc => {
                    self.editing = None;
                    self.buffer.clear();
                    self.buffer_cursor = 0;
                    FormOutcome::Open
                }
                KeyKind::Enter => self.commit_edit(),
                KeyKind::Up | KeyKind::Down | KeyKind::Tab | KeyKind::CycleTemper => {
                    // Leaving the field keeps what was typed.
                    let what = self.editing;
                    let outcome = self.commit_edit();
                    if what == Some(Editing::Note) && outcome == FormOutcome::Open {
                        self.move_cursor(if matches!(key, KeyKind::Up) { -1 } else { 1 });
                    }
                    outcome
                }
                other => match handle_key(&mut self.buffer, &mut self.buffer_cursor, other) {
                    InputOutcome::Submit(_) => self.commit_edit(),
                    InputOutcome::Quit => FormOutcome::Cancel,
                    InputOutcome::Editing => FormOutcome::Open,
                },
            };
        }
        match key {
            KeyKind::Esc | KeyKind::Interrupt => FormOutcome::Cancel,
            KeyKind::Up | KeyKind::Char('k') => {
                self.move_cursor(-1);
                FormOutcome::Open
            }
            KeyKind::Down | KeyKind::Char('j') => {
                self.move_cursor(1);
                FormOutcome::Open
            }
            KeyKind::Home => {
                self.cursor = self.first_row();
                FormOutcome::Open
            }
            KeyKind::End => {
                self.cursor = self.rows().last().copied().unwrap_or(Row::Other);
                FormOutcome::Open
            }
            KeyKind::Left | KeyKind::CycleTemper => {
                if self.current > 0 {
                    self.goto(self.current - 1);
                }
                FormOutcome::Open
            }
            KeyKind::Right | KeyKind::Tab => {
                if self.current + 1 < self.questions.len() {
                    self.goto(self.current + 1);
                }
                FormOutcome::Open
            }
            KeyKind::Char(c @ '1'..='9') => {
                let i = c as usize - '1' as usize;
                if i < self.question().options.len() {
                    let single = !self.question().multi;
                    self.choose(i);
                    if single {
                        return self.advance();
                    }
                }
                FormOutcome::Open
            }
            KeyKind::Char(' ') => {
                match self.cursor {
                    Row::Option(i) => self.choose(i),
                    Row::Other => self.start_editing(Editing::Other),
                    Row::Note => self.start_editing(Editing::Note),
                }
                FormOutcome::Open
            }
            KeyKind::Char('o') if self.question().allow_other => {
                self.start_editing(Editing::Other);
                FormOutcome::Open
            }
            KeyKind::Char('n') if self.question().allow_note => {
                self.start_editing(Editing::Note);
                FormOutcome::Open
            }
            KeyKind::Enter => match self.cursor {
                Row::Option(i) => {
                    let multi = self.question().multi;
                    if !multi || !self.draft().is_answered() {
                        self.choose(i);
                    }
                    self.advance()
                }
                Row::Other => {
                    self.start_editing(Editing::Other);
                    FormOutcome::Open
                }
                Row::Note => {
                    if self.draft().is_answered() {
                        self.advance()
                    } else {
                        self.flash = Some("choose an option (or type another answer) first");
                        FormOutcome::Open
                    }
                }
            },
            _ => FormOutcome::Open,
        }
    }

    /// Resolve a remote answer for the CURRENT question in the legacy text shape (an option
    /// number/label, or free text). `true` when it applied.
    pub fn apply_text(&mut self, text: &str) -> bool {
        let q = self.question().clone();
        let Some(label) = crate::resolve_answer(text, &q.options, q.allow_other) else {
            return false;
        };
        let draft = &mut self.drafts[self.current];
        draft.selected.clear();
        draft.other.clear();
        match q.options.iter().position(|o| o.label == label) {
            Some(i) => {
                draft.selected.insert(i);
            }
            None => draft.other = label,
        }
        true
    }

    /// Resolve a remote answer for the whole form: `{"answers":[{selected,other,note},…]}`.
    /// Labels are matched to options; unknown labels become the free-text answer.
    pub fn apply_structured(&mut self, answers: &[Answer]) {
        for (i, a) in answers.iter().enumerate().take(self.questions.len()) {
            let q = &self.questions[i];
            let draft = &mut self.drafts[i];
            draft.selected = a
                .selected
                .iter()
                .filter_map(|l| q.options.iter().position(|o| &o.label == l))
                .collect();
            let unknown: Vec<&str> = a
                .selected
                .iter()
                .filter(|l| !q.options.iter().any(|o| &&o.label == l))
                .map(String::as_str)
                .collect();
            let mut other = a.other.clone().unwrap_or_default();
            if !unknown.is_empty() {
                if !other.is_empty() {
                    other.push_str(", ");
                }
                other.push_str(&unknown.join(", "));
            }
            draft.other = other;
            draft.note = a.note.clone().unwrap_or_default();
        }
    }
}

/// Parse the structured remote answer body, if `text` is one.
pub fn parse_structured_answers(text: &str) -> Option<Vec<Answer>> {
    #[derive(serde::Deserialize)]
    struct Body {
        answers: Vec<Answer>,
    }
    let t = text.trim();
    if !t.starts_with('{') {
        return None;
    }
    serde_json::from_str::<Body>(t).ok().map(|b| b.answers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::QChoice;

    fn opt(label: &str) -> QChoice {
        QChoice {
            label: label.into(),
            description: String::new(),
        }
    }

    fn single() -> Question {
        Question {
            allow_note: true,
            ..Question::single("Which DB?", &[opt("Postgres"), opt("SQLite")], true)
        }
    }

    fn multi() -> Question {
        Question {
            header: "Scope".into(),
            multi: true,
            ..Question::single(
                "Which features?",
                &[opt("Auth"), opt("Billing"), opt("Search")],
                true,
            )
        }
    }

    #[test]
    fn enter_on_an_option_answers_a_single_select_question_and_submits() {
        let mut f = QuestionForm::new(vec![single()]);
        assert_eq!(f.key(KeyKind::Down), FormOutcome::Open);
        match f.key(KeyKind::Enter) {
            FormOutcome::Submit(a) => assert_eq!(a[0].selected, ["SQLite"]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn digits_pick_directly_and_space_toggles_on_multi_select() {
        let mut f = QuestionForm::new(vec![multi(), single()]);
        assert_eq!(
            f.key(KeyKind::Char('1')),
            FormOutcome::Open,
            "multi: digit toggles, stays"
        );
        assert_eq!(f.key(KeyKind::Down), FormOutcome::Open);
        assert_eq!(f.key(KeyKind::Down), FormOutcome::Open);
        assert_eq!(f.key(KeyKind::Char(' ')), FormOutcome::Open);
        assert_eq!(
            f.draft().selected.iter().copied().collect::<Vec<_>>(),
            [0, 2]
        );
        assert_eq!(
            f.key(KeyKind::Enter),
            FormOutcome::Open,
            "confirm → next question"
        );
        assert_eq!(f.current, 1);
        match f.key(KeyKind::Char('2')) {
            FormOutcome::Submit(a) => {
                assert_eq!(a[0].selected, ["Auth", "Search"]);
                assert_eq!(a[1].selected, ["SQLite"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn other_is_typed_inline_and_replaces_the_choice_on_single_select() {
        let mut f = QuestionForm::new(vec![single()]);
        f.key(KeyKind::Char('o'));
        assert_eq!(f.editing, Some(Editing::Other));
        for c in "mysql".chars() {
            f.key(KeyKind::Char(c));
        }
        match f.key(KeyKind::Enter) {
            FormOutcome::Submit(a) => {
                assert!(a[0].selected.is_empty());
                assert_eq!(a[0].other.as_deref(), Some("mysql"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_note_rides_along_with_the_choice() {
        let mut f = QuestionForm::new(vec![single()]);
        f.key(KeyKind::Char('n'));
        for c in "managed please".chars() {
            f.key(KeyKind::Char(c));
        }
        assert_eq!(
            f.key(KeyKind::Enter),
            FormOutcome::Open,
            "note saved, still on question"
        );
        assert_eq!(f.editing, None);
        f.key(KeyKind::Home);
        match f.key(KeyKind::Enter) {
            FormOutcome::Submit(a) => {
                assert_eq!(a[0].selected, ["Postgres"]);
                assert_eq!(a[0].note.as_deref(), Some("managed please"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn submitting_with_an_unanswered_question_jumps_back_to_it() {
        let mut f = QuestionForm::new(vec![single(), multi()]);
        f.key(KeyKind::Right); // skip ahead without answering
        assert_eq!(f.current, 1);
        f.key(KeyKind::Char('1'));
        assert_eq!(f.key(KeyKind::Enter), FormOutcome::Open);
        assert_eq!(f.current, 0, "parked on the unanswered question");
        assert!(f.flash.is_some());
        assert_eq!(
            f.key(KeyKind::Enter),
            FormOutcome::Open,
            "answered → on to question 2"
        );
        assert_eq!(f.current, 1);
        assert!(matches!(f.key(KeyKind::Enter), FormOutcome::Submit(_)));
    }

    #[test]
    fn esc_cancels_and_esc_while_typing_only_leaves_the_field() {
        let mut f = QuestionForm::new(vec![single()]);
        f.key(KeyKind::Char('n'));
        f.key(KeyKind::Char('x'));
        assert_eq!(f.key(KeyKind::Esc), FormOutcome::Open);
        assert_eq!(f.editing, None);
        assert_eq!(f.key(KeyKind::Esc), FormOutcome::Cancel);
    }

    #[test]
    fn remote_text_and_structured_answers_apply() {
        let mut f = QuestionForm::new(vec![single(), multi()]);
        assert!(f.apply_text("2"));
        assert_eq!(f.draft().selected.iter().copied().collect::<Vec<_>>(), [1]);
        assert!(!f.apply_text(""));
        let body = r#"{"answers":[{"selected":["Postgres"]},{"selected":["Auth","Kafka"],"note":"later"}]}"#;
        let answers = parse_structured_answers(body).unwrap();
        f.apply_structured(&answers);
        let out = f.answers();
        assert_eq!(out[0].selected, ["Postgres"]);
        assert_eq!(out[1].selected, ["Auth"]);
        assert_eq!(
            out[1].other.as_deref(),
            Some("Kafka"),
            "unknown label → free text"
        );
        assert_eq!(out[1].note.as_deref(), Some("later"));
        assert!(parse_structured_answers("plain").is_none());
    }
}
