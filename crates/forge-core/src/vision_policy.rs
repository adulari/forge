//! What happens when a request carries an image and the model it is about to go to cannot read it
//! (`[mesh] vision_fallback`, docs/features/image-delivery.md).
//!
//! "Cannot read" means [`forge_mesh::catalog::vision_capability`] answers `Some(false)` — known
//! text-only from catalog data, a transport with no image channel, or a text-only family. An
//! UNKNOWN model (`None`) still receives the image: refusing it here would turn every custom or
//! freshly released vision model into a text-only one, and the provider's own error is the honest
//! answer for a model nobody has described yet.

use super::*;
use crate::compaction_policy::record_model_failure_in;
use crate::spend_guard::priced;
use forge_config::VisionFallback;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

const DESCRIBE_SYSTEM: &str = "You describe images for a colleague who cannot see them and will \
act on your description alone. Transcribe every piece of visible text, code, error message, \
number and UI label VERBATIM. Then describe layout, structure and anything visual that matters \
(charts, diagrams, colours, highlighted regions). State only what is visible; do not guess.";

#[derive(Default)]
pub(crate) struct VisionState {
    /// `(model, image)` pairs already warned about, so a tool loop warns once, not per step.
    warned: HashSet<(String, u64)>,
    /// Image fingerprint -> description, so each image is described once per session.
    described: HashMap<u64, String>,
}

fn fingerprint(image: &forge_types::ImageAttachment) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    image.media_type.hash(&mut hasher);
    image.data_base64.hash(&mut hasher);
    hasher.finish()
}

fn cannot_read_images(model: &str) -> bool {
    forge_mesh::catalog::vision_capability(model) == Some(false)
}

/// Remove the images from `messages[index]`, leaving a marker so the model knows something was
/// attached that it cannot see.
fn strip_images(messages: &mut [Message], index: usize, note: &str) {
    let message = &mut messages[index];
    message.images.clear();
    if !message.content.is_empty() {
        message.content.push_str("\n\n");
    }
    message.content.push_str(note);
}

impl Session {
    /// Whether the next request will carry an image: one queued for this turn, or one still on the
    /// transcript's latest image turn (the one turn whose images providers keep sending).
    pub(crate) fn transcript_carries_images(&self) -> bool {
        !self.pending_images.is_empty()
            || forge_provider::last_image_turn(&self.transcript).is_some()
    }

    /// Failover under `vision_fallback = "switch"` must not walk an image request onto a model
    /// known to be text-only.
    pub(crate) fn skips_for_vision(&self, model: &str) -> bool {
        self.config.mesh.vision_fallback == VisionFallback::Switch
            && self.transcript_carries_images()
            && cannot_read_images(model)
    }

