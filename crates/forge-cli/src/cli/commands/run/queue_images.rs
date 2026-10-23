//! Queueing a plain prompt that was typed while a turn runs, together with its pasted images.
//!
//! Text-only prompts are offered to the running turn at its next step (steering). A prompt with
//! images cannot be: the running turn already built its user message, and a steered line has no
//! way to carry pixels. So image prompts wait for the turn boundary and run as a normal queued
//! turn, with their images attached when [`forge_tui::App::promote_queued_images`] hands them to
//! the spawn.
//!
//! Prompts keep their arrival order: a prompt is only offered to the running turn when every
//! prompt ahead of it is offered too, so it never runs before an image prompt or a fleet
//! follow-up that was queued first. Queued commands are outside this ordering by design (they run
//! at the turn boundary, never through the model).

use forge_types::ImageAttachment;

use super::busy_commands::is_command_line;

/// How many entries of `queue` are prompts (not queued commands): what the steer inbox must hold
/// for a new prompt to be offered to the running turn without overtaking one.
pub(crate) fn prompts_ahead(queue: &[String]) -> usize {
    queue.iter().filter(|q| !is_command_line(q)).count()
}

pub(crate) fn queue_prompt(
    queue: &mut Vec<String>,
    steer: &forge_core::steer::SteerInbox,
    app: &mut forge_tui::App,
    line: String,
    images: Vec<ImageAttachment>,
) {
    let with_images = !images.is_empty();
    let steered = if with_images {
        app.stash_queued_images(&line, images);
        false
    } else {
        steer.push_behind(line.clone(), prompts_ahead(queue))
    };
    queue.push(line);
    app.set_queued(queue);
    // The terminal draws the queue itself, above the composer; a transcript row would only be a
    // second copy that outlives the message.
    if app.queue_visible {
        return;
    }
    if steered {
        app.note(&format!(
            "⏳ queued ({} pending) — sent at this turn's next step",
            queue.len()
        ));
    } else if with_images {
        app.note(&format!(
            "⏳ queued with image ({} pending) — runs when this turn finishes",
            queue.len()
        ));
    } else {
        app.note(&format!(
            "⏳ queued ({} pending) — runs when this turn finishes, after the prompts ahead of it",
            queue.len()
        ));
    }
}

/// Take the newest queued message back into the composer to edit it (the Up key while a turn
/// runs and the composer is empty). The steer inbox forgets it too, so the running turn cannot
/// still pick it up; its images come back as attachments. Returns whether anything was queued.
pub(crate) fn pull_back_queued(
    queue: &mut Vec<String>,
    steer: &forge_core::steer::SteerInbox,
    app: &mut forge_tui::App,
) -> bool {
    let Some(text) = queue.pop() else {
        return false;
    };
    steer.remove_pending(&text);
    // Before `set_queued`, which drops the stashed images of every prompt that left the queue.
    app.restore_queued_to_input(&text);
    app.set_queued(queue);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img() -> ImageAttachment {
        ImageAttachment {
            media_type: "image/png".into(),
            data_base64: "AAAA".into(),
        }
    }

    #[test]
    fn image_prompt_is_queued_with_its_images_and_not_steered() {
        let steer = forge_core::steer::SteerInbox::default();
        let mut app = forge_tui::App::default();
        let mut queue = Vec::new();
        queue_prompt(
            &mut queue,
            &steer,
            &mut app,
            "what is this".into(),
            vec![img()],
        );
        queue_prompt(&mut queue, &steer, &mut app, "plain".into(), vec![]);
        assert_eq!(queue, ["what is this", "plain"]);
        assert!(
            steer.drain().is_empty(),
            "a plain prompt must not overtake the image prompt queued ahead of it"
        );

        let first = crate::cli::commands::run::dequeue_prompt(&mut queue, &mut app).unwrap();
        assert_eq!(first, "what is this");
        assert_eq!(app.take_ready_images(), vec![img()]);
        let second = crate::cli::commands::run::dequeue_prompt(&mut queue, &mut app).unwrap();
        assert_eq!(second, "plain");
        assert!(app.take_ready_images().is_empty());
    }

    fn flushed_text(app: &mut forge_tui::App) -> String {
        app.drain_flush()
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_surface_that_draws_the_queue_prints_no_row_for_it() {
        let steer = forge_core::steer::SteerInbox::default();
        let mut queue = Vec::new();

        let mut drawn = forge_tui::App::default();
        drawn.queue_visible = true;
        queue_prompt(&mut queue, &steer, &mut drawn, "one".into(), vec![]);
        crate::cli::commands::run::queue_command(&mut queue, &mut drawn, "/compact");
        assert_eq!(drawn.queued, ["one", "/compact"]);
        assert!(
            flushed_text(&mut drawn).trim().is_empty(),
            "the band is the only trace, so nothing is left behind once the message is delivered"
        );

        let mut mirrored = forge_tui::App::default();
        let mut queue = Vec::new();
        queue_prompt(&mut queue, &steer, &mut mirrored, "one".into(), vec![]);
        assert!(
            flushed_text(&mut mirrored).contains("queued (1 pending)"),
            "a headless mirror has no band, so it keeps announcing the queue"
        );
    }

    #[test]
    fn pulling_back_takes_the_newest_prompt_out_of_the_queue_and_the_turn() {
        let steer = forge_core::steer::SteerInbox::default();
        let mut app = forge_tui::App::default();
        let mut queue = Vec::new();
        queue_prompt(&mut queue, &steer, &mut app, "first".into(), vec![]);
        queue_prompt(&mut queue, &steer, &mut app, "second".into(), vec![]);

        assert!(pull_back_queued(&mut queue, &steer, &mut app));
        assert_eq!(queue, ["first"]);
        assert_eq!(app.queued, ["first"], "the band follows the queue");
        assert_eq!(app.input, "second", "back in the composer, to edit");
        assert_eq!(
            steer.drain(),
            ["first"],
            "the running turn must not still pick up what was pulled back"
        );

        app.input.clear();
        assert!(pull_back_queued(&mut queue, &steer, &mut app));
        assert_eq!(app.input, "first");
        assert!(queue.is_empty() && app.queued.is_empty());
        assert!(
            !pull_back_queued(&mut queue, &steer, &mut app),
            "nothing queued: Up falls through to history"
        );
    }

    #[test]
    fn pulling_back_an_image_prompt_returns_its_images() {
        let steer = forge_core::steer::SteerInbox::default();
        let mut app = forge_tui::App::default();
        let mut queue = Vec::new();
        queue_prompt(
            &mut queue,
            &steer,
            &mut app,
            "what is this".into(),
            vec![img()],
        );
        assert!(pull_back_queued(&mut queue, &steer, &mut app));
        let (text, images) = app.resolve_paste_blocks(app.input.clone());
        assert_eq!(text.trim(), "what is this");
        assert_eq!(images, vec![img()]);
    }

    #[test]
    fn a_queued_command_comes_back_as_text_too() {
        let steer = forge_core::steer::SteerInbox::default();
        let mut app = forge_tui::App::default();
        let mut queue = Vec::new();
        crate::cli::commands::run::queue_command(&mut queue, &mut app, "/compact");
        assert!(pull_back_queued(&mut queue, &steer, &mut app));
        assert_eq!(app.input, "/compact");
        assert!(queue.is_empty());
    }
}
