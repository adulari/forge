//! Bookkeeping for routing rules that reorder the ranked candidate list, so the inspector can
//! mark rows a rule (not their catalog score) placed.

/// Record that a routing rule, not the catalog score, decided where `model` sits. The last rule
/// to touch a model is the one that decided its final position, so a later rule overwrites.
pub(crate) fn record_reorder(
    reasons: &mut Vec<(String, &'static str)>,
    model: &str,
    reason: &'static str,
) {
    match reasons.iter_mut().find(|(m, _)| m == model) {
        Some(entry) => entry.1 = reason,
        None => reasons.push((model.to_string(), reason)),
    }
}

/// Record every model a sort actually moved.
pub(crate) fn record_moved(
    reasons: &mut Vec<(String, &'static str)>,
    before: &[String],
    after: &[String],
    reason: &'static str,
) {
    for (i, model) in after.iter().enumerate() {
        if before.get(i) != Some(model) {
            record_reorder(reasons, model, reason);
        }
    }
}
