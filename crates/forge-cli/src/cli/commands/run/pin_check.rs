//! Whether a pinned model id deserves the "unknown model" warning.
//!
//! The cached catalog is only as complete as the process that wrote it. A headless daemon whose
//! `PATH` lacked `~/.local/bin` cataloged no `claude-cli::` models, and every later login-shell
//! run then flagged a perfectly valid `claude-cli::claude-opus-5-5` pin against that stale file.
//! A CLI bridge that is routable in THIS process is therefore judged by the bridge's own alias
//! table, not by catalog membership.

use forge_provider::CliKind;

/// What to do about one pinned id given the cached catalog.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum PinVerdict {
    Ok,
    /// Warn: not in the catalog and not explainable by a routable bridge.
    Unknown,
}

/// Aliases `claude --model` accepts beyond the documented tiers in [`CliKind::default_models`]:
/// `default` is what claude's own model list advertises first, `best` and `opusplan` are its
/// selector aliases.
const CLAUDE_SELECTOR_ALIASES: &[&str] = &["default", "best", "opusplan"];

/// True when `model` (the part after `provider::`) looks like a full model name for `kind`'s
/// vendor, which the CLI accepts even though the alias table predates it.
fn plausible_full_id(kind: CliKind, model: &str) -> bool {
    match kind {
        CliKind::ClaudeCode => {
            // `sonnet[1m]` / `claude-sonnet-…[1m]` select the long-context variant.
            let base = model.strip_suffix("[1m]").unwrap_or(model);
            base.starts_with("claude-")
                || CLAUDE_SELECTOR_ALIASES.contains(&base)
                || kind.default_models().contains(&base)
        }
        CliKind::Codex => model.starts_with("gpt-"),
        CliKind::Antigravity => model.starts_with("gemini-"),
    }
}

/// A pinned `claude-cli::<model>` that claude cannot run. In the streaming mode the bridge drives,
/// claude does not reject an unknown `--model`: the turn runs on the account's default model and
/// exits 0, so a typo silently spent the subscription on the wrong model. Refused before anything
/// is spawned instead. Codex and agy keep the soft warning.
pub(super) fn unknown_claude_bridge_model(id: &str, claude_routable: bool) -> bool {
    id.starts_with("claude-cli::")
        && claude_routable
        && judge_pin(id, CliKind::ClaudeCode.prefix(), false, |k| {
            k == CliKind::ClaudeCode
        }) == PinVerdict::Unknown
}

pub(super) fn judge_pin(
    id: &str,
    prefix: &str,
    in_catalog: bool,
    bridge_routable: impl Fn(CliKind) -> bool,
) -> PinVerdict {
    if in_catalog {
        return PinVerdict::Ok;
    }
    let Some(kind) = CliKind::all().into_iter().find(|k| k.prefix() == prefix) else {
        return PinVerdict::Unknown;
    };
    if !bridge_routable(kind) {
        return PinVerdict::Unknown;
    }
    let model = id.split_once("::").map_or("", |(_, m)| m);
    if model.is_empty() || kind.default_models().contains(&model) || plausible_full_id(kind, model)
    {
        PinVerdict::Ok
    } else {
        PinVerdict::Unknown
    }
}

/// True when the cached catalog has no entry for a bridge that is routable here — the cache was
/// written by a process that could not see that CLI, so it should be refreshed.
pub(super) fn catalog_missing_routable_bridge(
    catalog_models: &[String],
    bridge_routable: impl Fn(CliKind) -> bool,
) -> bool {
    CliKind::all().into_iter().any(|k| {
        bridge_routable(k)
            && !catalog_models
                .iter()
                .any(|m| m.starts_with(&format!("{}::", k.prefix())))
    })
}

