//! Host pressure: the daemon samples the free space of the filesystems it works in and the
//! machine's available memory, and says so before a build dies of it.
//!
//! The decision logic is pure ([`Tracker::evaluate`] takes a [`Reading`]), so thresholds and
//! hysteresis are tested with injected numbers. A level rises as soon as a reading crosses a
//! threshold but only falls once free space is [`RECOVERY_MARGIN`] times past it, so a disk
//! hovering at the line does not flap the push, the header chip or the refusal.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use forge_config::pressure::PressureConfig;
use forge_core::worktree_reclaim::human_bytes;

use crate::serve::DaemonState;

const GB: f64 = 1024.0 * 1024.0 * 1024.0;
const MB: f64 = 1024.0 * 1024.0;
/// A level only clears once the reading is this far past the threshold that raised it.
const RECOVERY_MARGIN: f64 = 1.25;

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Level {
    #[default]
    Ok,
    Warn,
    Floor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiskReading {
    pub label: String,
    pub path: String,
    pub free: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MemoryReading {
    pub available: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Reading {
    pub disks: Vec<DiskReading>,
    pub memory: Option<MemoryReading>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub(crate) struct DiskStatus {
    pub label: String,
    pub path: String,
    pub free_bytes: u64,
    pub total_bytes: u64,
    pub level: Level,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub(crate) struct MemoryStatus {
    pub available_bytes: u64,
    pub total_bytes: u64,
    pub level: Level,
}

/// What the status routes and the fleet surfaces show.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub(crate) struct Pressure {
    pub level: Level,
    /// One sentence for the header chip and the push; `None` while the host is fine.
    pub summary: Option<String>,
    pub disks: Vec<DiskStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryStatus>,
    pub sampled_at: u64,
}

/// An overall level that just got worse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Transition {
    pub from: Level,
    pub to: Level,
}

fn below(value: u64, total: u64, abs: f64, percent: u8, unit: f64, factor: f64) -> bool {
    let by_size = (value as f64) < abs * unit * factor;
    let by_share =
        total > 0 && ((value as f64) / (total as f64) * 100.0) < f64::from(percent) * factor;
    by_size || by_share
}

fn disk_level(cfg: &PressureConfig, free: u64, total: u64, factor: f64) -> Level {
    if total == 0 {
        return Level::Ok;
    }
    if below(
        free,
        total,
        cfg.disk_floor_free_gb as f64,
        cfg.disk_floor_free_percent,
        GB,
        factor,
    ) {
        Level::Floor
    } else if below(
        free,
        total,
        cfg.disk_warn_free_gb as f64,
        cfg.disk_warn_free_percent,
        GB,
        factor,
    ) {
        Level::Warn
    } else {
        Level::Ok
    }
}

fn memory_level(cfg: &PressureConfig, available: u64, total: u64, factor: f64) -> Level {
    if total > 0
        && below(
            available,
            total,
            cfg.memory_low_mb as f64,
            cfg.memory_low_percent,
            MB,
            factor,
        )
    {
        Level::Warn
    } else {
        Level::Ok
    }
}

/// Rise at once, fall only past the recovery margin.
fn step(prev: Level, enter: Level, hold: Level) -> Level {
    enter.max(prev.min(hold))
}

#[derive(Debug, Default)]
pub(crate) struct Tracker {
    disks: HashMap<String, Level>,
    memory: Level,
    overall: Level,
}

impl Tracker {
    pub(crate) fn evaluate(
        &mut self,
        cfg: &PressureConfig,
        reading: &Reading,
        now: u64,
    ) -> (Pressure, Option<Transition>) {
        let mut disks = Vec::new();
        let mut kept = HashMap::new();
        for disk in &reading.disks {
            let prev = self.disks.get(&disk.path).copied().unwrap_or_default();
            let level = step(
                prev,
                disk_level(cfg, disk.free, disk.total, 1.0),
                disk_level(cfg, disk.free, disk.total, RECOVERY_MARGIN),
            );
            kept.insert(disk.path.clone(), level);
            disks.push(DiskStatus {
                label: disk.label.clone(),
                path: disk.path.clone(),
                free_bytes: disk.free,
                total_bytes: disk.total,
                level,
            });
        }
        self.disks = kept;
        let memory = reading.memory.map(|m| {
            let level = step(
                self.memory,
                memory_level(cfg, m.available, m.total, 1.0),
                memory_level(cfg, m.available, m.total, RECOVERY_MARGIN),
            );
            self.memory = level;
            MemoryStatus {
                available_bytes: m.available,
                total_bytes: m.total,
                level,
            }
        });
        if memory.is_none() {
            self.memory = Level::Ok;
        }
        let level = disks
            .iter()
            .map(|d| d.level)
            .chain(memory.iter().map(|m| m.level))
            .max()
            .unwrap_or_default();
        let transition = (level > self.overall).then_some(Transition {
            from: self.overall,
            to: level,
        });
        self.overall = level;
        let mut pressure = Pressure {
            level,
            summary: None,
            disks,
            memory,
            sampled_at: now,
        };
        pressure.summary = summarize(&pressure);
        (pressure, transition)
    }
}

fn percent(part: u64, total: u64) -> u64 {
    if total == 0 {
        0
    } else {
        (part as f64 / total as f64 * 100.0).round() as u64
    }
}

fn summarize(p: &Pressure) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(worst) = p
        .disks
        .iter()
        .filter(|d| d.level > Level::Ok)
        .max_by_key(|d| (d.level, std::cmp::Reverse(d.free_bytes)))
    {
        parts.push(format!(
            "{} disk: {} free of {} ({}%)",
            worst.label,
            human_bytes(worst.free_bytes),
            human_bytes(worst.total_bytes),
            percent(worst.free_bytes, worst.total_bytes),
        ));
    }
    if let Some(m) = p.memory.as_ref().filter(|m| m.level > Level::Ok) {
        parts.push(format!(
            "memory: {} available of {}",
            human_bytes(m.available_bytes),
            human_bytes(m.total_bytes),
        ));
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

// ---------------------------------------------------------------------------------------------
// Sampling
// ---------------------------------------------------------------------------------------------

/// The nearest ancestor of `path` that exists, so a worktree root not created yet still maps to
/// the filesystem it will land on.
pub(crate) fn nearest_existing(path: &Path) -> Option<PathBuf> {
    path.ancestors().find(|p| p.exists()).map(Path::to_path_buf)
}

#[cfg(unix)]
fn device_of(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| m.dev())
}

#[cfg(not(unix))]
fn device_of(_path: &Path) -> Option<u64> {
    None
}

pub(crate) fn read_disk(label: &str, path: &Path) -> Option<DiskReading> {
    let at = nearest_existing(path)?;
    Some(DiskReading {
        label: label.to_string(),
        path: at.display().to_string(),
        free: fs2::available_space(&at).ok()?,
        total: fs2::total_space(&at).ok()?,
    })
}

/// One reading per distinct filesystem among `places` (first label wins), plus memory.
pub(crate) fn sample(places: &[(String, PathBuf)]) -> Reading {
    let mut seen_devices = Vec::new();
    let mut disks: Vec<DiskReading> = Vec::new();
    for (label, path) in places {
        let Some(reading) = read_disk(label, path) else {
            continue;
        };
        if let Some(dev) = device_of(Path::new(&reading.path)) {
            if seen_devices.contains(&dev) {
                continue;
            }
            seen_devices.push(dev);
        } else if disks.iter().any(|d| d.path == reading.path) {
            continue;
        }
        disks.push(reading);
    }
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    let total = system.total_memory();
    Reading {
        disks,
        memory: (total > 0).then(|| MemoryReading {
            available: system.available_memory(),
            total,
        }),
    }
}

/// The places whose free space matters: the data dir, the default project, every project root and
/// the worktree of every live session.
async fn places(state: &DaemonState) -> Vec<(String, PathBuf)> {
    let mut places = Vec::new();
    places.extend(forge_config::data_dir().map(|dir| ("data".to_string(), dir)));
    places.push(("projects".into(), PathBuf::from(&state.default_cwd)));
    for root in &state.project_roots {
        places.push(("projects".into(), root.clone()));
    }
    for handle in state.registry.all().await {
        if let Some(worktree) = &handle.worktree {
            places.push(("worktrees".into(), PathBuf::from(worktree)));
        }
    }
    places
}

// ---------------------------------------------------------------------------------------------
// The daemon loop
// ---------------------------------------------------------------------------------------------

fn cell() -> &'static Mutex<Option<Pressure>> {
    static CELL: OnceLock<Mutex<Option<Pressure>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

/// The latest sample; `None` before the first one or when `[pressure] enabled = false`.
pub(crate) fn current() -> Option<Pressure> {
    cell().lock().ok().and_then(|p| p.clone())
}

fn publish(pressure: Option<Pressure>) {
    if let Ok(mut slot) = cell().lock() {
        *slot = pressure;
    }
}

fn resample() -> &'static tokio::sync::Notify {
    static NOTIFY: OnceLock<tokio::sync::Notify> = OnceLock::new();
    NOTIFY.get_or_init(tokio::sync::Notify::new)
}

/// Sample again now rather than at the next tick (after a reclaim freed space).
pub(crate) fn request_resample() {
    resample().notify_one();
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

pub(crate) fn push_request(
    summary: &str,
    transition: Transition,
) -> forge_core::push_notify::PushRequest {
    let (title, priority) = if transition.to == Level::Floor {
        (
            "Host nearly out of space",
            forge_core::push_notify::Priority::High,
        )
    } else {
        (
            "Host running low",
            forge_core::push_notify::Priority::Normal,
        )
    };
    forge_core::push_notify::PushRequest {
        message: format!("{summary}. Open Forge to reclaim worktree space."),
        title: Some(title.into()),
        priority,
        session_id: String::new(),
    }
}

pub(crate) fn start(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        let mut tracker = Tracker::default();
        loop {
            let cfg =
                tokio::task::spawn_blocking(|| forge_config::load().unwrap_or_default().pressure)
                    .await
                    .unwrap_or_default();
            if cfg.enabled {
                let places = places(&state).await;
                let reading = tokio::task::spawn_blocking(move || sample(&places))
                    .await
                    .unwrap_or_default();
                let (pressure, transition) = tracker.evaluate(&cfg, &reading, now_unix());
                let changed = current().as_ref().map(|p| p.level) != Some(pressure.level);
                if let (Some(transition), Some(summary)) = (transition, pressure.summary.clone()) {
                    notify(&state, &summary, transition).await;
                }
                publish(Some(pressure));
                if changed {
                    state.registry.notify_fleet();
                }
            } else {
                tracker = Tracker::default();
                publish(None);
            }
            tokio::select! {
                () = tokio::time::sleep(Duration::from_secs(cfg.sample_secs.max(5))) => {}
                () = resample().notified() => {}
            }
        }
    });
}

async fn notify(state: &DaemonState, summary: &str, transition: Transition) {
    let request = push_request(summary, transition);
    let channels = crate::push_tool::channels_for(state, !state.mock);
    let reports = channels.deliver(&request).await;
    let detail: Vec<String> = reports
        .iter()
        .map(|r| format!("{}: {}", r.channel, r.detail))
        .collect();
    tracing::warn!(
        from = ?transition.from,
        to = ?transition.to,
        "host pressure: {summary}; push attempted ({})",
        detail.join(", ")
    );
}

// ---------------------------------------------------------------------------------------------
// Gating new worktree sessions
// ---------------------------------------------------------------------------------------------

/// Free space on the filesystem a new worktree would land on, when it is under a threshold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Gate {
    /// `Warn` creates the session with a warning; `Floor` refuses it.
    pub level: Level,
    pub path: String,
    pub free_bytes: u64,
    pub total_bytes: u64,
    pub summary: String,
}

