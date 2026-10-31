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
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(hosts)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub(crate) fn list() -> Result<Vec<HostRecord>> {
    read_at(&registry_path()?)
}

/// Insert or replace by name, keeping the original `added_ms` when the machine is re-added.
pub(crate) fn upsert(mut record: HostRecord) -> Result<()> {
    let path = registry_path()?;
    let mut hosts = read_at(&path)?;
    if let Some(existing) = hosts.iter().position(|h| h.name == record.name) {
        record.added_ms = hosts[existing].added_ms;
        hosts[existing] = record;
    } else {
        hosts.push(record);
    }
    write_at(&path, &hosts)
}

pub(crate) fn remove(name: &str) -> Result<bool> {
    let path = registry_path()?;
    let mut hosts = read_at(&path)?;
    let before = hosts.len();
    hosts.retain(|h| h.name != name);
    if hosts.len() == before {
        return Ok(false);
    }
    write_at(&path, &hosts)?;
    Ok(true)
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