/// Catalog models worth offering for a pinned id the catalog does not list.
///
/// Same provider as the id when it names one; for a bare id only names that are genuinely close,
/// whichever provider lists them. Never "the first few models of the catalog": a suggestion from an
/// unrelated provider reads as advice and is wrong far more often than not.
pub(super) fn close_matches(id: &str, catalog: &[String]) -> Vec<String> {
    let (prefix, name) = id.split_once("::").unwrap_or(("", id));
    let name = name.to_lowercase();
    let reach = (name.chars().count() / 4).clamp(1, 3);
    let mut scored: Vec<(usize, &String)> = catalog
        .iter()
        .filter_map(|candidate| {
            let (candidate_prefix, candidate_name) =
                candidate.split_once("::").unwrap_or(("", candidate));
            if !prefix.is_empty() && candidate_prefix != prefix {
                return None;
            }
            let candidate_name = candidate_name.to_lowercase();
            // A name typed up to some point (`llama-3.3-70b`) is closer than any edit distance says.
            let extends = name.len() >= 4 && candidate_name.starts_with(&name);
            let distance = super::dispatch_checks::edit_distance(&name, &candidate_name);
            if extends {
                Some((0, candidate))
            } else if distance <= reach {
                Some((distance, candidate))
            } else {
                None
            }
        })
        .collect();
    scored.sort_by_key(|(distance, candidate)| (*distance, candidate.len()));
    scored
        .into_iter()
        .map(|(_, candidate)| candidate.clone())
        .take(3)
        .collect()
}

/// Whether the catalog lists a pin that names no provider. A bare id is dispatched to the adapter
/// genai infers from its name (`gpt-*` to OpenAI, `claude-*` to Anthropic, …), and to the local
/// Ollama server when the name matches nothing, so it is listed when some model carries that name
/// and, for the Ollama fallback, only when it is an `ollama::` one.
pub(super) fn bare_pin_listed(id: &str, catalog: &[String]) -> bool {
    if forge_provider::falls_back_to_local_ollama(id) {
        return catalog
            .iter()
            .any(|m| m.strip_prefix("ollama::") == Some(id));
    }
    catalog
        .iter()
        .any(|m| m.split_once("::").is_some_and(|(_, name)| name == id))
}

/// The one line shown at launch for a pin nothing lists: what is wrong, the nearest real models
/// when there are any, and the way out. A script has no `/model`, so it is pointed at
/// `forge models`.
pub(super) fn unknown_pin_warning(
    id: &str,
    why: &str,
    suggestions: &[String],
    attended: bool,
) -> String {
    let way_out = if attended {
        "Run /model to pick another."
    } else {
        "`forge models` lists the usable ids."
    };
    if suggestions.is_empty() {
        format!("unknown model '{id}' — {why}. {way_out}")
    } else {
        format!(
            "unknown model '{id}' — did you mean {}? {way_out}",
            suggestions.join(" or ")
        )
    }
}

/// What the launch check found: ids no catalog lists, and the subset known to be uncallable.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct PinCheck {
    /// Ids nothing lists; one fast refusal of one ends a turn at once.
    pub(super) unverified: Vec<String>,
    /// Ids no provider can serve right now (a bare id only the absent local Ollama would take).
    /// The composer refuses to send while the whole pin is in this list.
    pub(super) unavailable: Vec<String>,
}

/// Launch-time check of every pinned id against what is known about the machine. Emits one plain
/// warning per id nothing lists and returns those ids, so a refusal of one ends the turn at once.
/// Soft by design: a brand-new model may simply not be discovered yet.
///
/// With a catalog, an id is judged against it. Without one (a first launch with nothing cached) a
/// bare id the provider layer cannot place still has a known destination, the local Ollama server,
/// so one that is not running is reported here instead of after the first message. Ollama is only
/// named when the user has set it up (`ollama_configured`); to anyone else it is not a thing.
pub(super) async fn check_pins(
    pin: Option<&str>,
    catalog: Option<&[String]>,
    ollama_configured: bool,
    presenter: &mut dyn forge_tui::Presenter,
) -> PinCheck {
    check_pins_with(
        pin,
        catalog,
        ollama_configured,
        presenter,
        local_ollama_answers,
    )
    .await
}

/// Whether the user has set Ollama up: a configured local model, autostart, or an `OLLAMA_HOST`.
pub(super) fn ollama_configured(config: &forge_config::Config) -> bool {
    config.local.model.is_some()
        || config.local.autostart
        || std::env::var("OLLAMA_HOST").is_ok_and(|host| !host.trim().is_empty())
}

