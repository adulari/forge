//! "This session is working in <repo>. Move it there?": pure detection and state, no I/O of its
//! own beyond the git-root lookup the caller injects.
//!
//! A session started from the home directory (or any non-repo directory) gets none of a project's
//! instructions, Lattice index, change queue, trust or worktree option, yet the agent often spends
//! the whole session on `cd ~/code/repo && cargo …`. [`ProjectOffer`] counts shell calls whose
//! leading `cd` lands in one git repository below home and, once enough of them agree, raises a
//! [`Suggestion`] the shells show as a one-tap card. Moving the session reuses the `/project`
//! switch; this module only decides when to ask.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

/// Shell calls into one repository before the card appears.
pub const THRESHOLD: u32 = 3;

/// The repository the session keeps working in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub path: PathBuf,
    pub name: String,
    pub calls: u32,
}

#[derive(Debug, Clone, Default)]
pub struct ProjectOffer {
    counts: HashMap<PathBuf, u32>,
    dismissed: HashSet<PathBuf>,
    offer: Option<Suggestion>,
}

impl ProjectOffer {
    pub fn suggestion(&self) -> Option<&Suggestion> {
        self.offer.as_ref()
    }

    /// Whether the card owns the Alt+G / Alt+D chords: it is on screen (idle, no turn running).
    /// The chords never type a character, so a draft in the composer does not matter.
    pub fn captures_keys(&self, turn_busy: bool) -> bool {
        self.offer.is_some() && !turn_busy
    }

    /// "Not this repo in this session": it is never offered again, but another repo still can be.
    pub fn dismiss(&mut self) {
        if let Some(offer) = self.offer.take() {
            self.counts.remove(&offer.path);
            self.dismissed.insert(offer.path);
        }
    }

    /// The user accepted: hand back the repository and forget everything counted, because the
    /// session is about to leave the directory these counts were about.
    pub fn take_accepted(&mut self) -> Option<Suggestion> {
        let offer = self.offer.take();
        if offer.is_some() {
            self.counts.clear();
        }
        offer
    }

    /// Forget all counts and the pending card (a different session took over the screen).
    /// Dismissals stay: they belong to the repo, not to the screen.
    pub fn reset(&mut self) {
        self.counts.clear();
        self.offer = None;
    }

    /// Count one `shell` tool call's arguments (the JSON the model sent) made while the session
    /// sat in `cwd`.
    pub fn observe_shell_args(
        &mut self,
        args: &str,
        cwd: &Path,
        home: &Path,
        git_root: &dyn Fn(&Path) -> Option<PathBuf>,
    ) {
        if !session_is_unrooted(cwd, home, git_root) {
            self.reset();
            return;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(args) else {
            return;
        };
        let base = match value.get("cwd").and_then(|v| v.as_str()) {
            Some(dir) => resolve_dir(cwd, dir, home).unwrap_or_else(|| cwd.to_path_buf()),
            None => cwd.to_path_buf(),
        };
        let mut commands: Vec<&str> = Vec::new();
        if let Some(command) = value.get("command").and_then(|v| v.as_str()) {
            commands.push(command);
        }
        if let Some(list) = value.get("commands").and_then(|v| v.as_array()) {
            commands.extend(list.iter().filter_map(|c| c.as_str()));
        }
        for command in commands {
            if let Some(root) = repo_entered_by(command, &base, home, git_root) {
                self.count(root);
            }
        }
    }

    fn count(&mut self, root: PathBuf) {
        if self.dismissed.contains(&root) {
            return;
        }
        let calls = {
            let n = self.counts.entry(root.clone()).or_insert(0);
            *n += 1;
            *n
        };
        match &mut self.offer {
            Some(offer) if offer.path == root => offer.calls = calls,
            Some(_) => {}
            None if calls >= THRESHOLD => {
                let name = root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| root.display().to_string());
                self.offer = Some(Suggestion {
                    path: root,
                    name,
                    calls,
                });
            }
            None => {}
        }
    }
}

/// The directory a repo's own tooling cannot see from: home itself, or anywhere outside a repo.
fn session_is_unrooted(
    cwd: &Path,
    home: &Path,
    git_root: &dyn Fn(&Path) -> Option<PathBuf>,
) -> bool {
    same_dir(cwd, home) || git_root(cwd).is_none()
}

