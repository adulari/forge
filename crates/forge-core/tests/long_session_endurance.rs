use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use forge_config::{ClassifierKind, Config, OneOrMany, PriceOverride};
use forge_core::Session;
use forge_mesh::HeuristicRouter;
use forge_provider::{EventSink, ModelResponse, Provider, ProviderError, ToolSpec};
use forge_store::Store;
use forge_tui::HeadlessPresenter;
use forge_types::{Message, ReplayItem, Role, Usage};

const MODEL: &str = "ollama::endurance";
// Exceeds the largest real Codex/Claude history observed by
// `scripts/manual-e2e/profile_agent_history.py` (654 user turns).
const TURNS: usize = 768;

#[derive(Default)]
struct EnduranceProvider {
    compactions: AtomicUsize,
    visible_fact_counts: Mutex<Vec<usize>>,
    models: Mutex<BTreeSet<String>>,
    response_delay: Duration,
}

struct InterruptibleProvider {
    slow: AtomicBool,
    entered: tokio::sync::Semaphore,
}

#[async_trait::async_trait]
impl Provider for InterruptibleProvider {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut EventSink<'_>,
    ) -> Result<ModelResponse, ProviderError> {
        self.entered.add_permits(1);
        if self.slow.load(Ordering::Relaxed) {
            std::future::pending::<()>().await;
        }
        Ok(ModelResponse {
            content: "recovered after repeated interruption".to_string(),
            tool_calls: Vec::new(),
            usage: Usage::default(),
            quotas: Vec::new(),
        })
    }
}