/// Judge one filesystem for worktree creation. Pure; no hysteresis, because each request is
/// asked afresh and a refusal that clears the moment a build frees a byte helps nobody.
pub(crate) fn gate_for(cfg: &PressureConfig, disk: &DiskReading) -> Option<Gate> {
    if !cfg.enabled {
        return None;
    }
    let level = disk_level(cfg, disk.free, disk.total, 1.0);
    (level > Level::Ok).then(|| Gate {
        level,
        path: disk.path.clone(),
        free_bytes: disk.free,
        total_bytes: disk.total,
        summary: format!(
            "{} has {} free of {} ({}%)",
            disk.path,
            human_bytes(disk.free),
            human_bytes(disk.total),
            percent(disk.free, disk.total),
        ),
    })
}

/// The gate for a worktree about to be created under `repo`, or `None` while there is room.
pub(crate) fn gate_for_new_worktree(repo: &Path) -> Option<Gate> {
    let cfg = forge_config::load().unwrap_or_default().pressure;
    if !cfg.enabled {
        return None;
    }
    let settings = forge_config::load_in(repo)
        .map(|config| config.worktree)
        .unwrap_or_default();
    let target = forge_core::worktree_setup::worktree_path(repo, &settings, "new");
    gate_for(&cfg, &read_disk("worktrees", &target)?)
}

