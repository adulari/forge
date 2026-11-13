//! Walking the failover chain: the next usable model, the pacing-held last resort, and the
//! one-per-turn note naming what pacing skipped.

use super::*;

/// Advance `chain` to the next model this turn may dispatch, applying the live subscription
/// pacing verdict: a model in `paced_held` is parked in `deferred_held` instead of dispatched,
/// so failover exhausts every non-held candidate (free or on-pace subscription, in rank order)
/// first. Returns `None` when only held models remain.
pub(super) async fn advance_to_next_usable(
    session: &mut Session,
    chain: &mut std::vec::IntoIter<String>,
    paced_held: &[String],
    skip_provider: Option<&str>,
    freshly_benched: Option<&forge_types::ModelHealth>,
    skip_reserved: bool,
    deferred_held: &mut Vec<String>,
) -> Result<Option<String>, CoreError> {
    for next in chain.by_ref() {
        if forge_config::is_model_disabled(&next, &session.config.mesh.disabled) {
            continue;
        }
        if skip_provider.is_some_and(|p| forge_config::provider_of(&next) == p) {
            continue;
        }
        // A CLI bridge already known to have no credentials — from its own credential file, or
        // because it printed its own sign-in prompt earlier in this process — cannot serve this
        // hop. Skipping it saves a subprocess launch that can only fail the same way.
        //
        // Set only by the CLI's own login machinery ("please sign in", an OAuth prompt), never by
        // this loop's auth classification (`one_auth_error_benches_only_the_model_not_the_whole_provider`).
        if forge_provider::bridge_credentials_known_absent(forge_config::provider_of(&next)) {
            continue;
        }
        if session.skips_for_vision(&next) {
            continue;
        }
        if skip_reserved && session.store.is_model_reserved(&next) {
            continue;
        }
        // The original chain was built before this failure. Re-read health so an auth failure's
        // new provider-wide bench immediately skips its sibling aliases in THIS turn.
        if freshly_benched.is_some_and(|health| health.is_benched(&next)) {
            continue;
        }
        if paced_held.iter().any(|held| held == &next) {
            deferred_held.push(next);
            continue;
        }
        match session.admit_failover_model(&next).await {
            Ok(true) => return Ok(Some(next)),
            Ok(false) => {
                session.presenter.emit(PresenterEvent::Warning(format!(
                    "skipped {next} (declined compaction) — trying the next model"
                )));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// Take the best held model as a genuine LAST RESORT, once nothing non-held is left. Named as
/// such in the routing rationale so the operator can see the pacing hold was overridden rather
/// than ignored.
pub(super) async fn admit_paced_held(
    session: &mut Session,
    deferred_held: &mut Vec<String>,
) -> Result<Option<String>, CoreError> {
    while !deferred_held.is_empty() {
        let next = deferred_held.remove(0);
        match session.admit_failover_model(&next).await {
            Ok(true) => return Ok(Some(next)),
            Ok(false) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// One per-turn line naming what pacing kept the failover chain away from, so a hop that skipped
/// an over-pace subscription is explainable after the fact.
pub(super) fn warn_paced_held_skips(
    session: &mut Session,
    deferred_held: &[String],
    warned: &mut bool,
) {
    if *warned || deferred_held.is_empty() {
        return;
    }
    *warned = true;
    session.presenter.emit(PresenterEvent::Warning(format!(
        "subscription pacing: skipped {} in failover — over pace on its window; \
         trying non-held models first",
        deferred_held.join(", ")
    )));
}
