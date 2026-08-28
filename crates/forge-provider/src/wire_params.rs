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
