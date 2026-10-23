//! Messages typed while a turn runs stay visible above the composer until the turn takes them.

use super::*;

fn busy(queued: &[&str]) -> App {
    let mut app = App {
        fullscreen: true,
        busy: true,
        ..Default::default()
    };
    let queued: Vec<String> = queued.iter().map(|s| s.to_string()).collect();
    app.set_queued(&queued);
    app
}

fn row_of(rows: &[String], needle: &str) -> usize {
    rows.iter()
        .position(|r| r.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} missing: {rows:#?}"))
}

#[test]
fn each_queued_message_is_listed_above_the_composer_on_one_line() {
    let app = busy(&["fix the flaky test", "then update the changelog"]);
    let rows = screen_rows(&app, 80, 24);
    let header = row_of(&rows, "2 queued");
    let composer = row_of(&rows, "╭");
    assert!(rows[header + 1].contains("fix the flaky test"), "{rows:#?}");
    assert!(
        rows[header + 2].contains("then update the changelog"),
        "{rows:#?}"
    );
    assert!(header + 2 < composer, "above the composer: {rows:#?}");
}

#[test]
fn a_long_or_multi_line_message_is_cut_to_one_row() {
    let long = format!("{} tail", "word ".repeat(40));
    let app = busy(&[&long, "line one\nline two"]);
    for width in [40u16, 60, 100] {
        let rows = screen_rows(&app, width, 24);
        let header = row_of(&rows, "2 queued");
        let first = rows[header + 1].trim_end();
        assert!(first.ends_with('…'), "{width}: {first}");
        assert!(first.chars().count() <= width as usize, "{width}: {first}");
        assert!(
            rows[header + 2].contains("line one line two"),
            "newlines fold to spaces: {:?}",
            rows[header + 2]
        );
    }
}

#[test]
fn the_header_names_the_keys_that_work_right_now() {
    let mut app = busy(&["next"]);
    let rows = screen_rows(&app, 80, 24);
    let header = &rows[row_of(&rows, "1 queued")];
    assert!(header.contains("↑ edit"), "{header}");
    assert!(header.contains("Esc stop and send now"), "{header}");

    app.input = "half a thought".into();
    let rows = screen_rows(&app, 80, 24);
    let header = &rows[row_of(&rows, "1 queued")];
    assert!(
        !header.contains("↑ edit"),
        "Up edits history while the composer holds text: {header}"
    );

    app.input.clear();
    app.open_permission("shell", "Shell");
    let rows = screen_rows(&app, 80, 24);
    let header = &rows[row_of(&rows, "1 queued")];
    assert!(
        !header.contains("↑ edit") && !header.contains("Esc"),
        "a decision block owns the keyboard: {header}"
    );
}

#[test]
fn the_header_gives_up_words_before_it_clips_one() {
    let app = busy(&["next"]);
    for width in 40..=100u16 {
        let rows = screen_rows(&app, width, 24);
        let header = rows[row_of(&rows, "1 queued")].trim_end().to_string();
        assert!(
            header.chars().count() <= width as usize,
            "{width}: {header}"
        );
        for word in header.split_whitespace() {
            assert!(
                ["1", "queued", "·", "↑", "edit", "Esc", "stop", "and", "send", "now"]
                    .contains(&word),
                "{width}: {word:?} is clipped in {header:?}"
            );
        }
    }
}

#[test]
fn only_the_newest_rows_are_listed_and_the_header_counts_them_all() {
    let app = busy(&["one", "two", "three", "four", "five"]);
    let rows = screen_rows(&app, 80, 24);
    row_of(&rows, "5 queued");
    let text = rows.join("\n");
    for newest in ["three", "four", "five"] {
        assert!(text.contains(newest), "{newest}: {text}");
    }
    assert!(!text.contains("❯ one") && !text.contains("❯ two"), "{text}");
}

#[test]
fn the_band_is_gone_once_the_queue_is_empty() {
    let mut app = busy(&["next"]);
    assert!(screen(&app).contains("1 queued"));
    app.set_queued(&[]);
    let text = screen(&app);
    assert!(!text.contains("queued"), "no stale row, no chip: {text}");
}

#[test]
fn it_fits_the_inline_live_region_without_pushing_the_composer_out() {
    let mut app = busy(&["next", "after that"]);
    app.fullscreen = false;
    let rows = screen_rows(&app, 80, LIVE_H);
    let header = row_of(&rows, "2 queued");
    let composer = row_of(&rows, "╭");
    assert!(header < composer, "{rows:#?}");
    assert!(
        rows.last().is_some_and(|r| !r.trim().is_empty()),
        "the status line is still the last row: {rows:#?}"
    );
}

#[test]
fn a_queued_prompt_comes_back_to_the_composer_with_its_images() {
    let image = forge_types::ImageAttachment {
        media_type: "image/png".into(),
        data_base64: "AAAA".into(),
    };
    let mut app = busy(&[]);
    app.stash_queued_images("what is this", vec![image.clone()]);
    app.set_queued(&["what is this".to_string()]);
    app.restore_queued_to_input("what is this");
    app.set_queued(&[]);
    assert!(app.input.starts_with("what is this"), "{:?}", app.input);
    let (text, images) = app.resolve_paste_blocks(app.input.clone());
    assert_eq!(text.trim(), "what is this");
    assert_eq!(images, vec![image], "the image rides with the draft again");
}