/// [`check_pins`] with the local-Ollama probe supplied, so the no-catalog path is testable.
async fn check_pins_with<Probe, Answers>(
    pin: Option<&str>,
    catalog: Option<&[String]>,
    ollama_configured: bool,
    presenter: &mut dyn forge_tui::Presenter,
    ollama_answers: Probe,
) -> PinCheck
where
    Probe: Fn() -> Answers,
    Answers: std::future::Future<Output = bool>,
{
    let attended = presenter.is_attended();
    let mut found = PinCheck::default();
    for id in pin.map(super::session::pin_members).unwrap_or_default() {
        let prefix = forge_config::provider_of(&id);
        let only_ollama_would_take_it =
            prefix.is_empty() && forge_provider::falls_back_to_local_ollama(&id);
        let why_unplaceable = if ollama_configured {
            "no provider knows it, and Ollama isn't running"
        } else {
            "no provider knows it"
        };
        let mut unavailable = false;
        let warning = match catalog {
            Some(models) => {
                let listed = if prefix.is_empty() {
                    bare_pin_listed(&id, models)
                } else {
                    models.contains(&id)
                };
                let unknown = !listed
                    && (prefix.is_empty()
                        || judge_pin(&id, prefix, false, |k| k.routable()) == PinVerdict::Unknown);
                if unknown && only_ollama_would_take_it {
                    unavailable = !ollama_answers().await;
                }
                unknown.then(|| {
                    unknown_pin_warning(
                        &id,
                        "no model like it in the catalog",
                        &close_matches(&id, models),
                        attended,
                    )
                })
            }
            None if only_ollama_would_take_it && !ollama_answers().await => {
                unavailable = true;
                Some(unknown_pin_warning(&id, why_unplaceable, &[], attended))
            }
            None => None,
        };
        if let Some(warning) = warning {
            presenter.emit(forge_tui::PresenterEvent::Warning(warning));
            found.unverified.push(id.clone());
            if unavailable {
                found.unavailable.push(id);
            }
        }
    }
    found
}

/// A prompt typed while the session's whole pin is known to be uncallable is not sent: the turn
/// would only fail, and the text is worth keeping. It goes back into the composer, with one line
/// that says what to do. `true` when the prompt was held back.
pub(super) async fn hold_prompt_for_unavailable_model(
    session: &std::sync::Arc<tokio::sync::Mutex<forge_core::Session>>,
    app: &mut forge_tui::App,
    line: &str,
) -> bool {
    let Some(model) = session.lock().await.unavailable_pin().map(str::to_string) else {
        return false;
    };
    app.replace_input(line.to_string());
    app.apply(forge_tui::PresenterEvent::Error(format!(
        "not sent: model '{model}' isn't available. Run /model to pick another."
    )));
    true
}

/// `host:port` of the local Ollama server a bare, unrecognised model id is sent to, from an
/// `OLLAMA_HOST` value (empty when unset).
fn ollama_address_from(host: &str) -> String {
    let host = host.trim();
    let host = host
        .strip_prefix("http://")
        .or_else(|| host.strip_prefix("https://"))
        .unwrap_or(host);
    let host = host.split('/').next().unwrap_or("");
    match host {
        "" => "127.0.0.1:11434".to_string(),
        h if h.contains(':') => h.to_string(),
        h => format!("{h}:11434"),
    }
}