/// What the create-session answer says about a gate, with the reclaimable amount when known.
pub(crate) fn gate_details(gate: &Gate, reclaimable: Option<u64>) -> serde_json::Value {
    serde_json::json!({
        "level": gate.level,
        "path": gate.path,
        "free_bytes": gate.free_bytes,
        "total_bytes": gate.total_bytes,
        "summary": gate.summary,
        "reclaimable_bytes": reclaimable,
    })
}

pub(crate) fn refusal_message(gate: &Gate, reclaimable: Option<u64>) -> String {
    let offer = match reclaimable {
        Some(bytes) if bytes > 0 => {
            format!(
                " Reclaim can free {} from old worktrees.",
                human_bytes(bytes)
            )
        }
        Some(_) => " Nothing is provably safe to reclaim; free space by hand.".to_string(),
        None => " Run `forge worktree reclaim --artifacts` to free space.".to_string(),
    };
    format!(
        "not creating a worktree: {} — below the hard floor.{offer}",
        gate.summary
    )
}

/// The `forge doctor` line: a fresh reading of this machine, judged by the configured thresholds.
pub(crate) fn doctor_check() -> crate::doctor::Check {
    let cfg = forge_config::load().unwrap_or_default().pressure;
    if !cfg.enabled {
        return crate::doctor::check(
            crate::doctor::Status::Info,
            "host pressure",
            "sampling is off ([pressure] enabled = false)",
            None,
        );
    }
    let mut places = Vec::new();
    places.extend(forge_config::data_dir().map(|dir| ("data".to_string(), dir)));
    if let Ok(cwd) = std::env::current_dir() {
        places.push(("projects".into(), cwd));
    }
    let (pressure, _) = Tracker::default().evaluate(&cfg, &sample(&places), now_unix());
    let status = match pressure.level {
        Level::Ok => crate::doctor::Status::Ok,
        Level::Warn => crate::doctor::Status::Warn,
        Level::Floor => crate::doctor::Status::Fail,
    };
    let detail = pressure.summary.clone().unwrap_or_else(|| {
        let free = pressure.disks.iter().map(|d| d.free_bytes).min();
        match free {
            Some(free) => format!(
                "{} free on the tightest filesystem, memory fine",
                human_bytes(free)
            ),
            None => "no filesystem could be read".to_string(),
        }
    });
    let fix = (pressure.level > Level::Ok)
        .then_some("`forge worktree reclaim --artifacts` to preview what can be freed");
    crate::doctor::check(status, "host pressure", detail, fix)
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: u64 = 1024 * 1024 * 1024;

    fn disk(free_gb: f64, total_gb: u64) -> Reading {
        Reading {
            disks: vec![DiskReading {
                label: "projects".into(),
                path: "/work".into(),
                free: (free_gb * G as f64) as u64,
                total: total_gb * G,
            }],
            memory: None,
        }
    }

    fn level(tracker: &mut Tracker, reading: &Reading) -> Level {
        tracker
            .evaluate(&PressureConfig::default(), reading, 0)
            .0
            .level
    }

    #[test]
    fn a_healthy_host_reports_ok_and_no_summary() {
        let mut t = Tracker::default();
        let (p, transition) = t.evaluate(&PressureConfig::default(), &disk(500.0, 1000), 7);
        assert_eq!(p.level, Level::Ok);
        assert_eq!(p.summary, None);
        assert_eq!(transition, None);
        assert_eq!(p.sampled_at, 7);
    }

    #[test]
    fn free_space_under_the_gb_or_percent_threshold_warns() {
        let mut t = Tracker::default();
        assert_eq!(level(&mut t, &disk(15.0, 500)), Level::Warn);
        let mut t = Tracker::default();
        assert_eq!(
            level(&mut t, &disk(80.0, 1000)),
            Level::Warn,
            "8% of 1 TB is under 10%"
        );
    }

    #[test]
    fn the_floor_is_its_own_level_and_names_the_disk() {
        let mut t = Tracker::default();
        let (p, transition) = t.evaluate(&PressureConfig::default(), &disk(2.0, 500), 0);
        assert_eq!(p.level, Level::Floor);
        assert_eq!(
            transition,
            Some(Transition {
                from: Level::Ok,
                to: Level::Floor
            })
        );
        assert!(p.summary.unwrap().starts_with("projects disk: 2.0 GB free"));
    }

    #[test]
    fn a_reading_hovering_at_the_line_does_not_flap() {
        let mut t = Tracker::default();
        assert_eq!(level(&mut t, &disk(19.0, 100)), Level::Warn);
        // Above the 20 GB line but inside the 25 GB recovery margin: still warning.
        assert_eq!(level(&mut t, &disk(21.0, 100)), Level::Warn);
        assert_eq!(level(&mut t, &disk(19.5, 100)), Level::Warn);
        assert_eq!(level(&mut t, &disk(26.0, 100)), Level::Ok);
        // And it needs to cross the line again, not the margin, to warn again.
        assert_eq!(level(&mut t, &disk(22.0, 100)), Level::Ok);
    }

    #[test]
    fn leaving_the_floor_steps_down_through_warn() {
        let mut t = Tracker::default();
        assert_eq!(level(&mut t, &disk(2.0, 100)), Level::Floor);
        assert_eq!(
            level(&mut t, &disk(3.5, 100)),
            Level::Floor,
            "inside the 3 GB * 1.25 margin"
        );
        assert_eq!(level(&mut t, &disk(5.0, 100)), Level::Warn);
        assert_eq!(level(&mut t, &disk(40.0, 100)), Level::Ok);
    }

    #[test]
    fn only_a_worsening_level_is_a_transition() {
        let mut t = Tracker::default();
        let cfg = PressureConfig::default();
        let step_of = |t: &mut Tracker, free| t.evaluate(&cfg, &disk(free, 500), 0).1;
        assert_eq!(
            step_of(&mut t, 15.0),
            Some(Transition {
                from: Level::Ok,
                to: Level::Warn
            })
        );
        assert_eq!(step_of(&mut t, 14.0), None, "still warning");
        assert_eq!(
            step_of(&mut t, 2.0),
            Some(Transition {
                from: Level::Warn,
                to: Level::Floor
            })
        );
        assert_eq!(step_of(&mut t, 100.0), None, "recovery is not a push");
        assert_eq!(
            step_of(&mut t, 15.0),
            Some(Transition {
                from: Level::Ok,
                to: Level::Warn
            })
        );
    }

    #[test]
    fn low_memory_warns_but_never_reaches_the_floor() {
        let mut t = Tracker::default();
        let reading = Reading {
            disks: Vec::new(),
            memory: Some(MemoryReading {
                available: 200 * 1024 * 1024,
                total: 16 * G,
            }),
        };
        let (p, transition) = t.evaluate(&PressureConfig::default(), &reading, 0);
        assert_eq!(p.level, Level::Warn);
        assert_eq!(transition.map(|t| t.to), Some(Level::Warn));
        assert!(p
            .summary
            .unwrap()
            .starts_with("memory: 200.0 MB available of 16.0 GB"));
    }

    #[test]
    fn a_zero_threshold_turns_that_check_off_and_unknown_totals_never_alarm() {
        let off = PressureConfig {
            disk_warn_free_gb: 0,
            disk_warn_free_percent: 0,
            disk_floor_free_gb: 0,
            disk_floor_free_percent: 0,
            ..PressureConfig::default()
        };
        let mut t = Tracker::default();
        assert_eq!(t.evaluate(&off, &disk(0.0, 500), 0).0.level, Level::Ok);
        let mut t = Tracker::default();
        assert_eq!(level(&mut t, &disk(0.0, 0)), Level::Ok);
    }

    #[test]
    fn the_push_request_is_high_priority_only_at_the_floor() {
        let warn = push_request(
            "x",
            Transition {
                from: Level::Ok,
                to: Level::Warn,
            },
        );
        let floor = push_request(
            "x",
            Transition {
                from: Level::Warn,
                to: Level::Floor,
            },
        );
        assert_eq!(warn.priority, forge_core::push_notify::Priority::Normal);
        assert_eq!(floor.priority, forge_core::push_notify::Priority::High);
        assert!(floor.session_id.is_empty());
    }

    #[test]
    fn the_wire_shape_is_lowercase_levels_with_an_optional_memory_block() {
        let (p, _) = Tracker::default().evaluate(&PressureConfig::default(), &disk(2.0, 500), 9);
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["level"], "floor");
        assert_eq!(json["disks"][0]["level"], "floor");
        assert_eq!(json["disks"][0]["label"], "projects");
        assert!(json.get("memory").is_none());
    }

    #[test]
    fn the_gate_warns_below_the_threshold_refuses_below_the_floor_and_can_be_switched_off() {
        let cfg = PressureConfig::default();
        let on = |free_gb: f64| {
            let r = disk(free_gb, 100);
            gate_for(&cfg, &r.disks[0])
        };
        assert_eq!(on(50.0), None);
        assert_eq!(on(15.0).unwrap().level, Level::Warn);
        let floor = on(2.0).unwrap();
        assert_eq!(floor.level, Level::Floor);
        assert!(
            floor.summary.contains("2.0 GB free of 100.0 GB (2%)"),
            "{}",
            floor.summary
        );
        let off = PressureConfig {
            enabled: false,
            ..cfg
        };
        assert_eq!(gate_for(&off, &disk(1.0, 100).disks[0]), None);
    }

    #[test]
    fn the_refusal_names_the_reclaim_offer_only_when_there_is_something_to_reclaim() {
        let gate = gate_for(&PressureConfig::default(), &disk(2.0, 100).disks[0]).unwrap();
        let with = refusal_message(&gate, Some(30 * G));
        assert!(with.contains("Reclaim can free 30.0 GB"), "{with}");
        assert!(refusal_message(&gate, Some(0)).contains("Nothing is provably safe"));
        assert!(refusal_message(&gate, None).contains("forge worktree reclaim"));
        assert_eq!(gate_details(&gate, Some(5))["reclaimable_bytes"], 5);
    }

    #[test]
    fn sampling_this_machine_reads_a_real_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        let reading = sample(&[
            ("a".into(), dir.path().to_path_buf()),
            ("b".into(), dir.path().join("not-created-yet")),
        ]);
        assert_eq!(reading.disks.len(), 1, "one filesystem, reported once");
        assert!(reading.disks[0].total > 0);
    }
}
