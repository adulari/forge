//! Reading the device log.
//!
//! The inspection half of the device tooling, and the reason a crash is diagnosable at all: the
//! screen shows that an app died, logcat shows why. Filtering happens here rather than being left
//! to the caller because an unfiltered buffer is tens of thousands of lines.

use std::time::Duration;

use anyhow::Result;
use serde::Serialize;

use crate::adb::Adb;

/// Android log priorities, ordered so `>=` means "at least this severe".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Level {
    Verbose,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl Level {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().chars().next()? {
            'v' => Some(Self::Verbose),
            'd' => Some(Self::Debug),
            'i' => Some(Self::Info),
            'w' => Some(Self::Warn),
            'e' => Some(Self::Error),
            'f' | 'a' => Some(Self::Fatal),
            _ => None,
        }
    }

    fn letter(&self) -> char {
        match self {
            Self::Verbose => 'V',
            Self::Debug => 'D',
            Self::Info => 'I',
            Self::Warn => 'W',
            Self::Error => 'E',
            Self::Fatal => 'F',
        }
    }
}

/// One parsed log line. Lines that don't parse are kept with `level: None` rather than dropped —
/// native crash dumps and ANR traces are exactly the lines that don't fit the format.
#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    pub timestamp: String,
    pub pid: Option<u32>,
    pub level: Option<Level>,
    pub tag: String,
    pub message: String,
}

impl LogLine {
    pub fn render(&self) -> String {
        match self.level {
            Some(level) => {
                format!(
                    "{} {} {}: {}",
                    self.timestamp,
                    level.letter(),
                    self.tag,
                    self.message
                )
            }
            None => self.message.clone(),
        }
    }
}

/// What to keep from the buffer.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    /// Only lines from this app's process.
    pub package: Option<String>,
    /// Only lines whose tag contains this (case-insensitive).
    pub tag: Option<String>,
    /// Only lines at least this severe.
    pub min_level: Option<Level>,
    /// Only lines whose text contains this (case-insensitive).
    pub contains: Option<String>,
    /// Cap on returned lines, most recent kept.
    pub limit: usize,
}

impl Filter {
    pub fn matches(&self, line: &LogLine, pids: Option<&[u32]>) -> bool {
        if let Some(pids) = pids {
            // An unparsed line has no pid; keep it only when we aren't filtering by process.
            match line.pid {
                Some(pid) if pids.contains(&pid) => {}
                _ => return false,
            }
        }
        if let Some(tag) = &self.tag {
            if !line.tag.to_lowercase().contains(&tag.to_lowercase()) {
                return false;
            }
        }
        if let Some(min) = self.min_level {
            match line.level {
                Some(level) if level >= min => {}
                _ => return false,
            }
        }
        if let Some(text) = &self.contains {
            let needle = text.to_lowercase();
            if !line.message.to_lowercase().contains(&needle)
                && !line.tag.to_lowercase().contains(&needle)
            {
                return false;
            }
        }
        true
    }
}

/// Drain the log buffer and return the lines that survive `filter`.
pub async fn read(adb: &Adb, filter: &Filter) -> Result<Vec<LogLine>> {
    let pids = match &filter.package {
        Some(package) => {
            let found = pids_for(adb, package).await?;
            if found.is_empty() {
                // Not an error: an app that already crashed has no process, and its log lines are
                // exactly what the caller wants. Fall back to matching the package name as text.
                None
            } else {
                Some(found)
            }
        }
        None => None,
    };

    let text = adb
        .shell_timeout("logcat -d -v threadtime", Duration::from_secs(60))
        .await?;
    let mut lines: Vec<LogLine> = text.lines().map(parse_line).collect();

    // The package had no live process — match its name in the text instead of dropping everything.
    let name_fallback = filter.package.as_ref().filter(|_| pids.is_none()).cloned();
    lines.retain(|line| {
        if !filter.matches(line, pids.as_deref()) {
            return false;
        }
        match &name_fallback {
            Some(package) => line.message.contains(package) || line.tag.contains(package),
            None => true,
        }
    });

    let limit = if filter.limit == 0 { 200 } else { filter.limit };
    if lines.len() > limit {
        lines.drain(..lines.len() - limit);
    }
    Ok(lines)
}

