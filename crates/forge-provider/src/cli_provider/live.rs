//! Persistent claude transport internals: the live process ([`LiveSession`]), the per-owner slot
//! map, the idle reaper and the bridge turn file. Policy (when to reuse / resume / fall back)
//! lives in `CliProvider::complete_persistent`.

use super::*;

/// A long-lived claude `--input-format stream-json` process driving multiple turns (P1). Holds the
/// child's stdin open between turns; each turn writes one user line and reads stdout until the
/// `result` event, leaving the process alive for the next turn.
pub(super) struct LiveSession {
    pub(super) child: Child,
    pub(super) stdin: tokio::process::ChildStdin,
    pub(super) lines: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    pub(super) stderr_task: Option<tokio::task::JoinHandle<String>>,
    pub(super) pgid: Option<i32>,
    /// Bare model the session was started under; a model change forces a respawn.
    pub(super) model: String,
    /// Transcript messages already consumed by the live process (the delta high-water mark).
    pub(super) sent: usize,
    /// [`CheckpointContext::epoch`] at spawn — see [`ResumeState::epoch`]. A rewind mid-session
    /// forces a respawn so the parked process's context can't outlive the history it came from.
    pub(super) epoch: u64,
    /// Per-turn dynamic context for the served `forge mcp-serve` (`FORGE_BRIDGE_TURN_FILE`): the
    /// parent rewrites it before EVERY turn with the current checkpoint seq and permission mode,
    /// so the one long-lived process follows `/undo` granularity and temper switches without a
    /// respawn. See [`write_turn_file`].
    pub(super) turn_file: Option<std::path::PathBuf>,
    /// claude's own `session_id` (from the stream's `system/init` and `result` lines): what a later
    /// respawn passes to `--resume` to get claude's structured history back.
    pub(super) claude_session: Option<String>,
    /// Set at spawn, cleared once the Forge MCP handshake (harness mode) has completed.
    pub(super) needs_init: bool,
    /// Spawned with `--resume`: a failure before any output falls back to a fresh full-transcript
    /// spawn instead of failing the turn.
    pub(super) resumed: bool,
    /// The reasoning rung baked into the process's argv at spawn (`--effort`). A live process
    /// cannot be re-asked for a different rung, so a parked session may only be reused for a turn
    /// running at the SAME rung. Without this, an
    /// `/effort` change would keep driving turns on a process still running at the rung the
    /// session started in, while every readout reported the new one.
    pub(super) effort: Option<EffortLevel>,
    pub(super) sink_path: Option<std::path::PathBuf>,
    pub(super) sub_rx: tokio::sync::mpsc::UnboundedReceiver<StreamEvent>,
    pub(super) tailer: Option<tokio::task::JoinHandle<()>>,
    /// Set just before writing a turn to stdin, cleared only once `drive_turn` returns `Ok` (the
    /// turn's `result` event was fully read). If the calling future is dropped/cancelled mid-turn
    /// (e.g. an external per-turn timeout or user interrupt), dropping the `self.live` mutex guard
    /// only releases the lock — it does NOT drop this `LiveSession` or its `kill_on_drop` child, so
    /// the process is left parked mid-turn with its abandoned output still arriving. Left `true`
    /// here forces the NEXT call to tear down and respawn instead of reusing a session whose
    /// stdout stream position is ambiguous, which would otherwise let two turns' events interleave.
    pub(super) turn_in_flight: bool,
    /// When the session was last parked idle (spawn or turn completion). The idle reaper compares
    /// against this so a reuse between arm and wake-up cancels the older reaper's claim.
    pub(super) parked_at: std::time::Instant,
}

/// Default for how long a parked live bridge process may sit idle before being reaped. Each parked
/// session is a full `claude` process plus its served `forge mcp-serve` child (~300-500 MB
/// resident); without a reaper an idle daemon session holds that memory for its whole lifetime.
/// 20 minutes: people think between turns, and a reap forces a `--resume` respawn. Override with
/// `FORGE_BRIDGE_IDLE_TTL_SECS`.
pub(super) const LIVE_IDLE_TTL: Duration = Duration::from_secs(20 * 60);

/// Max parked live processes per provider (one per conversation owner). Past it the least recently
/// used idle one is torn down.
pub(super) const LIVE_MAX_SLOTS: usize = 4;