fn same_dir(a: &Path, b: &Path) -> bool {
    canonical(a) == canonical(b)
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// The git repository a command's leading `cd` / `pushd` chain ends in, when it is a repository
/// strictly below `home`.
pub fn repo_entered_by(
    command: &str,
    cwd: &Path,
    home: &Path,
    git_root: &dyn Fn(&Path) -> Option<PathBuf>,
) -> Option<PathBuf> {
    let mut dir = cwd.to_path_buf();
    let mut moved = false;
    for segment in split_chain(command) {
        let Ok(words) = shell_words::split(segment.trim()) else {
            break;
        };
        let mut words = words.into_iter();
        if !matches!(words.next().as_deref(), Some("cd" | "pushd")) {
            break;
        }
        let target = words.find(|w| !w.starts_with('-') || w == "-");
        let Some(target) = target else {
            // A bare `cd` goes home, which is never a project.
            dir = home.to_path_buf();
            moved = true;
            continue;
        };
        if target == "-" {
            return None;
        }
        dir = resolve_dir(&dir, &target, home)?;
        moved = true;
    }
    if !moved {
        return None;
    }
    let root = canonical(&git_root(&dir)?);
    let home = canonical(home);
    (root.starts_with(&home) && root != home).then_some(root)
}

/// Split a command line at `&&`, `||`, `;` and newlines outside quotes.
fn split_chain(command: &str) -> Vec<&str> {
    let bytes = command.as_bytes();
    let mut out = Vec::new();
    let (mut start, mut i) = (0, 0);
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) => {
                if b == b'\\' && q == b'"' {
                    i += 1;
                } else if b == q {
                    quote = None;
                }
            }
            None => match b {
                b'\'' | b'"' => quote = Some(b),
                b'\\' => i += 1,
                b';' | b'\n' => {
                    out.push(&command[start..i]);
                    start = i + 1;
                }
                b'&' | b'|' if bytes.get(i + 1) == Some(&b) => {
                    out.push(&command[start..i]);
                    i += 1;
                    start = i + 1;
                }
                _ => {}
            },
        }
        i += 1;
    }
    out.push(&command[start.min(command.len())..]);
    out
}

/// `target` as an absolute, normalized directory: `~`, `$HOME` and `${HOME}` expand, a relative
/// path resolves against `base`. Any other variable or `~user` makes it unknowable (`None`).
fn resolve_dir(base: &Path, target: &str, home: &Path) -> Option<PathBuf> {
    let expanded: PathBuf = if target == "~" || target == "$HOME" || target == "${HOME}" {
        home.to_path_buf()
    } else if let Some(rest) = target
        .strip_prefix("~/")
        .or_else(|| target.strip_prefix("$HOME/"))
        .or_else(|| target.strip_prefix("${HOME}/"))
    {
        home.join(rest)
    } else if target.starts_with('~') || target.contains('$') || target.contains('`') {
        return None;
    } else {
        base.join(target)
    };
    let mut out = PathBuf::new();
    for component in expanded.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Some(out)
}

