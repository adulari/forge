//! Listening TCP sockets owned by a set of processes and their descendants, plus any process
//! working inside a given directory (an agent-started dev server has no terminal parent but does
//! run in the worktree). Shared by the daemon's `GET /api/ports` and the TUI's `/run` overlay.
//!
//! Linux reads `/proc`; other Unixes shell out to `ps` and `lsof`; Windows reports nothing.

use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct ListeningPort {
    pub port: u16,
    /// `loopback` (127.0.0.1 / ::1), `all` (0.0.0.0 / ::) or `other`: who can reach it.
    pub host: &'static str,
    pub pid: u32,
    pub process: String,
    pub command: String,
    /// `action` (started by a project action), `terminal` or `workspace`.
    pub source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

const MAX_COMMAND_CHARS: usize = 200;

/// A root process and, for a project action, the action's id.
pub(crate) type Root = (u32, Option<String>);

/// Whether a process working in `cwd` counts as running in `workspace`. Other sessions' worktrees
/// live under `<repo>/.forge/worktrees`, so they are not this workspace's even though their path
/// starts with it.
pub(crate) fn in_workspace(cwd: &Path, workspace: &Path) -> bool {
    cwd.starts_with(workspace) && !cwd.starts_with(workspace.join(".forge").join("worktrees"))
}

/// A workspace so broad (the home directory or something above it) that matching by cwd would
/// claim most of the user's processes.
fn too_broad(workspace: &Path) -> bool {
    workspace.parent().is_none()
        || std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .is_some_and(|home| home.starts_with(workspace))
}

pub(crate) fn scan(roots: &[Root], workspace: Option<&Path>) -> Vec<ListeningPort> {
    let workspace = workspace.filter(|path| !too_broad(path));
    let mut found = platform::scan(roots, workspace);
    found.retain(|entry| entry.pid != std::process::id());
    found.sort_by_key(|a| (a.port, a.pid));
    found.dedup_by(|a, b| a.port == b.port && a.pid == b.pid);
    found
}

/// Classify an address given as the four bytes of a v4 address, network order, or sixteen of v6.
pub(crate) fn classify_host(bytes: &[u8]) -> &'static str {
    let v4_loopback = bytes.len() == 4 && bytes[0] == 127;
    let v6_loopback = bytes.len() == 16 && bytes[..15].iter().all(|b| *b == 0) && bytes[15] == 1;
    if v4_loopback || v6_loopback {
        "loopback"
    } else if bytes.iter().all(|b| *b == 0) {
        "all"
    } else {
        "other"
    }
}

/// Which roots' trees contain each pid: the nearest root wins so an action's child reports the
/// action, not the shell that happens to be its ancestor too.
fn owners(
    roots: &[Root],
    parent_of: &HashMap<u32, u32>,
) -> HashMap<u32, (&'static str, Option<String>)> {
    let root_ids: HashMap<u32, &Root> = roots.iter().map(|root| (root.0, root)).collect();
    let mut owned = HashMap::new();
    for pid in parent_of.keys().copied().chain(root_ids.keys().copied()) {
        let mut at = pid;
        let mut hops = 0;
        while hops < 64 {
            if let Some(root) = root_ids.get(&at) {
                let source = if root.1.is_some() {
                    "action"
                } else {
                    "terminal"
                };
                owned.insert(pid, (source, root.1.clone()));
                break;
            }
            match parent_of.get(&at) {
                Some(parent) if *parent > 1 => at = *parent,
                _ => break,
            }
            hops += 1;
        }
    }
    owned
}

