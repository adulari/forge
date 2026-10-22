//! Switching the pinned model from chat (`/model <id>`, the model and rung pickers). Every switch
//! goes through `Session::request_model_switch`, so `PreModelSwitch` hooks can keep the current
//! model and `PostModelSwitch` hooks see the change (docs/features/hooks.md).
//!
//! Split out of `dispatch.rs` and `pickers.rs`, which sit at their CI size ratchets.

use super::*;

/// Pin `to` (`None` returns to mesh routing) as the user asked from `source`. `false` when a
/// `PreModelSwitch` hook kept the current model (the session shows why).
async fn switch(session: &mut Session, to: Option<String>, source: &str) -> bool {
    session.request_model_switch(to, source).await.is_ok()
}

/// `/model <provider::model>`: pin it immediately, no picker.
pub(super) async fn pin_from_command(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    model_id: &str,
) {
    let model_id = forge_provider::normalize_model_id(model_id).into_owned();
    if let Err(e) = super::session::ensure_pin_providers_known(&model_id) {
        app.note(&format!("⚠ {e}"));
        return;
    }
    let rung = {
        let mut s = session.lock().await;
        if !switch(&mut s, Some(model_id.clone()), "command").await {
            return;
        }
        // The rung the pin will actually run at. Computed here so the statusline is
        // correct the INSTANT the pin lands: previously `pin_model` changed session
        // state and emitted nothing, so the status row kept naming the previously
        // routed model until the next turn produced a Routing event.
        let ceiling = s.pinned_effort();
        s.catalog().and_then(|catalog| {
            forge_mesh::rung_cost::best_value_rung(catalog, &model_id, ceiling, false)
        })
    };
    app.apply(forge_tui::PresenterEvent::Routing {
        tier: "pinned".to_string(),
        model: model_id.clone(),
        rationale: "pinned by /model".to_string(),
        effort: rung,
    });
    app.note(&format!("⊕ model pinned: {model_id} (clear with /model)"));
    let unknown = super::dispatch_checks::model_not_in_catalog(session, &model_id).await;
    if unknown {
        app.note(&format!(
            "⚠ {model_id} is not in the model catalog — the provider may be unknown or have no key; /models lists what is usable"
        ));
    }
}

/// The flat model pin picker: "mesh" clears the pin, any other row pins it.
pub(super) async fn accept_model_pin(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    row: &forge_tui::PickerRow,
) {
    if row.id == "mesh" {
        if switch(&mut *session.lock().await, None, "picker").await {
            app.note("⊕ model pin cleared — mesh auto-routing restored");
        }
        return;
    }
    let model_id = forge_provider::normalize_model_id(&row.id).into_owned();
    // Emit the pick so the status row is right immediately — see the matching comment
    // on the `/model <full-id>` path. Selecting from the picker used to change session state and
    // leave the statusline naming the previous model.
    // Enter pins at the model's best-value rung; `->` opens the rung list for anyone
    // who wants a different trade. Pinning the rung too is the point of the two-step
    // design — a model pin that left effort unset would keep running at whatever the
    // session ceiling happened to be.
    let rung = {
        let mut s = session.lock().await;
        if !switch(&mut s, Some(model_id.clone()), "picker").await {
            return;
        }
        let ceiling = s.pinned_effort();
        let rung = s.catalog().and_then(|catalog| {
            forge_mesh::rung_cost::best_value_rung(catalog, &model_id, ceiling, false)
        });
        if let Some(level) = rung {
            s.set_effort(Some(level));
        }
        rung
    };
    if let Some(level) = rung {
        app.apply(forge_tui::PresenterEvent::Effort(Some(level)));
    }
    app.apply(forge_tui::PresenterEvent::Routing {
        tier: "pinned".to_string(),
        model: model_id.clone(),
        rationale: "pinned by /model".to_string(),
        effort: rung,
    });
    match rung {
        Some(level) => app.note(&format!(
            "⊕ pinned: {model_id} at {} effort (best value) — → for other rungs",
            forge_types::effort::wire_name(level)
        )),
        None => app.note(&format!("⊕ model pinned: {model_id} (clear with /model)")),
    }
}

/// The rung picker: row ids are `provider::model@rung`, so the pair is pinned without carrying
/// state between the two steps.
pub(super) async fn accept_model_rung(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    row: &forge_tui::PickerRow,
) {
    let Some((model_id, rung)) = row.id.rsplit_once('@') else {
        return;
    };
    let level = forge_types::EffortLevel::parse(rung);
    let model_id = model_id.to_string();
    {
        let mut s = session.lock().await;
        if !switch(&mut s, Some(model_id.clone()), "picker").await {
            return;
        }
        if let Some(level) = level {
            s.set_effort(Some(level));
        }
    }
    if let Some(level) = level {
        app.apply(forge_tui::PresenterEvent::Effort(Some(level)));
    }
    app.apply(forge_tui::PresenterEvent::Routing {
        tier: "pinned".to_string(),
        model: model_id.clone(),
        rationale: "pinned by /model".to_string(),
        effort: level,
    });
    app.note(&format!(
        "⊕ pinned: {model_id} at {rung} effort (clear with /model)"
    ));
}