/// The nearest ancestor of `dir` (itself included) that holds a `.git` entry.
pub fn find_git_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|p| p.join(".git").exists())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _tmp: tempdir_lite::Dir,
        home: PathBuf,
    }

    mod tempdir_lite {
        use std::path::{Path, PathBuf};
        pub struct Dir(PathBuf);
        impl Dir {
            pub fn new(tag: &str) -> Self {
                let p = std::env::temp_dir().join(format!(
                    "forge-project-offer-{tag}-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_nanos())
                ));
                std::fs::create_dir_all(&p).unwrap();
                Dir(p.canonicalize().unwrap())
            }
            pub fn path(&self) -> &Path {
                &self.0
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    fn fixture(tag: &str) -> Fixture {
        let tmp = tempdir_lite::Dir::new(tag);
        let home = tmp.path().join("home");
        for repo in ["code/forge/crates/core", "code/other", "plain/dir"] {
            std::fs::create_dir_all(home.join(repo)).unwrap();
        }
        std::fs::create_dir_all(home.join("code/forge/.git")).unwrap();
        std::fs::create_dir_all(home.join("code/other/.git")).unwrap();
        std::fs::create_dir_all(tmp.path().join("elsewhere/.git")).unwrap();
        Fixture { _tmp: tmp, home }
    }

    fn entered(fx: &Fixture, command: &str) -> Option<PathBuf> {
        repo_entered_by(command, &fx.home, &fx.home, &find_git_root)
    }

    #[test]
    fn plain_cd_chain_and_semicolon_forms_find_the_repo() {
        let fx = fixture("forms");
        let forge = fx.home.join("code/forge");
        assert_eq!(
            entered(&fx, "cd code/forge && cargo build"),
            Some(forge.clone())
        );
        assert_eq!(entered(&fx, "cd code/forge; ls"), Some(forge.clone()));
        assert_eq!(entered(&fx, "cd ~/code/forge && ls"), Some(forge.clone()));
        assert_eq!(
            entered(&fx, "cd $HOME/code/forge && ls"),
            Some(forge.clone())
        );
        assert_eq!(
            entered(&fx, "cd \"${HOME}/code/forge\" && ls"),
            Some(forge.clone())
        );
        assert_eq!(entered(&fx, "pushd code/forge && ls"), Some(forge.clone()));
        assert_eq!(entered(&fx, "cd -P code/forge && ls"), Some(forge.clone()));
        assert_eq!(entered(&fx, "cd 'code/forge' || true"), Some(forge));
    }

    #[test]
    fn a_subdirectory_resolves_to_the_repo_root() {
        let fx = fixture("subdir");
        assert_eq!(
            entered(&fx, "cd code/forge/crates/core && cargo test"),
            Some(fx.home.join("code/forge"))
        );
    }

    #[test]
    fn chained_cds_follow_each_other_and_dotdot_normalizes() {
        let fx = fixture("chain");
        assert_eq!(
            entered(&fx, "cd code && cd other && ls"),
            Some(fx.home.join("code/other"))
        );
        assert_eq!(
            entered(&fx, "cd code/forge/crates/core/../.. && ls"),
            Some(fx.home.join("code/forge"))
        );
    }

    #[test]
    fn relative_paths_resolve_against_the_given_cwd() {
        let fx = fixture("relative");
        let code = fx.home.join("code");
        assert_eq!(
            repo_entered_by("cd forge && ls", &code, &fx.home, &find_git_root),
            Some(fx.home.join("code/forge"))
        );
    }

    #[test]
    fn only_a_leading_cd_counts() {
        let fx = fixture("leading");
        assert_eq!(entered(&fx, "ls && cd code/forge"), None);
        assert_eq!(entered(&fx, "echo hi"), None);
        assert_eq!(entered(&fx, "echo 'cd code/forge && ls'"), None);
    }

    #[test]
    fn non_repo_unknown_and_unsafe_targets_never_count() {
        let fx = fixture("targets");
        assert_eq!(entered(&fx, "cd plain/dir && ls"), None);
        assert_eq!(entered(&fx, "cd $PROJECT && ls"), None);
        assert_eq!(entered(&fx, "cd ~bob/code && ls"), None);
        assert_eq!(entered(&fx, "cd - && ls"), None);
        assert_eq!(entered(&fx, "cd && ls"), None);
    }

    #[test]
    fn paths_outside_home_and_home_itself_never_count() {
        let fx = fixture("outside");
        let outside = fx.home.parent().unwrap().join("elsewhere");
        assert_eq!(
            entered(&fx, &format!("cd {} && ls", outside.display())),
            None
        );
        fs_git_home(&fx);
        assert_eq!(entered(&fx, "cd ~ && ls"), None);
        assert_eq!(entered(&fx, "cd plain/dir && ls"), None);
    }

    fn fs_git_home(fx: &Fixture) {
        std::fs::create_dir_all(fx.home.join(".git")).unwrap();
    }

    fn args(command: &str) -> String {
        serde_json::json!({ "command": command }).to_string()
    }

    fn observe(offer: &mut ProjectOffer, fx: &Fixture, cwd: &Path, command: &str) {
        offer.observe_shell_args(&args(command), cwd, &fx.home, &find_git_root);
    }

    #[test]
    fn the_third_call_into_one_repo_raises_the_offer() {
        let fx = fixture("threshold");
        let mut offer = ProjectOffer::default();
        observe(&mut offer, &fx, &fx.home, "cd code/forge && ls");
        observe(&mut offer, &fx, &fx.home, "cd ~/code/forge && cargo build");
        assert!(offer.suggestion().is_none());
        observe(
            &mut offer,
            &fx,
            &fx.home,
            "cd code/forge/crates/core && cargo test",
        );
        let s = offer.suggestion().expect("offer after three calls");
        assert_eq!(s.name, "forge");
        assert_eq!(s.path, fx.home.join("code/forge"));
        assert_eq!(s.calls, 3);
        observe(&mut offer, &fx, &fx.home, "cd code/forge && git status");
        assert_eq!(offer.suggestion().unwrap().calls, 4);
    }

    #[test]
    fn calls_into_different_repos_do_not_add_up() {
        let fx = fixture("split");
        let mut offer = ProjectOffer::default();
        for command in [
            "cd code/forge && ls",
            "cd code/other && ls",
            "cd code/forge && ls",
            "cd code/other && ls",
        ] {
            observe(&mut offer, &fx, &fx.home, command);
        }
        assert!(offer.suggestion().is_none());
    }

    #[test]
    fn a_session_already_in_a_repo_is_never_offered_a_move() {
        let fx = fixture("rooted");
        let mut offer = ProjectOffer::default();
        let inside = fx.home.join("code/other");
        for _ in 0..4 {
            observe(&mut offer, &fx, &inside, "cd ../forge && ls");
        }
        assert!(offer.suggestion().is_none());
    }

    #[test]
    fn a_session_in_a_non_repo_directory_is_offered_one() {
        let fx = fixture("nonrepo");
        let mut offer = ProjectOffer::default();
        let plain = fx.home.join("plain/dir");
        for _ in 0..3 {
            observe(&mut offer, &fx, &plain, "cd ~/code/forge && ls");
        }
        assert!(offer.suggestion().is_some());
    }

    #[test]
    fn a_dismissed_repo_is_not_offered_again_but_another_is() {
        let fx = fixture("dismiss");
        let mut offer = ProjectOffer::default();
        for _ in 0..3 {
            observe(&mut offer, &fx, &fx.home, "cd code/forge && ls");
        }
        offer.dismiss();
        assert!(offer.suggestion().is_none());
        for _ in 0..5 {
            observe(&mut offer, &fx, &fx.home, "cd code/forge && ls");
        }
        assert!(offer.suggestion().is_none());
        for _ in 0..3 {
            observe(&mut offer, &fx, &fx.home, "cd code/other && ls");
        }
        assert_eq!(offer.suggestion().unwrap().name, "other");
    }

    #[test]
    fn accepting_hands_back_the_repo_and_clears_the_counts() {
        let fx = fixture("accept");
        let mut offer = ProjectOffer::default();
        for _ in 0..3 {
            observe(&mut offer, &fx, &fx.home, "cd code/forge && ls");
        }
        let accepted = offer.take_accepted().unwrap();
        assert_eq!(accepted.path, fx.home.join("code/forge"));
        assert!(offer.suggestion().is_none());
        observe(&mut offer, &fx, &fx.home, "cd code/forge && ls");
        assert!(offer.suggestion().is_none());
    }

    #[test]
    fn the_commands_list_and_the_cwd_argument_are_read() {
        let fx = fixture("args");
        let mut offer = ProjectOffer::default();
        let batch =
            serde_json::json!({ "commands": ["cd code/forge && ls", "cd code/forge && pwd"] });
        offer.observe_shell_args(&batch.to_string(), &fx.home, &fx.home, &find_git_root);
        let with_cwd = serde_json::json!({
            "command": "cd forge && ls",
            "cwd": fx.home.join("code").display().to_string(),
        });
        offer.observe_shell_args(&with_cwd.to_string(), &fx.home, &fx.home, &find_git_root);
        assert_eq!(offer.suggestion().unwrap().calls, 3);
    }

    #[test]
    fn the_keys_belong_to_the_card_only_while_idle() {
        let fx = fixture("keys");
        let mut offer = ProjectOffer::default();
        assert!(!offer.captures_keys(false));
        for _ in 0..3 {
            observe(&mut offer, &fx, &fx.home, "cd code/forge && ls");
        }
        assert!(offer.captures_keys(false));
        assert!(!offer.captures_keys(true));
    }

    #[test]
    fn quoted_separators_do_not_split_a_command() {
        assert_eq!(split_chain("cd 'a;b' && ls"), vec!["cd 'a;b' ", " ls"]);
        assert_eq!(split_chain("a; b\nc || d"), vec!["a", " b", "c ", " d"]);
    }
}