pub(super) fn live_idle_ttl_from_env() -> Duration {
    std::env::var("FORGE_BRIDGE_IDLE_TTL_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map_or(LIVE_IDLE_TTL, Duration::from_secs)
}

/// One conversation owner's persistent state: the live process (if any) and the resume info of its
/// last COMPLETED turn, which outlives the process (reap, eviction, effort change) so the next
/// spawn can `--resume` claude's own history and send only a delta.
#[derive(Default)]
pub(super) struct LiveSlot {
    pub(super) session: Option<LiveSession>,
    pub(super) last: ResumeState,
}

pub(super) type SlotRef = std::sync::Arc<tokio::sync::Mutex<LiveSlot>>;

/// Live slots keyed by conversation owner ([`CheckpointContext::session`]; `None` = a caller with
/// no checkpoint context). One `CliProvider` is shared by the main session AND its subagents (and
/// by every session a daemon hosts), so reuse MUST be keyed by owner — otherwise one conversation's
/// turn would be written into another's claude process.
#[derive(Default)]
pub(super) struct LiveMap {
    pub(super) slots: Vec<(Option<String>, SlotRef, std::time::Instant)>,
}

impl LiveMap {
    pub(super) fn slot_for(&mut self, owner: Option<&str>) -> SlotRef {
        if let Some(entry) = self
            .slots
            .iter_mut()
            .find(|(o, _, _)| o.as_deref() == owner)
        {
            entry.2 = std::time::Instant::now();
            return std::sync::Arc::clone(&entry.1);
        }
        // Make room: drop the least recently used slot that is not mid-turn. A busy slot is never
        // evicted (we cannot lock it); the map may briefly exceed the cap instead.
        while self.slots.len() >= LIVE_MAX_SLOTS {
            let victim = self
                .slots
                .iter()
                .enumerate()
                .filter(|(_, (_, slot, _))| slot.try_lock().is_ok())
                .min_by_key(|(_, (_, _, used))| *used)
                .map(|(i, _)| i);
            let Some(i) = victim else { break };
            let (_, slot, _) = self.slots.remove(i);
            tokio::spawn(async move {
                if let Some(s) = slot.lock().await.session.take() {
                    s.teardown().await;
                }
            });
        }
        let slot: SlotRef = std::sync::Arc::default();
        self.slots.push((
            owner.map(str::to_string),
            std::sync::Arc::clone(&slot),
            std::time::Instant::now(),
        ));
        slot
    }
}

/// Tear down the parked live session once it has sat idle for `ttl`. Armed after every turn that
/// parks a session; `pid` pins the claim so a session reused (and re-parked, re-armed) or
/// respawned in the meantime is left alone — its own newer reaper covers it. The slot's resume
/// info is kept so the next turn can `--resume`.
pub(super) fn arm_live_idle_reaper(slot: SlotRef, pid: Option<u32>, ttl: Duration) {
    tokio::spawn(async move {
        tokio::time::sleep(ttl + ttl.min(Duration::from_secs(1))).await;
        let mut guard = slot.lock().await;
        let expired = matches!(&guard.session, Some(s)
            if s.child.id() == pid && !s.turn_in_flight && s.parked_at.elapsed() >= ttl);
        if expired {
            if let Some(s) = guard.session.take() {
                s.teardown().await;
            }
        }
    });
}

/// Atomically (tmp + rename) publish the per-turn context the served `forge mcp-serve` reads
/// (`forge_core::snapshot::ENV_TURN_FILE`): `{"seq": <i64>, "mode": "<permission key>"}`.
pub(super) fn write_turn_file(
    path: &std::path::Path,
    seq: Option<i64>,
    mode: Option<&str>,
) -> std::io::Result<()> {
    let mut obj = serde_json::Map::new();
    if let Some(seq) = seq {
        obj.insert("seq".into(), seq.into());
    }
    if let Some(mode) = mode {
        obj.insert("mode".into(), mode.into());
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, Value::Object(obj).to_string())?;
    std::fs::rename(&tmp, path)
}

impl LiveSession {
    pub(super) async fn write_control_request(
        &mut self,
        request_id: &str,
        request: Value,
    ) -> std::io::Result<()> {
        use tokio::io::AsyncWriteExt;
        let line = serde_json::json!({
            "type": "control_request",
            "request_id": request_id,
            "request": request,
        })
        .to_string();
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await
    }

    pub(super) async fn read_control_response(
        &mut self,
        request_id: &str,
        deadline: tokio::time::Instant,
    ) -> std::io::Result<Value> {
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Claude control request timed out during Forge MCP startup",
                ));
            }
            let line = tokio::time::timeout(remaining, self.lines.next_line())
                .await
                .map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "Claude control request timed out during Forge MCP startup",
                    )
                })??
                .ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "Claude exited before connecting the Forge MCP server",
                    )
                })?;

            let Ok(event) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if event.get("type").and_then(Value::as_str) != Some("control_response")
                || event
                    .get("response")
                    .and_then(|response| response.get("request_id"))
                    .and_then(Value::as_str)
                    != Some(request_id)
            {
                continue;
            }
            let response = &event["response"];
            if response.get("subtype").and_then(Value::as_str) != Some("success") {
                return Err(std::io::Error::other(format!(
                    "Claude rejected Forge MCP startup control request: {}",
                    response
                )));
            }
            return Ok(response.get("response").cloned().unwrap_or(Value::Null));
        }
    }

    /// Initialize Claude's streaming control protocol and poll its MCP status before sending the
    /// first user turn. Claude Code 2.1.210 may otherwise start Sonnet while Forge is still
    /// `pending`, leaving the model with no tools and turning tool calls into inert prose.
    pub(super) async fn initialize_forge_mcp(&mut self, timeout: Duration) -> std::io::Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        let initialize = claude_initialize_request("forge-init", true);
        self.write_control_request(
            "forge-init",
            initialize
                .get("request")
                .cloned()
                .expect("initialize request always has a request payload"),
        )
        .await?;
        self.read_control_response("forge-init", deadline).await?;

        let mut attempt = 0u32;
        loop {
            let request_id = format!("forge-mcp-status-{attempt}");
            self.write_control_request(&request_id, serde_json::json!({"subtype": "mcp_status"}))
                .await?;
            let response = self.read_control_response(&request_id, deadline).await?;
            let Some(server) = response
                .get("mcpServers")
                .and_then(Value::as_array)
                .and_then(|servers| {
                    servers
                        .iter()
                        .find(|server| server.get("name").and_then(Value::as_str) == Some("forge"))
                })
            else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Claude initialized without the Forge MCP server",
                ));
            };
            match server
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
            {
                "connected" => return Ok(()),
                "pending" => {
                    attempt = attempt.saturating_add(1);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                other => {
                    return Err(std::io::Error::other(format!(
                        "Claude could not connect the Forge MCP server (status: {other})"
                    )));
                }
            }
        }
    }

    /// Write one user turn to the live process's stdin (kept open afterwards).
    pub(super) async fn write_user(&mut self, payload: &str) -> std::io::Result<()> {
        use tokio::io::AsyncWriteExt;
        let line = stream_user_line(payload);
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await
    }

    /// Read the stream until this turn's `result` event, forwarding live events. Reuses the
    /// idle-timeout / subagent-sink select of the one-shot path, but stops at `result` (the process
    /// stays alive) instead of EOF.
    pub(super) async fn drive_turn(
        &mut self,
        idle: Duration,
        kind: CliKind,
        on_event: &mut EventSink<'_>,
    ) -> Result<TurnData, TurnError> {
        let mut data = TurnData::default();
        let mut tool_names: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut active_tools: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut claude_stream = ClaudeStreamState::default();
        // Backstop against a wedged turn that never truly progresses (#714: a claude persistent-
        // bridge tool round-trip that deadlocks — claude goes silent on stdout while out-of-band
        // subagent-sink events keep trickling in, which reset the idle timer forever → the session
        // hangs `busy` indefinitely). The idle window measures TOTAL silence; this one measures
        // silence of CLAUDE ITSELF (its stdout), which a sink-event trickle can't refresh. Generous
        // (idle × 6) so a legitimately long subagent — during which claude is silent but the sink
        // streams real progress — is never truncated; only a genuinely stuck turn trips it.
        let max_claude_silence = idle.saturating_mul(6);
        let mut last_line = std::time::Instant::now();
        enum Ev {
            Line(std::io::Result<Option<String>>),
            Sub(StreamEvent),
        }
        loop {
            if last_line.elapsed() >= max_claude_silence {
                return Err(TurnError::Stall {
                    tool_ran: data.tool_ran,
                });
            }
            // A Forge MCP shell command has its own timeout at roughly the same boundary as this
            // provider-idle watchdog. Give a positively identified in-flight tool one additional
            // idle window so a legitimate long build can return its ToolFinished event instead of
            // racing the stream watchdog. Truly silent model streams retain the original cutoff;
            // the independent `max_claude_silence` backstop still bounds sink-event trickles.
            let quiet_timeout = if active_tools.is_empty() {
                idle
            } else {
                idle.saturating_mul(2)
            };
            let tick = tokio::time::timeout(quiet_timeout, async {
                tokio::select! {
                    biased;
                    line = self.lines.next_line() => Ev::Line(line),
                    Some(ev) = self.sub_rx.recv() => Ev::Sub(ev),
                }
            })
            .await;
            let line = match tick {
                Err(_) => {
                    return Err(TurnError::Stall {
                        tool_ran: data.tool_ran,
                    })
                }
                Ok(Ev::Sub(ev)) => {
                    on_event(ev);
                    continue;
                }
                Ok(Ev::Line(Ok(Some(l)))) => {
                    last_line = std::time::Instant::now();
                    l
                }
                Ok(Ev::Line(Ok(None))) => {
                    return Err(TurnError::Eof {
                        tool_ran: data.tool_ran,
                    })
                }
                Ok(Ev::Line(Err(e))) => return Err(TurnError::Read(e)),
            };
            let mut turn_done = false;
            // Every claude line carries `session_id`; read it off the first one and refresh on
            // `result` rather than parsing every streamed delta twice.
            if kind == CliKind::ClaudeCode
                && (data.session_id.is_none() || line.contains(r#""type":"result""#))
            {
                if let Some(id) = claude_session_id(&line) {
                    data.session_id = Some(id);
                }
            }
            for item in parse_stream_line(kind, &line, &mut claude_stream) {
                match item {
                    Parsed::Activity => on_event(StreamEvent::ProviderActivity),
                    Parsed::Reasoning(t) => on_event(StreamEvent::Reasoning(t)),
                    Parsed::Text(t) => {
                        if !is_cli_auth_instruction(&t) {
                            data.content.push_str(&t);
                            on_event(StreamEvent::Text(t));
                        }
                    }
                    Parsed::ToolStarted { id, name, args } => {
                        data.tool_ran = true;
                        active_tools.insert(id.clone());
                        tool_names.insert(id, name.clone());
                        on_event(StreamEvent::ToolStarted { name, args });
                    }
                    Parsed::ToolFinished { id, ok, summary } => {
                        active_tools.remove(&id);
                        let name = tool_names.get(&id).cloned().unwrap_or_default();
                        on_event(StreamEvent::ToolFinished { name, ok, summary });
                    }
                    Parsed::Usage(u) => data.usage = u,
                    Parsed::Quota {
                        window,
                        status,
                        resets_at,
                        fraction,
                    } => data.quotas.push(forge_types::QuotaHint {
                        provider: kind.prefix().to_string(),
                        window,
                        status,
                        resets_at,
                        fraction_used: fraction,
                    }),
                    Parsed::Thread(_) => {}
                    // `result` ends the turn; the process stays alive for the next one.
                    Parsed::Final(f) => {
                        data.final_text = Some(f);
                        turn_done = true;
                    }
                    Parsed::Error(e) => data.in_band_error = Some(e),
                }
            }
            if turn_done {
                // Drain subagent events that landed just before the result line.
                while let Ok(ev) = self.sub_rx.try_recv() {
                    on_event(ev);
                }
                return Ok(data);
            }
        }
    }

    /// Stop the process and clean up (called on model change, error, or drop-equivalent).
    pub(super) async fn teardown(mut self) {
        // (see `impl Drop for LiveSession` for the drop-without-teardown safety net)
        use tokio::io::AsyncWriteExt;
        if let Some(t) = self.tailer.take() {
            t.abort();
        }
        if let Some(t) = self.stderr_task.take() {
            t.abort();
        }
        if let Some(p) = &self.sink_path {
            let _ = std::fs::remove_file(p);
        }
        if let Some(p) = &self.turn_file {
            let _ = std::fs::remove_file(p);
        }
        // Closing stdin (EOF) lets claude exit its input loop cleanly; then make sure it's gone.
        let _ = self.stdin.shutdown().await;
        terminate(&mut self.child, self.pgid).await;
    }
}

impl Drop for LiveSession {
    /// Belt-and-suspenders: if a `LiveSession` is dropped WITHOUT `teardown` (e.g. the `CliProvider`
    /// itself is dropped while a session is still parked in `self.live`), reap the whole process
    /// GROUP, not just the `kill_on_drop` direct child — otherwise a grandchild (the served
    /// `forge mcp-serve`, or a hung env-build subshell) is orphaned and leaks. No-op on non-Unix.
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pg) = self.pgid {
            // SIGKILL the group (negative pid). ESRCH (already reaped by `teardown`) is harmless.
            unsafe { libc::kill(-pg, libc::SIGKILL) };
        }
    }
}
