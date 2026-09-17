//! Wire-level parameter normalisation shared by every OpenAI-shaped provider, plus the account
//! -vs-model distinction the failover policy keys on. Split out of `lib.rs` to keep the crate
//! root under the architecture size guard.

use crate::ProviderError;

/// Widen an `f32` temperature to the `f64` a JSON body carries, WITHOUT dragging the binary
/// representation's noise along.
///
/// `0.1f32 as f64` is `0.10000000149011612` — seventeen decimal places, all of them an artefact of
/// the f32→f64 widening rather than anything the caller asked for. Most providers round it off;
/// b.ai rejects the request outright with `temperature参数非法：限制小数点[2]位` ("temperature is
/// invalid: limited to 2 decimal places"), which killed every turn on that provider. Two decimals
/// is also all a sampling temperature meaningfully carries.
pub fn temperature_for_wire(temperature: f32) -> f64 {
    (f64::from(temperature) * 100.0).round() / 100.0
}

/// Providers whose chat API accepts OpenAI's `frequency_penalty`/`presence_penalty`.
///
/// Sent only while a turn is under repetition pressure, so the blast radius of a wrong entry is
/// one rescued turn rather than every request. Anthropic's Messages API has no such parameter and
/// rejects unknown body fields, so it is deliberately absent; the CLI bridges do not expose
/// sampling at all. Namespaces match `forge_config::provider_of`.
const PENALTY_CAPABLE_PROVIDERS: &[&str] = &[
    "openai",
    "openrouter",
    "kimi",
    "moonshot",
    "deepseek",
    "groq",
    "cerebras",
    "nvidia",
    "mistral",
    "together",
    "xai",
    "opencode",
    "ollama",
    "zen",
];

/// Whether `model`'s provider takes the OpenAI repetition penalties.
pub(crate) fn accepts_repetition_penalties(model: &str) -> bool {
    PENALTY_CAPABLE_PROVIDERS.contains(&forge_config::provider_of(model))
}

/// The `extra_body` fragment carrying whichever penalties the caller set, or `None` when a turn is
/// not repeating (the ordinary case) or the provider cannot take them.
pub(crate) fn repetition_penalty_body(
    model: &str,
    frequency: Option<f32>,
    presence: Option<f32>,
) -> Option<serde_json::Value> {
    if !accepts_repetition_penalties(model) {
        return None;
    }
    let mut body = serde_json::Map::new();
    if let Some(value) = frequency {
        body.insert(
            "frequency_penalty".into(),
            serde_json::json!(temperature_for_wire(value)),
        );
    }
    if let Some(value) = presence {
        body.insert(
            "presence_penalty".into(),
            serde_json::json!(temperature_for_wire(value)),
        );
    }
    (!body.is_empty()).then_some(serde_json::Value::Object(body))
}

/// Attach the penalties to a genai request when the caller reports repetition pressure and the
/// provider takes them. genai has no typed field for either, so they ride in `extra_body`, which
/// OpenAI-compatible endpoints merge into the request body.
pub(crate) fn with_repetition_penalties(
    options: genai::chat::ChatOptions,
    model: &str,
    opts: &crate::CompletionOptions,
) -> genai::chat::ChatOptions {
    match repetition_penalty_body(model, opts.frequency_penalty, opts.presence_penalty) {
        Some(body) => options.with_extra_body(body),
        None => options,
    }
}

/// Whether a stream ended because the provider detected a repetition loop (Kimi's
/// `finish_reason: "repeat"`).
pub(crate) fn cut_for_repetition(reason: Option<&genai::chat::StopReason>) -> bool {
    matches!(reason, Some(genai::chat::StopReason::Other(r)) if r == "repeat")
}

impl ProviderError {
    /// Whether the credential itself is invalid or missing. Unlike a model capability failure,
    /// every alias for this provider will fail until the user re-authenticates.
    pub fn is_auth(&self) -> bool {
        matches!(self, Self::Auth(_))
    }