/// Empty the log buffer, so a later read shows only what happened next.
pub async fn clear(adb: &Adb) -> Result<()> {
    adb.shell("logcat -c").await.map(|_| ())
}

/// Live pids for a package, including its `:remote`-style child processes.
async fn pids_for(adb: &Adb, package: &str) -> Result<Vec<u32>> {
    let text = adb
        .shell(&format!("pidof {package}"))
        .await
        .unwrap_or_default();
    let mut pids: Vec<u32> = text
        .split_whitespace()
        .filter_map(|p| p.parse().ok())
        .collect();
    if pids.is_empty() {
        // `pidof` matches the exact process name; sub-processes are named `pkg:suffix`.
        let listing = adb.shell("ps -A -o PID,NAME").await.unwrap_or_default();
        pids = listing
            .lines()
            .filter(|line| {
                line.split_whitespace()
                    .nth(1)
                    .is_some_and(|name| name == package || name.starts_with(&format!("{package}:")))
            })
            .filter_map(|line| line.split_whitespace().next()?.parse().ok())
            .collect();
    }
    Ok(pids)
}

/// Parse one `-v threadtime` line: `MM-DD HH:MM:SS.mmm  PID  TID L TAG: message`.
fn parse_line(raw: &str) -> LogLine {
    let unparsed = || LogLine {
        timestamp: String::new(),
        pid: None,
        level: None,
        tag: String::new(),
        message: raw.to_string(),
    };
    let mut fields = raw.split_whitespace();
    let (Some(date), Some(time), Some(pid), Some(_tid), Some(level)) = (
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
    ) else {
        return unparsed();
    };
    let (Ok(pid), Some(level)) = (pid.parse::<u32>(), Level::parse(level)) else {
        return unparsed();
    };
    // The rest is `TAG: message`, and a tag may contain spaces, so split on the first colon.
    let rest = fields.collect::<Vec<_>>().join(" ");
    let (tag, message) = rest.split_once(':').unwrap_or((rest.as_str(), ""));
    LogLine {
        timestamp: format!("{date} {time}"),
        pid: Some(pid),
        level: Some(level),
        tag: tag.trim().to_string(),
        message: message.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
09-06 11:02:03.100  4210  4210 I ActivityManager: Start proc com.app
09-06 11:02:03.200  4210  4233 E AndroidRuntime: FATAL EXCEPTION: main
09-06 11:02:03.300  9999  9999 D OtherApp: unrelated chatter
--------- beginning of crash";

    fn parsed() -> Vec<LogLine> {
        SAMPLE.lines().map(parse_line).collect()
    }

    #[test]
    fn parses_threadtime_lines() {
        let lines = parsed();
        assert_eq!(lines[0].pid, Some(4210));
        assert_eq!(lines[0].level, Some(Level::Info));
        assert_eq!(lines[0].tag, "ActivityManager");
        assert_eq!(lines[0].message, "Start proc com.app");
    }

    #[test]
    fn keeps_lines_that_do_not_fit_the_format() {
        let lines = parsed();
        let separator = lines.last().unwrap();
        assert!(separator.level.is_none());
        assert!(separator.message.contains("beginning of crash"));
    }

    #[test]
    fn min_level_drops_less_severe_lines() {
        let filter = Filter {
            min_level: Some(Level::Error),
            ..Default::default()
        };
        let kept: Vec<_> = parsed()
            .into_iter()
            .filter(|l| filter.matches(l, None))
            .collect();
        assert_eq!(kept.len(), 1);
        assert!(kept[0].message.contains("FATAL EXCEPTION"));
    }

    #[test]
    fn filtering_by_pid_excludes_other_apps_and_unparsed_lines() {
        let filter = Filter::default();
        let kept: Vec<_> = parsed()
            .into_iter()
            .filter(|l| filter.matches(l, Some(&[4210])))
            .collect();
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn levels_order_by_severity() {
        assert!(Level::Error > Level::Warn);
        assert!(Level::Fatal > Level::Error);
        assert_eq!(Level::parse("A"), Some(Level::Fatal));
    }
}
