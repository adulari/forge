//! The machines added with `forge host add`: a small registry beside the session store.
//!
//! It exists so `forge host list` and the Fleet's add-a-machine screen can say what was set up
//! and when. Reachability and last-seen come from the live fleet, never from here.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct HostRecord {
    pub(crate) name: String,
    pub(crate) target: String,
    pub(crate) added_ms: u64,
    pub(crate) paired: bool,
    #[serde(default)]
    pub(crate) version: String,
}

fn registry_path() -> Result<PathBuf> {
    Ok(forge_config::data_dir()
        .context("no data directory on this platform")?
        .join("hosts.json"))
}

fn read_at(path: &std::path::Path) -> Result<Vec<HostRecord>> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

fn write_at(path: &std::path::Path, hosts: &[HostRecord]) -> Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!(
        "json.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&tmp, serde_json::to_vec_pretty(hosts)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Serialises read-modify-write of the registry within this process (the daemon can finish
/// several add jobs at once).
fn registry_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn list() -> Result<Vec<HostRecord>> {
    read_at(&registry_path()?)
}

/// A machine is identified by its ssh target, not by the name it shows under: two cloud images
/// that both report the hostname `ubuntu` are two machines.
pub(crate) fn upsert(record: HostRecord) -> Result<()> {
    let path = registry_path()?;
    let _guard = registry_lock();
    upsert_at(&path, record)
}

fn upsert_at(path: &std::path::Path, mut record: HostRecord) -> Result<()> {
    let mut hosts = read_at(path)?;
    if hosts
        .iter()
        .any(|h| h.name == record.name && h.target != record.target)
    {
        record.name = format!("{} ({})", record.name, record.target);
    }
    if let Some(existing) = hosts.iter().position(|h| h.target == record.target) {
        record.added_ms = hosts[existing].added_ms;
        hosts[existing] = record;
    } else {
        hosts.push(record);
    }
    write_at(path, &hosts)
}

/// Forget a machine by its ssh target, or by its name when that is unambiguous.
pub(crate) fn remove(key: &str) -> Result<bool> {
    let path = registry_path()?;
    let _guard = registry_lock();
    remove_at(&path, key)
}

fn remove_at(path: &std::path::Path, key: &str) -> Result<bool> {
    let mut hosts = read_at(path)?;
    let by_target = hosts.iter().position(|h| h.target == key);
    let index = match by_target {
        Some(index) => index,
        None => {
            let named: Vec<usize> = hosts
                .iter()
                .enumerate()
                .filter(|(_, h)| h.name == key)
                .map(|(i, _)| i)
                .collect();
            match named.as_slice() {
                [] => return Ok(false),
                [only] => *only,
                _ => bail!("more than one machine is named {key}; use its ssh target"),
            }
        }
    };
    hosts.remove(index);
    write_at(path, &hosts)?;
    Ok(true)
}

/// `/host remove <name>`: everything after the word `remove`, so names may contain spaces.
pub(crate) fn remove_argument(arg: &str) -> Option<&str> {
    let rest = arg.trim().strip_prefix("remove")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(rest.trim()).filter(|name| !name.is_empty())
}

/// `[user@]host` as ssh takes it. Anything ssh could read as an option, or a shell could read as
/// syntax, is refused: the value reaches `ssh` as an argument and is also shown in the UI.
pub(crate) fn validate_target(target: &str) -> Result<&str> {
    let target = target.trim();
    if target.is_empty() || target.len() > 253 {
        bail!("give a host as user@host");
    }
    if target.starts_with('-') {
        bail!("a host cannot start with '-'");
    }
    let ok = target.chars().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@' | ':' | '[' | ']' | '%')
    });
    if !ok {
        bail!("a host may only contain letters, digits and . _ - @ : [ ] %");
    }
    Ok(target)
}

/// The name a machine shows under in the fleet: short, printable, no shell-significant text.
pub(crate) fn validate_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        bail!("a name is 1 to 64 characters");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ' '))
    {
        bail!("a name may only contain letters, digits, spaces and . _ -");
    }
    Ok(name)
}

/// Single-quote a value for a POSIX shell.
pub(crate) fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_that_ssh_or_a_shell_could_misread_are_refused() {
        assert!(validate_target("dev@build-box.example.com").is_ok());
        assert!(validate_target("[::1]").is_ok());
        for bad in ["", "-oProxyCommand=x", "a b", "a;b", "a$(x)", "a'b", "a`b"] {
            assert!(validate_target(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn names_stay_printable() {
        assert_eq!(validate_name(" build box ").unwrap(), "build box");
        assert!(validate_name("a'; rm").is_err());
        assert!(validate_name("").is_err());
    }

    #[test]
    fn quoting_survives_embedded_quotes() {
        assert_eq!(sh_quote("a'b"), "'a'\\''b'");
    }

    fn record(name: &str, target: &str) -> HostRecord {
        HostRecord {
            name: name.into(),
            target: target.into(),
            added_ms: 5,
            paired: true,
            version: String::new(),
        }
    }

    #[test]
    fn two_machines_with_the_same_hostname_stay_two_machines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.json");
        upsert_at(&path, record("ubuntu", "u@10.0.0.5")).unwrap();
        upsert_at(&path, record("ubuntu", "u@10.0.0.6")).unwrap();
        let hosts = read_at(&path).unwrap();
        assert_eq!(hosts.len(), 2);
        assert_ne!(hosts[0].name, hosts[1].name);
        let mut again = record("ubuntu", "u@10.0.0.5");
        again.added_ms = 99;
        upsert_at(&path, again).unwrap();
        let hosts = read_at(&path).unwrap();
        assert_eq!(hosts.len(), 2, "re-adding the same target replaces it");
        assert_eq!(hosts[0].added_ms, 5, "and keeps the first added time");
        assert!(remove_at(&path, "u@10.0.0.6").unwrap());
        assert_eq!(read_at(&path).unwrap().len(), 1);
    }

    #[test]
    fn host_remove_takes_names_with_spaces() {
        assert_eq!(remove_argument("remove build box"), Some("build box"));
        assert_eq!(remove_argument(" remove  x "), Some("x"));
        assert_eq!(remove_argument("remove"), None);
        assert_eq!(remove_argument("removex"), None);
        assert_eq!(remove_argument(""), None);
    }

    #[test]
    fn registry_round_trips_and_keeps_the_first_added_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.json");
        assert!(read_at(&path).unwrap().is_empty());
        let a = HostRecord {
            name: "a".into(),
            target: "u@a".into(),
            added_ms: 5,
            paired: true,
            version: String::new(),
        };
        write_at(&path, std::slice::from_ref(&a)).unwrap();
        assert_eq!(read_at(&path).unwrap(), vec![a]);
    }
}
