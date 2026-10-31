use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::*;
use crate::app::{App, KeyKind};

fn todo(id: &str, text: &str, done: bool) -> TodoView {
    TodoView {
        id: id.into(),
        text: text.into(),
        done,
        thread: None,
    }
}

fn loaded() -> NotesOverlay {
    let mut m = NotesOverlay::default();
    m.open();
    m.apply(NotesSnapshot {
        project: "/w/app".into(),
        body: "# plan\nship the notes".into(),
        todos: vec![
            todo("a", "fix the flaky test", false),
            todo("b", "write the docs", false),
            TodoView {
                thread: Some("abcd1234".into()),
                ..todo("c", "old thing", true)
            },
        ],
    });
    m
}

fn type_text(m: &mut NotesOverlay, text: &str) {
    for c in text.chars() {
        m.key(KeyKind::Char(c));
    }
}

#[test]
fn opening_asks_for_a_load_once() {
    let mut m = NotesOverlay::default();
    m.open();
    assert!(m.take_reload());
    assert!(!m.take_reload());
}

#[test]
fn space_ticks_the_focused_item_and_enter_starts_a_thread() {
    let mut m = loaded();
    m.key(KeyKind::Down);
    assert_eq!(
        m.key(KeyKind::Char(' ')),
        NotesAction::Toggle {
            id: "b".into(),
            done: true
        }
    );
    assert_eq!(
        m.key(KeyKind::Enter),
        NotesAction::StartThread {
            id: "b".into(),
            text: "write the docs".into()
        }
    );
}

#[test]
fn a_linked_item_names_its_thread_and_t_starts_another() {
    let mut m = loaded();
    m.apply(NotesSnapshot {
        project: "/w/app".into(),
        body: String::new(),
        todos: vec![TodoView {
            thread: Some("abcd1234".into()),
            ..todo("a", "fix the flaky test", false)
        }],
    });
    assert_eq!(m.key(KeyKind::Enter), NotesAction::None);
    assert!(m.notice.as_ref().unwrap().text.contains("abcd1234"));
    assert_eq!(
        m.key(KeyKind::Char('t')),
        NotesAction::StartThread {
            id: "a".into(),
            text: "fix the flaky test".into()
        }
    );
}

#[test]
fn a_finished_item_will_not_start_a_thread() {
    let mut m = loaded();
    m.key(KeyKind::End);
    assert_eq!(m.key(KeyKind::Enter), NotesAction::None);
    assert!(m.notice.is_some());
    assert_eq!(
        m.key(KeyKind::Char('x')),
        NotesAction::Toggle {
            id: "c".into(),
            done: false
        }
    );
}

#[test]
fn adding_types_a_line_and_blank_input_adds_nothing() {
    let mut m = loaded();
    m.key(KeyKind::Char('a'));
    assert!(m.typing());
    type_text(&mut m, "q ship it");
    assert_eq!(m.key(KeyKind::Enter), NotesAction::Add("q ship it".into()));
    assert!(!m.typing());

    m.key(KeyKind::Char('a'));
    type_text(&mut m, "   ");
    assert_eq!(m.key(KeyKind::Enter), NotesAction::None);
    m.key(KeyKind::Char('a'));
    type_text(&mut m, "nope");
    assert_eq!(m.key(KeyKind::Esc), NotesAction::None);
    assert!(m.input.is_empty());
}

#[test]
fn editing_starts_from_the_current_text() {
    let mut m = loaded();
    m.key(KeyKind::Char('e'));
    assert_eq!(m.input, "fix the flaky test");
    m.key(KeyKind::Backspace);
    type_text(&mut m, "t!");
    assert_eq!(
        m.key(KeyKind::Enter),
        NotesAction::Edit {
            id: "a".into(),
            text: "fix the flaky tes".to_string() + "t!"
        }
    );
}

#[test]
fn delete_needs_a_yes() {
    let mut m = loaded();
    m.key(KeyKind::Char('d'));
    assert_eq!(m.key(KeyKind::Char('n')), NotesAction::None);
    m.key(KeyKind::Char('d'));
    assert_eq!(m.key(KeyKind::Char('y')), NotesAction::Delete("a".into()));
}

#[test]
fn the_scratchpad_saves_on_escape_and_discards_on_ctrl_c() {
    let mut m = loaded();
    m.key(KeyKind::Char('p'));
    assert_eq!(m.input, "# plan\nship the notes");
    m.key(KeyKind::Enter);
    type_text(&mut m, "more");
    assert_eq!(
        m.key(KeyKind::Esc),
        NotesAction::SavePad("# plan\nship the notes\nmore".into())
    );

    m.key(KeyKind::Char('p'));
    m.key(KeyKind::Char('z'));
    assert_eq!(m.key(KeyKind::Interrupt), NotesAction::None);
    assert!(m.open);

    m.key(KeyKind::Char('p'));
    assert_eq!(
        m.key(KeyKind::Esc),
        NotesAction::None,
        "unchanged text is not a write"
    );
}

#[test]
fn q_closes_when_browsing_but_types_when_a_field_is_open() {
    let mut m = loaded();
    m.key(KeyKind::Char('a'));
    m.key(KeyKind::Char('q'));
    assert!(m.open);
    assert_eq!(m.input, "q");
    m.key(KeyKind::Esc);
    assert_eq!(m.key(KeyKind::Char('q')), NotesAction::Close);
    assert!(!m.open);
}

#[test]
fn the_cursor_stays_in_range_when_items_go() {
    let mut m = loaded();
    m.key(KeyKind::End);
    assert_eq!(m.cursor, 2);
    m.apply(NotesSnapshot {
        project: "/w/app".into(),
        body: String::new(),
        todos: vec![todo("a", "only", false)],
    });
    assert_eq!(m.cursor, 0);
}

fn screen(m: NotesOverlay, w: u16, h: u16) -> String {
    let mut app = App::default();
    app.notes_overlay = m;
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal
        .draw(|f| render_notes_overlay(f, &app, f.area()))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn it_renders_the_list_the_pad_and_the_keys_at_80x24() {
    let text = screen(loaded(), 80, 24);
    assert!(text.contains("Notes"), "{text}");
    assert!(text.contains("app"), "{text}");
    assert!(text.contains("2 open"), "{text}");
    assert!(text.contains("fix the flaky test"), "{text}");
    assert!(text.contains("write the docs"), "{text}");
    assert!(text.contains("abcd1234"), "{text}");
    assert!(text.contains("# plan"), "{text}");
    assert!(text.contains("Space tick"), "{text}");
    assert!(text.contains("Esc close"), "{text}");
    assert!(
        !text.contains("p pad"),
        "narrow cards drop the later hints: {text}"
    );
}

#[test]
fn it_renders_the_add_line_and_an_empty_state() {
    let mut m = loaded();
    m.key(KeyKind::Char('a'));
    type_text(&mut m, "buy milk");
    assert!(screen(m, 80, 24).contains("add: buy milk"));

    let mut empty = NotesOverlay::default();
    empty.open();
    empty.apply(NotesSnapshot {
        project: "/w/app".into(),
        ..NotesSnapshot::default()
    });
    let text = screen(empty, 100, 30);
    assert!(text.contains("Nothing on the list"), "{text}");
    assert!(text.contains("Empty. Press p"), "{text}");
}

#[test]
fn it_survives_a_tiny_terminal() {
    let _ = screen(loaded(), 24, 8);
    let _ = screen(loaded(), 10, 4);
}