impl EnduranceProvider {
    fn facts(messages: &[Message]) -> BTreeSet<String> {
        messages
            .iter()
            .flat_map(|message| {
                message
                    .content
                    .split(|character: char| {
                        !(character.is_ascii_alphanumeric() || character == '_')
                    })
                    .filter(|word| word.starts_with("FACT_"))
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl Provider for EnduranceProvider {
    async fn complete(
        &self,
        model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut EventSink<'_>,
    ) -> Result<ModelResponse, ProviderError> {
        self.models.lock().unwrap().insert(model.to_string());
        if !self.response_delay.is_zero() {
            tokio::time::sleep(self.response_delay).await;
        }
        let facts = Self::facts(messages);
        let compacting = messages.iter().any(|message| {
            message.role == Role::System
                && message
                    .content
                    .starts_with("You are compacting a coding-assistant conversation")
        });
        let content = if compacting {
            self.compactions.fetch_add(1, Ordering::Relaxed);
            format!(
                "Preserved long-session facts: {}",
                facts.into_iter().collect::<Vec<_>>().join(" ")
            )
        } else {
            self.visible_fact_counts.lock().unwrap().push(facts.len());
            "acknowledged".to_string()
        };
        Ok(ModelResponse {
            content,
            tool_calls: Vec::new(),
            usage: Usage::default(),
            quotas: Vec::new(),
        })
    }
}

fn endurance_config() -> Config {
    let mut config = Config::default();
    config.mesh.classifier = ClassifierKind::Heuristic;
    config.mesh.auto_discover = false;
    config.mesh.failover = true;
    let models = (0..8)
        .map(|index| format!("{MODEL}-{index}"))
        .collect::<Vec<_>>();
    for tier in ["trivial", "standard", "complex"] {
        config
            .mesh
            .models
            .insert(tier.to_string(), OneOrMany::Many(models.clone()));
    }
    for model in models {
        config.mesh.pricing.insert(
            model,
            PriceOverride {
                input_per_1k: 0.0,
                output_per_1k: 0.0,
            },
        );
    }
    config
}

fn resume(
    store: Arc<Store>,
    provider: Arc<EnduranceProvider>,
    config: &Config,
    workspace: &std::path::Path,
    session_id: &str,
) -> Session {
    Session::resume(
        store,
        provider,
        Arc::new(HeuristicRouter::new(config.clone())),
        forge_tools::ToolRegistry::with_core_tools_in(workspace),
        Box::new(HeadlessPresenter::new(false)),
        config.clone(),
        session_id,
    )
    .unwrap()
}

#[tokio::test]
async fn hundreds_of_turns_survive_repeated_compaction_and_resume() {
    let workspace = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(EnduranceProvider::default());
    let config = endurance_config();
    let mut session = Session::start(
        Arc::clone(&store),
        provider.clone(),
        Arc::new(HeuristicRouter::new(config.clone())),
        forge_tools::ToolRegistry::with_core_tools_in(workspace.path()),
        Box::new(HeadlessPresenter::new(false)),
        config.clone(),
        workspace.path().to_str().unwrap(),
    )
    .unwrap();
    let session_id = session.id().to_string();
    let started = Instant::now();

    for turn in 0..TURNS {
        let fact = format!("FACT_{turn:03}");
        session
            .run_turn(&format!(
                "Remember {fact} for this ongoing task. Reply acknowledged."
            ))
            .await
            .unwrap();

        if (turn + 1) % 4 == 0 {
            session.compact(true).await.unwrap();
        }
        if (turn + 1) % 16 == 0 && turn + 1 < TURNS {
            drop(session);
            session = resume(
                Arc::clone(&store),
                provider.clone(),
                &config,
                workspace.path(),
                &session_id,
            );
        }
    }

    assert!(
        started.elapsed() < Duration::from_secs(90),
        "the deterministic {TURNS}-turn endurance run became pathologically slow: {:?}",
        started.elapsed()
    );
    assert!(
        provider.compactions.load(Ordering::Relaxed) >= 60,
        "the test did not exercise enough real compaction calls"
    );
    assert!(
        session.was_compacted(),
        "the final model-facing transcript should remain compacted"
    );
    assert!(
        session.history().len() <= 8,
        "repeated compaction must keep live context bounded, got {} visible messages",
        session.history().len()
    );
    assert_eq!(
        session.checkpoints().unwrap().len(),
        TURNS,
        "every turn remains independently rewindable"
    );

    let replay = session.replay_items_full();
    assert_eq!(
        replay
            .iter()
            .filter(|item| matches!(item, ReplayItem::User(_)))
            .count(),
        TURNS,
        "full user-visible history must retain every original prompt"
    );
    for turn in 0..TURNS {
        let fact = format!("FACT_{turn:03}");
        assert!(
            replay.iter().any(|item| match item {
                ReplayItem::User(content) | ReplayItem::Assistant(content) => {
                    content.contains(&fact)
                }
                _ => false,
            }),
            "full replay lost {fact}"
        );
    }

    assert_eq!(
        provider
            .visible_fact_counts
            .lock()
            .unwrap()
            .iter()
            .copied()
            .max(),
        Some(TURNS),
        "the model-facing compacted summary lost an earlier task fact"
    );

    session.uncompact().unwrap();
    assert_eq!(
        session
            .history()
            .iter()
            .filter(|(role, _)| *role == Role::User)
            .count(),
        TURNS,
        "uncompact must restore all original user turns"
    );
}

#[tokio::test]
async fn repeated_interruptions_release_the_session_and_preserve_recovery() {
    const INTERRUPTIONS: usize = 32;

    let workspace = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(InterruptibleProvider {
        slow: AtomicBool::new(true),
        entered: tokio::sync::Semaphore::new(0),
    });
    let config = endurance_config();
    let session = Arc::new(tokio::sync::Mutex::new(
        Session::start(
            Arc::clone(&store),
            provider.clone(),
            Arc::new(HeuristicRouter::new(config.clone())),
            forge_tools::ToolRegistry::with_core_tools_in(workspace.path()),
            Box::new(HeadlessPresenter::new(false)),
            config,
            workspace.path().to_str().unwrap(),
        )
        .unwrap(),
    ));

    for interruption in 0..INTERRUPTIONS {
        let session_for_turn = Arc::clone(&session);
        let handle = tokio::spawn(async move {
            let mut session = session_for_turn.lock().await;
            session
                .run_turn(&format!("interrupted prompt {interruption}"))
                .await
        });
        provider.entered.acquire().await.unwrap().forget();
        handle.abort();
        let _ = handle.await;

        let guard = tokio::time::timeout(Duration::from_millis(250), session.lock())
            .await
            .expect("every interrupted turn must release the session lock");
        drop(guard);
    }

    provider.slow.store(false, Ordering::Relaxed);
    {
        let mut session = session.lock().await;
        session
            .run_turn("finish cleanly after all interruptions")
            .await
            .unwrap();
        let history = session.history();
        assert_eq!(
            history
                .iter()
                .filter(|(role, _)| *role == Role::User)
                .count(),
            INTERRUPTIONS + 1,
            "every interrupted prompt and the recovery prompt remain visible"
        );
        assert!(
            history.iter().any(|(role, content)| {
                *role == Role::Assistant
                    && content.contains("recovered after repeated interruption")
            }),
            "the session must complete a normal turn after repeated cancellation"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_endurance_sessions_keep_history_isolated() {
    const LANES: usize = 8;
    const TURNS_PER_LANE: usize = 32;

    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());
    let config = endurance_config();
    let mut sessions = tokio::task::JoinSet::new();

    for lane in 0..LANES {
        let workspace = root.path().join(format!("lane-{lane}"));
        std::fs::create_dir_all(&workspace).unwrap();
        let store = Arc::clone(&store);
        let config = config.clone();
        sessions.spawn(async move {
            let provider = Arc::new(EnduranceProvider {
                response_delay: Duration::from_millis(20),
                ..EnduranceProvider::default()
            });
            let mut session = Session::start(
                store,
                provider.clone(),
                Arc::new(HeuristicRouter::new(config.clone())),
                forge_tools::ToolRegistry::with_core_tools_in(&workspace),
                Box::new(HeadlessPresenter::new(false)),
                config,
                workspace.to_str().unwrap(),
            )
            .unwrap();
            for turn in 0..TURNS_PER_LANE {
                session
                    .run_turn(&format!(
                        "Remember FACT_{lane}_{turn:03} only in lane {lane}. Reply acknowledged."
                    ))
                    .await
                    .unwrap();
                if (turn + 1) % 8 == 0 {
                    session.compact(true).await.unwrap();
                }
            }
            let user_turns = session
                .replay_items_full()
                .iter()
                .filter(|item| matches!(item, ReplayItem::User(_)))
                .count();
            let models = provider.models.lock().unwrap().clone();
            (lane, user_turns, session.history(), models)
        });
    }

    let mut completed = 0;
    let mut routed_models = BTreeSet::new();
    while let Some(result) = sessions.join_next().await {
        let (lane, user_turns, active_history, models) = result.unwrap();
        routed_models.extend(models);
        assert_eq!(user_turns, TURNS_PER_LANE, "lane {lane} lost a user turn");
        for other_lane in 0..LANES {
            if other_lane == lane {
                continue;
            }
            assert!(
                active_history
                    .iter()
                    .all(|(_, content)| !content.contains(&format!("FACT_{other_lane}_"))),
                "lane {lane} observed lane {other_lane}'s task facts"
            );
        }
        completed += 1;
    }
    assert_eq!(completed, LANES);
    assert!(
        routed_models.len() >= 4,
        "reservation-aware mesh failover should spread concurrent sessions; used {routed_models:?}"
    );
}
