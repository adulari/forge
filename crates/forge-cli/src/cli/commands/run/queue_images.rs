//! Queueing a plain prompt that was typed while a turn runs, together with its pasted images.
//!
//! Text-only prompts are offered to the running turn at its next step (steering). A prompt with
//! images cannot be: the running turn already built its user message, and a steered line has no
//! way to carry pixels. So image prompts wait for the turn boundary and run as a normal queued
//! turn, with their images attached when [`forge_tui::App::promote_queued_images`] hands them to
//! the spawn.

use forge_types::ImageAttachment;

pub(crate) fn queue_prompt(
    queue: &mut Vec<String>,
    steer: &forge_core::steer::SteerInbox,
    app: &mut forge_tui::App,
    line: String,
    images: Vec<ImageAttachment>,
) {
    let with_images = !images.is_empty();
    if with_images {
        app.stash_queued_images(&line, images);
    } else {
        steer.push(line.clone());
    }
    queue.push(line);
    app.set_queued(queue);
    if with_images {
        app.note(&format!(
            "⏳ queued with image ({} pending) — runs when this turn finishes",
            queue.len()
        ));
    } else {
        app.note(&format!(
            "⏳ queued ({} pending) — sent at this turn's next step",
            queue.len()
        ));
    }
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
        assert_eq!(steer.drain(), ["plain"]);

        let first = crate::cli::commands::run::dequeue_prompt(&mut queue, &mut app).unwrap();
        assert_eq!(first, "what is this");
        assert_eq!(app.take_ready_images(), vec![img()]);
        let second = crate::cli::commands::run::dequeue_prompt(&mut queue, &mut app).unwrap();
        assert_eq!(second, "plain");
        assert!(app.take_ready_images().is_empty());
    }
}