/// Whether anything accepts connections where a bare, unrecognised id would be sent. A refused
/// connection answers instantly; the timeout only bounds an unreachable host.
pub(super) async fn local_ollama_answers() -> bool {
    let connect = tokio::net::TcpStream::connect(ollama_address_from(
        &std::env::var("OLLAMA_HOST").unwrap_or_default(),
    ));
    matches!(
        tokio::time::timeout(std::time::Duration::from_millis(400), connect).await,
        Ok(Ok(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const YES: fn(CliKind) -> bool = |_| true;
    const NO: fn(CliKind) -> bool = |_| false;

    #[test]
    fn catalog_member_is_always_ok() {
        assert_eq!(
            judge_pin("claude-cli::x", "claude-cli", true, NO),
            PinVerdict::Ok
        );
    }

    #[test]
    fn routable_bridge_accepts_full_ids_and_aliases_missing_from_a_stale_catalog() {
        assert_eq!(
            judge_pin("claude-cli::claude-opus-5-5", "claude-cli", false, YES),
            PinVerdict::Ok
        );
        assert_eq!(
            judge_pin("claude-cli::opus", "claude-cli", false, YES),
            PinVerdict::Ok
        );
        assert_eq!(
            judge_pin("claude-cli::", "claude-cli", false, YES),
            PinVerdict::Ok
        );
    }

    #[test]
    fn routable_bridge_still_flags_a_typo() {
        assert_eq!(
            judge_pin("claude-cli::opuss", "claude-cli", false, YES),
            PinVerdict::Unknown
        );
    }

    #[test]
    fn unroutable_bridge_and_other_providers_keep_the_warning() {
        assert_eq!(
            judge_pin("claude-cli::claude-opus-5-5", "claude-cli", false, NO),
            PinVerdict::Unknown
        );
        assert_eq!(
            judge_pin("groq::nope", "groq", false, YES),
            PinVerdict::Unknown
        );
    }

    #[test]
    fn claude_selector_aliases_and_long_context_variants_are_known() {
        for id in [
            "claude-cli::default",
            "claude-cli::opusplan",
            "claude-cli::sonnet[1m]",
            "claude-cli::claude-opus-5-5[1m]",
        ] {
            assert_eq!(
                judge_pin(id, "claude-cli", false, YES),
                PinVerdict::Ok,
                "{id}"
            );
        }
    }

    #[test]
    fn only_an_unexplainable_claude_pin_is_refused() {
        assert!(unknown_claude_bridge_model("claude-cli::opuss", true));
        assert!(unknown_claude_bridge_model(
            "claude-cli::not-a-real-model-xyz",
            true
        ));
        assert!(!unknown_claude_bridge_model("claude-cli::haiku", true));
        assert!(!unknown_claude_bridge_model("claude-cli::", true));
        assert!(!unknown_claude_bridge_model(
            "claude-cli::claude-haiku-5-5",
            true
        ));
        // Not installed: the spawn reports that itself.
        assert!(!unknown_claude_bridge_model("claude-cli::opuss", false));
        // Other bridges keep the soft warning.
        assert!(!unknown_claude_bridge_model("codex-cli::nope", true));
    }

    #[test]
    fn stale_catalog_detection() {
        let only_groq = vec!["groq::a".to_string()];
        assert!(catalog_missing_routable_bridge(&only_groq, |k| k == CliKind::ClaudeCode));
        assert!(!catalog_missing_routable_bridge(&only_groq, NO));
        let with_claude = vec!["claude-cli::opus".to_string()];
        assert!(!catalog_missing_routable_bridge(&with_claude, |k| k == CliKind::ClaudeCode));
    }

    fn catalog() -> Vec<String> {
        [
            "groq::llama-3.3-70b-versatile",
            "groq::llama-3.1-8b-instant",
            "openai::gpt-5.6-luna",
            "openai::gpt-5.6",
            "anthropic::claude-sonnet-5",
            "opencode_go::luna",
            "ollama::qwen3:8b",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }

    #[test]
    fn a_typo_is_matched_inside_its_own_provider_only() {
        let near = close_matches("groq::llama-3.3-70b-versatil", &catalog());
        assert_eq!(near, vec!["groq::llama-3.3-70b-versatile"]);
        // Same name under another provider is not offered: the id said groq.
        let near = close_matches("groq::gpt-5.6-luna", &catalog());
        assert!(near.is_empty(), "{near:?}");
    }

    #[test]
    fn a_name_typed_up_to_a_point_extends_to_the_full_id() {
        let near = close_matches("groq::llama-3.3-70b", &catalog());
        assert_eq!(near, vec!["groq::llama-3.3-70b-versatile"]);
    }

    #[test]
    fn nothing_close_means_no_suggestion_not_the_first_models_of_the_catalog() {
        assert!(close_matches("nonexistent-model-xyz", &catalog()).is_empty());
        assert!(close_matches("groq::nonexistent-model-xyz", &catalog()).is_empty());
        assert!(close_matches("zzz", &[]).is_empty());
    }

    #[test]
    fn a_bare_typo_may_match_across_providers_but_only_when_the_name_is_close() {
        let near = close_matches("gpt-5.6-lunaa", &catalog());
        assert_eq!(near, vec!["openai::gpt-5.6-luna"]);
    }

    #[test]
    fn at_most_three_suggestions_closest_first() {
        let many: Vec<String> = [
            "a::model-1",
            "a::model-2",
            "a::model-3",
            "a::model-4",
            "a::model",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let near = close_matches("a::model-", &many);
        assert_eq!(near.len(), 3);
        assert_eq!(near[0], "a::model-1", "shortest extension first: {near:?}");
    }

    #[test]
    fn a_bare_id_is_listed_by_name_and_the_ollama_fallback_only_by_an_ollama_model() {
        let c = catalog();
        assert!(bare_pin_listed("gpt-5.6-luna", &c));
        assert!(!bare_pin_listed("gpt-9", &c));
        assert!(bare_pin_listed("qwen3:8b", &c));
        // `luna` matches nothing genai knows, so it goes to Ollama, which lists no `luna`.
        assert!(!bare_pin_listed("luna", &c));
        assert!(!bare_pin_listed("nonexistent-model-xyz", &c));
    }

    #[test]
    fn the_launch_warning_is_one_line_with_the_way_out() {
        let none = unknown_pin_warning(
            "nonexistent-model-xyz",
            "no model like it in the catalog",
            &[],
            true,
        );
        assert_eq!(
            none,
            "unknown model 'nonexistent-model-xyz' — no model like it in the catalog. Run /model to pick another."
        );
        let some = unknown_pin_warning(
            "groq::llama-3.3-70b-versatil",
            "unused",
            &["groq::llama-3.3-70b-versatile".to_string()],
            true,
        );
        assert_eq!(
            some,
            "unknown model 'groq::llama-3.3-70b-versatil' — did you mean groq::llama-3.3-70b-versatile? Run /model to pick another."
        );
        let script = unknown_pin_warning("x", "why", &[], false);
        assert!(
            script.ends_with("`forge models` lists the usable ids."),
            "{script}"
        );
        assert!(!script.contains("/model"), "{script}");
    }

    #[test]
    fn ollama_host_forms_resolve_to_an_address() {
        for (value, want) in [
            ("", "127.0.0.1:11434"),
            ("http://box:11500/", "box:11500"),
            ("box", "box:11434"),
            ("0.0.0.0:9", "0.0.0.0:9"),
        ] {
            assert_eq!(ollama_address_from(value), want, "{value:?}");
        }
    }

    /// A presenter that only records what was emitted.
    fn presenter() -> (
        forge_tui::ChannelPresenter,
        std::sync::mpsc::Receiver<forge_tui::UiMsg>,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        (forge_tui::ChannelPresenter::new(tx), rx)
    }

    fn warnings(rx: &std::sync::mpsc::Receiver<forge_tui::UiMsg>) -> Vec<String> {
        rx.try_iter()
            .filter_map(|msg| match msg {
                forge_tui::UiMsg::Event(forge_tui::PresenterEvent::Warning(w)) => Some(w),
                _ => None,
            })
            .collect()
    }

    async fn never() -> bool {
        false
    }

    #[tokio::test]
    async fn a_typo_with_a_catalog_warns_once_with_same_provider_suggestions() {
        let (mut p, rx) = presenter();
        let models = catalog();
        let unverified = check_pins_with(
            Some("groq::llama-3.3-70b-versatil"),
            Some(&models),
            false,
            &mut p,
            never,
        )
        .await
        .unverified;
        assert_eq!(unverified, vec!["groq::llama-3.3-70b-versatil"]);
        assert_eq!(
            warnings(&rx),
            vec!["unknown model 'groq::llama-3.3-70b-versatil' — did you mean groq::llama-3.3-70b-versatile? Run /model to pick another."]
        );
    }

    #[tokio::test]
    async fn an_id_nothing_resembles_gets_no_borrowed_suggestions() {
        let (mut p, rx) = presenter();
        let models = catalog();
        let unverified = check_pins_with(
            Some("nonexistent-model-xyz"),
            Some(&models),
            false,
            &mut p,
            never,
        )
        .await
        .unverified;
        assert_eq!(unverified, vec!["nonexistent-model-xyz"]);
        let warnings = warnings(&rx);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].contains("no model like it in the catalog"),
            "{warnings:?}"
        );
        assert!(
            !warnings[0].contains("groq::"),
            "no unrelated provider: {warnings:?}"
        );
        assert!(
            warnings[0].ends_with("Run /model to pick another."),
            "{warnings:?}"
        );
    }

    #[tokio::test]
    async fn a_listed_pin_and_a_set_of_listed_pins_are_quiet() {
        let (mut p, rx) = presenter();
        let models = catalog();
        let unverified = check_pins_with(
            Some("groq::llama-3.1-8b-instant, openai::gpt-5.6"),
            Some(&models),
            false,
            &mut p,
            never,
        )
        .await
        .unverified;
        assert!(unverified.is_empty());
        assert!(warnings(&rx).is_empty());
    }

    /// The S11 launch: a clean machine with no catalog yet, `--model nonexistent-model-xyz`. The id
    /// is dispatched to the local Ollama server, which is not running, so that is knowable now.
    #[tokio::test]
    async fn without_a_catalog_an_unplaceable_bare_id_is_flagged_when_no_ollama_answers() {
        let (mut p, rx) = presenter();
        let found =
            check_pins_with(Some("nonexistent-model-xyz"), None, false, &mut p, never).await;
        assert_eq!(found.unverified, vec!["nonexistent-model-xyz"]);
        assert_eq!(found.unavailable, vec!["nonexistent-model-xyz"]);
        assert_eq!(
            warnings(&rx),
            vec!["unknown model 'nonexistent-model-xyz' — no provider knows it. Run /model to pick another."],
            "Ollama is not named to someone who never set it up"
        );
    }

    #[tokio::test]
    async fn ollama_is_named_only_when_the_user_configured_it() {
        let (mut p, rx) = presenter();
        check_pins_with(Some("nonexistent-model-xyz"), None, true, &mut p, never).await;
        assert_eq!(
            warnings(&rx),
            vec!["unknown model 'nonexistent-model-xyz' — no provider knows it, and Ollama isn't running. Run /model to pick another."]
        );
    }

    #[tokio::test]
    async fn only_an_id_nothing_can_serve_is_unavailable() {
        let (mut p, _rx) = presenter();
        let models = catalog();
        // Unlisted but a provider might still serve it (a new model on a known provider).
        let typo = check_pins_with(
            Some("groq::llama-3.3-70b-versatil"),
            Some(&models),
            false,
            &mut p,
            never,
        )
        .await;
        assert_eq!(typo.unverified.len(), 1);
        assert!(typo.unavailable.is_empty(), "{typo:?}");
        // A bare unknown id with a catalog and no Ollama listening.
        let bare = check_pins_with(
            Some("nonexistent-model-xyz"),
            Some(&models),
            false,
            &mut p,
            never,
        )
        .await;
        assert_eq!(bare.unavailable, vec!["nonexistent-model-xyz"]);
        // The same id when something listens where it would be sent.
        let up = check_pins_with(
            Some("nonexistent-model-xyz"),
            Some(&models),
            false,
            &mut p,
            || async { true },
        )
        .await;
        assert!(up.unavailable.is_empty(), "{up:?}");
    }

    #[tokio::test]
    async fn without_a_catalog_nothing_is_claimed_that_cannot_be_known() {
        let (mut p, rx) = presenter();
        // Ollama is up: the id may be a pulled model.
        let up = check_pins_with(Some("qwen3:8b"), None, false, &mut p, || async { true }).await;
        // A qualified or recognised id needs a catalog to be judged.
        let qualified = check_pins_with(Some("groq::whatever"), None, false, &mut p, never).await;
        let recognised = check_pins_with(Some("gpt-5.6-luna"), None, false, &mut p, never).await;
        assert!(
            up.unverified.is_empty()
                && qualified.unverified.is_empty()
                && recognised.unverified.is_empty()
        );
        assert!(warnings(&rx).is_empty());
    }
}