    /// `vision_fallback = "switch"` at routing time: when the chosen model (pinned or not) cannot
    /// read the image this request carries, re-route without the pin to the best vision-capable
    /// model. Returns the decision to use and whether it left the pin — the caller then skips the
    /// pin clamp for this one turn. A pin is otherwise a contract; this is the user's explicit
    /// configured exception.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn vision_switch(
        &mut self,
        decision: forge_mesh::RoutingDecision,
        prompt: &str,
        budget: BudgetState,
        health: &forge_types::ModelHealth,
        quota: &forge_types::SubscriptionQuota,
        tier: Option<TaskTier>,
        context: &forge_mesh::RoutingContext,
    ) -> (forge_mesh::RoutingDecision, bool) {
        if self.config.mesh.vision_fallback != VisionFallback::Switch
            || !self.transcript_carries_images()
            || !cannot_read_images(&decision.model)
        {
            return (decision, false);
        }
        let routed = self
            .router
            .route_contextual(
                prompt,
                true,
                budget,
                health,
                quota,
                tier,
                self.pinned_effort,
                &self.project,
                context,
            )
            .await;
        // A `--model` session's router is itself pinned, so it answers with the same text-only
        // model: look for a vision-capable one among the configured and discovered models.
        let mut routed = routed;
        if routed.unroutable || !forge_mesh::catalog::supports_vision(&routed.model) {
            let tiers = [TaskTier::Complex, TaskTier::Standard, TaskTier::Trivial];
            let configured = tiers.iter().flat_map(|t| self.config.candidates_for(*t));
            let discovered = self.catalog.iter().flat_map(|c| c.models().iter().cloned());
            let Some(model) = configured.chain(discovered).find(|m| {
                forge_mesh::catalog::supports_vision(m)
                    && !forge_config::is_model_disabled(m, &self.config.mesh.disabled)
                    && forge_config::has_api_key(forge_config::provider_of(m))
                    && forge_mesh::privacy::allows(&self.config, m)
            }) else {
                return (decision, false);
            };
            routed.model = model;
            routed.unroutable = false;
            routed.pinned = false;
            routed.fallbacks.clear();
        }
        self.presenter.emit(PresenterEvent::Warning(format!(
            "{} can't read images; routed this request to {} (mesh.vision_fallback = \"switch\")",
            decision.model, routed.model
        )));
        (routed, true)
    }

    /// Apply `vision_fallback` to the request about to go to `model`. Only the latest image turn is
    /// touched — earlier ones are already text-only on the wire.
    pub(crate) async fn apply_vision_policy(
        &mut self,
        model: &str,
        mut sent: Vec<Message>,
    ) -> Vec<Message> {
        let Some(index) = forge_provider::last_image_turn(&sent) else {
            return sent;
        };
        if !cannot_read_images(model) {
            return sent;
        }
        let images = sent[index].images.clone();
        if self.config.mesh.vision_fallback == VisionFallback::Describe {
            let request = sent[index].content.clone();
            if let Some(text) = self.describe_images(&images, &request).await {
                strip_images(
                    &mut sent,
                    index,
                    &format!(
                        "[{model} cannot read images; this description of the attached image(s) \
                         was generated by a vision model]\n{text}"
                    ),
                );
                return sent;
            }
        }
        let first = images.first().map(fingerprint).unwrap_or_default();
        if self.vision.warned.insert((model.to_string(), first)) {
            let hint = match self.config.mesh.vision_fallback {
                VisionFallback::Warn => " — set mesh.vision_fallback = \"describe\" or \"switch\"",
                VisionFallback::Describe => " — no vision model was available to describe it",
                VisionFallback::Switch => " — no vision-capable model was available to switch to (a --model pin restricts routing to that model)",
            };
            self.presenter.emit(PresenterEvent::Warning(format!(
                "{model} can't read images; sent text only{hint}"
            )));
        }
        strip_images(
            &mut sent,
            index,
            "[image attachment omitted: this model cannot read images]",
        );
        sent
    }

    /// Vision-capable models a side call may use, best-first. A subscription is never picked: the
    /// session is not running on it (it just failed to read the image on the model it IS running
    /// on) and a describe call is optional, so it must not spend a plan the user did not point
    /// this session at.
    fn describe_candidates(&self) -> Vec<String> {
        let health = self.provider_readiness().health;
        let mut seen = HashSet::new();
        let mut pool = self.router.trivial_candidates();
        pool.extend(self.config.candidates_for(TaskTier::Trivial));
        pool.extend(self.config.candidates_for(TaskTier::Standard));
        if let Some(catalog) = &self.catalog {
            pool.extend(catalog.models().iter().cloned());
        }
        pool.into_iter()
            .filter(|m| seen.insert(m.clone()))
            .filter(|m| forge_mesh::catalog::supports_vision(m))
            .filter(|m| !forge_mesh::catalog::is_subscription(m))
            .filter(|m| !forge_config::is_model_disabled(m, &self.config.mesh.disabled))
            .filter(|m| {
                crate::auxiliary_candidates::usable_for_side_call(m, &health, self.catalog.as_ref())
            })
            .filter(|m| forge_mesh::privacy::allows(&self.config, m))
            .filter(|m| forge_config::has_api_key(forge_config::provider_of(m)))
            .collect()
    }

    /// One description per image, cached by content. `None` when any image could not be described
    /// (no candidate, or every candidate failed) so the caller falls back to the warning path
    /// instead of sending half a description.
    async fn describe_images(
        &mut self,
        images: &[forge_types::ImageAttachment],
        request: &str,
    ) -> Option<String> {
        let mut parts = Vec::new();
        for (n, image) in images.iter().enumerate() {
            let key = fingerprint(image);
            let text = match self.vision.described.get(&key) {
                Some(cached) => cached.clone(),
                None => {
                    let text = self.describe_one(image, request).await?;
                    self.vision.described.insert(key, text.clone());
                    text
                }
            };
            parts.push(if images.len() > 1 {
                format!("Image {}:\n{text}", n + 1)
            } else {
                text
            });
        }
        Some(parts.join("\n\n"))
    }

    async fn describe_one(
        &mut self,
        image: &forge_types::ImageAttachment,
        request: &str,
    ) -> Option<String> {
        let snippet: String = request.chars().take(400).collect();
        let ask = if snippet.is_empty() {
            "Describe this image.".to_string()
        } else {
            format!("Describe this image. The user's request that came with it:\n{snippet}")
        };
        let messages = vec![
            Message::system(DESCRIBE_SYSTEM),
            Message::user_with_images(ask, vec![image.clone()]),
        ];
        let cooldown = std::time::Duration::from_secs(self.config.mesh.failover_cooldown_secs);
        for model in self.describe_candidates().into_iter().take(3) {
            let opts = Session::auxiliary_completion_options(&self.id, "vision-describe");
            let mut on_event = |_: StreamEvent| {};
            match self
                .provider
                .complete_with(&model, &messages, &[], &opts, &mut on_event)
                .await
            {
                Ok(reply) if !reply.content.trim().is_empty() => {
                    let _ = self.store.record_side_call_usage_for(
                        &self.id,
                        "vision-describe",
                        Some(&model),
                        &priced(&self.pricing, &model, &reply.usage),
                    );
                    return Some(reply.content.trim().to_string());
                }
                Ok(_) => {}
                Err(error) => record_model_failure_in(&self.store, &model, &error, cooldown),
            }
        }
        None
    }
}