    /// Whether the ACCOUNT — not the model — is what failed: a bad credential, an exhausted
    /// quota, or a billing wall. Distinct from [`is_auth`](Self::is_auth) because a payment
    /// failure arrives as [`Capability`](Self::Capability) (the `payment required` /
    /// `insufficient_quota` markers in `error_policy`), yet says nothing about what the model can
    /// do — swap the model and the turn runs.
    ///
    /// This is the one failure a strict pin may not hold against. Live failure: a pinned
    /// `claude-fable-5` answered "Payment required to access this resource. Visit your billing
    /// tab." and the whole turn died with `model unsupported`, while the mesh held a dozen
    /// healthy models. A genuine capability mismatch (`no tool support`) still fails the turn
    /// loudly — that one IS about the pinned model, and silently downgrading it would hide the
    /// very thing the pin was expressing.
    pub fn is_credential_failure(&self) -> bool {
        match self {
            Self::Auth(_) => true,
            Self::Capability(msg) => {
                let l = msg.to_ascii_lowercase();
                l.contains("payment required")
                    || l.contains("billing")
                    || l.contains("insufficient_quota")
                    || l.contains("insufficient quota")
                    || l.contains("quota exceeded")
                    || l.contains("credit")
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod temperature_wire_tests {
    use super::temperature_for_wire;

    /// The live failure: `0.1f32 as f64` serialises as `0.10000000149011612`, and b.ai answers
    /// `temperature参数非法：限制小数点[2]位` — every turn on that provider died on it.
    #[test]
    fn widening_noise_never_reaches_the_wire() {
        assert_eq!(
            serde_json::json!(0.1f32 as f64).to_string(),
            "0.10000000149011612",
            "guard: this is the raw widening the fix exists to avoid"
        );
        assert_eq!(
            serde_json::json!(temperature_for_wire(0.1)).to_string(),
            "0.1"
        );
        assert_eq!(
            serde_json::json!(temperature_for_wire(0.2)).to_string(),
            "0.2"
        );
        assert_eq!(
            serde_json::json!(temperature_for_wire(0.7)).to_string(),
            "0.7"
        );
    }

    /// Two decimals is the cap; a third is rounded rather than truncated, and the endpoints of
    /// the usual 0..=2 range survive intact.
    #[test]
    fn two_decimals_is_the_ceiling() {
        assert_eq!(temperature_for_wire(0.125), 0.13);
        assert_eq!(temperature_for_wire(0.0), 0.0);
        assert_eq!(temperature_for_wire(1.0), 1.0);
        assert_eq!(temperature_for_wire(2.0), 2.0);
        for t in [0.0f32, 0.1, 0.25, 0.7, 1.0, 1.33, 2.0] {
            let wire = serde_json::json!(temperature_for_wire(t)).to_string();
            let decimals = wire.split_once('.').map_or(0, |(_, d)| d.len());
            assert!(decimals <= 2, "{t} serialised as {wire}");
        }
    }
}

#[cfg(test)]
mod repetition_penalty_tests {
    use super::{accepts_repetition_penalties, repetition_penalty_body};

    #[test]
    fn only_providers_that_accept_the_penalties_are_sent_them() {
        assert!(accepts_repetition_penalties("kimi::k3-256k"));
        assert!(accepts_repetition_penalties("openrouter::some/model:free"));
        // Anthropic's Messages API has no such parameter and rejects unknown body fields.
        assert!(!accepts_repetition_penalties("anthropic::claude-opus-5"));
        assert!(!accepts_repetition_penalties("claude-cli::fable"));
    }

    #[test]
    fn nothing_is_sent_for_an_ordinary_step() {
        assert!(repetition_penalty_body("kimi::k3-256k", None, None).is_none());
    }

    #[test]
    fn a_repeating_turn_sends_the_penalties_it_asked_for() {
        let body = repetition_penalty_body("kimi::k3-256k", Some(0.8), Some(0.5))
            .expect("kimi takes the penalties");
        assert_eq!(body["frequency_penalty"], serde_json::json!(0.8));
        assert_eq!(body["presence_penalty"], serde_json::json!(0.5));
    }

    #[test]
    fn a_provider_that_cannot_take_them_gets_nothing_even_under_pressure() {
        assert!(
            repetition_penalty_body("anthropic::claude-opus-5", Some(0.8), Some(0.5)).is_none()
        );
    }
}