fn trim_command(command: &str) -> String {
    let flat = command.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(MAX_COMMAND_CHARS).collect()
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;

    fn read_stat(pid: u32) -> Option<(String, u32)> {
        let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let open = text.find('(')?;
        let close = text.rfind(')')?;
        let comm = text[open + 1..close].to_string();
        let ppid = text[close + 1..].split_whitespace().nth(1)?.parse().ok()?;
        Some((comm, ppid))
    }

    fn listening_inodes() -> HashMap<u64, (u16, &'static str)> {
        let mut out = HashMap::new();
        for file in ["/proc/net/tcp", "/proc/net/tcp6"] {
            let Ok(text) = std::fs::read_to_string(file) else {
                continue;
            };
            for line in text.lines().skip(1) {
                let cols: Vec<&str> = line.split_whitespace().collect();
                if cols.len() < 10 || cols[3] != "0A" {
                    continue;
                }
                let Some((addr, port)) = cols[1].split_once(':') else {
                    continue;
                };
                let (Ok(port), Ok(inode)) = (u16::from_str_radix(port, 16), cols[9].parse()) else {
                    continue;
                };
                out.insert(inode, (port, host_of(addr)));
            }
        }
        out
    }

    /// `/proc/net` prints each 32-bit word of the address in host byte order.
    fn host_of(hex: &str) -> &'static str {
        let mut bytes = Vec::new();
        for word in hex.as_bytes().chunks(8) {
            let Ok(text) = std::str::from_utf8(word) else {
                return "other";
            };
            let Ok(value) = u32::from_str_radix(text, 16) else {
                return "other";
            };
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
        classify_host(&bytes)
    }

    pub(super) fn scan(roots: &[Root], workspace: Option<&Path>) -> Vec<ListeningPort> {
        let Ok(dir) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        let mut parent_of = HashMap::new();
        let mut names = HashMap::new();
        for entry in dir.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            if let Some((comm, ppid)) = read_stat(pid) {
                parent_of.insert(pid, ppid);
                names.insert(pid, comm);
            }
        }
        let owned = owners(roots, &parent_of);
        let mut candidates: HashMap<u32, (&'static str, Option<String>)> = owned;
        if let Some(workspace) = workspace {
            for pid in parent_of.keys() {
                if candidates.contains_key(pid) {
                    continue;
                }
                let inside = std::fs::read_link(format!("/proc/{pid}/cwd"))
                    .is_ok_and(|cwd| in_workspace(&cwd, workspace));
                if inside {
                    candidates.insert(*pid, ("workspace", None));
                }
            }
        }
        if candidates.is_empty() {
            return Vec::new();
        }
        let sockets = listening_inodes();
        let mut found = Vec::new();
        let mut seen = HashSet::new();
        for (pid, (source, action)) in candidates {
            let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
                continue;
            };
            for fd in fds.flatten() {
                let Ok(target) = std::fs::read_link(fd.path()) else {
                    continue;
                };
                let target = target.to_string_lossy();
                let Some(inode) = target
                    .strip_prefix("socket:[")
                    .and_then(|rest| rest.strip_suffix(']'))
                    .and_then(|inode| inode.parse::<u64>().ok())
                else {
                    continue;
                };
                let Some((port, host)) = sockets.get(&inode) else {
                    continue;
                };
                if !seen.insert((pid, *port)) {
                    continue;
                }
                let command = std::fs::read(format!("/proc/{pid}/cmdline"))
                    .map(|raw| trim_command(&String::from_utf8_lossy(&raw).replace('\0', " ")))
                    .unwrap_or_default();
                found.push(ListeningPort {
                    port: *port,
                    host,
                    pid,
                    process: names.get(&pid).cloned().unwrap_or_default(),
                    command,
                    source,
                    action: action.clone(),
                });
            }
        }
        found
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
mod platform {
    use super::*;
    use std::process::Command;

    pub(super) fn scan(roots: &[Root], _workspace: Option<&Path>) -> Vec<ListeningPort> {
        let Ok(ps) = Command::new("ps")
            .args(["-A", "-o", "pid=,ppid=,comm="])
            .output()
        else {
            return Vec::new();
        };
        let mut parent_of = HashMap::new();
        let mut names = HashMap::new();
        for line in String::from_utf8_lossy(&ps.stdout).lines() {
            let mut parts = line.split_whitespace();
            let (Some(pid), Some(ppid)) = (
                parts.next().and_then(|p| p.parse::<u32>().ok()),
                parts.next().and_then(|p| p.parse::<u32>().ok()),
            ) else {
                continue;
            };
            parent_of.insert(pid, ppid);
            names.insert(pid, parts.collect::<Vec<_>>().join(" "));
        }
        let owned = owners(roots, &parent_of);
        let Ok(lsof) = Command::new("lsof")
            .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-Fpn"])
            .output()
        else {
            return Vec::new();
        };
        let mut found = Vec::new();
        let mut pid = 0u32;
        for line in String::from_utf8_lossy(&lsof.stdout).lines() {
            if let Some(rest) = line.strip_prefix('p') {
                pid = rest.parse().unwrap_or(0);
            } else if let Some(name) = line.strip_prefix('n') {
                let Some((source, action)) = owned.get(&pid) else {
                    continue;
                };
                let Some((addr, port)) = name.rsplit_once(':') else {
                    continue;
                };
                let Ok(port) = port.parse() else { continue };
                let host = match addr {
                    "*" => "all",
                    "127.0.0.1" | "[::1]" => "loopback",
                    _ => "other",
                };
                found.push(ListeningPort {
                    port,
                    host,
                    pid,
                    process: names.get(&pid).cloned().unwrap_or_default(),
                    command: String::new(),
                    source,
                    action: action.clone(),
                });
            }
        }
        found
    }
}

#[cfg(not(unix))]
mod platform {
    use super::*;

    pub(super) fn scan(_roots: &[Root], _workspace: Option<&Path>) -> Vec<ListeningPort> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_classified() {
        assert_eq!(classify_host(&[127, 0, 0, 1]), "loopback");
        assert_eq!(classify_host(&[0, 0, 0, 0]), "all");
        assert_eq!(classify_host(&[192, 168, 1, 4]), "other");
        let mut v6 = [0u8; 16];
        assert_eq!(classify_host(&v6), "all");
        v6[15] = 1;
        assert_eq!(classify_host(&v6), "loopback");
    }

    #[test]
    fn other_sessions_worktrees_are_not_this_workspace() {
        let repo = Path::new("/work/app");
        assert!(in_workspace(Path::new("/work/app/web"), repo));
        assert!(!in_workspace(
            Path::new("/work/app/.forge/worktrees/abc/web"),
            repo
        ));
        let worktree = Path::new("/work/app/.forge/worktrees/abc");
        assert!(in_workspace(
            Path::new("/work/app/.forge/worktrees/abc/web"),
            worktree
        ));
        assert!(!in_workspace(Path::new("/work/other"), repo));
    }

    #[test]
    fn a_home_or_root_workspace_is_too_broad() {
        assert!(too_broad(Path::new("/")));
        if let Some(home) = std::env::var_os("HOME") {
            assert!(too_broad(Path::new(&home)));
            assert!(!too_broad(&Path::new(&home).join("project")));
        }
    }

    #[test]
    fn the_nearest_root_owns_a_process() {
        let roots = vec![(10, None), (20, Some("dev".to_string()))];
        let parents = HashMap::from([(20, 10), (21, 20), (30, 10), (40, 1)]);
        let owned = owners(&roots, &parents);
        assert_eq!(owned[&21], ("action", Some("dev".to_string())));
        assert_eq!(owned[&30], ("terminal", None));
        assert!(!owned.contains_key(&40));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn finds_a_listener_in_this_process_tree() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        // The listener belongs to this test process, so make it the root.
        let found = platform::scan(&[(std::process::id(), None)], None);
        assert!(
            found.iter().any(|p| p.port == port && p.host == "loopback"),
            "{found:?}"
        );
        drop(listener);
    }
}
