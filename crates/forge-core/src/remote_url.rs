//! Validation for anything a client hands the daemon that git will treat as a remote.
//!
//! A git remote URL is code-adjacent input: `ext::sh -c ...` runs a command, `file:` reads the
//! daemon host's disk, `fd::` reuses an open descriptor, and a leading `-` is read as an option.
//! Everything the daemon accepts as a remote or clone URL goes through [`validate_remote_url`]
//! (clone-from-URL features reuse it), and the network git commands additionally run with
//! [`NETWORK_ENV`] so a URL already in the repository's config cannot reach those transports.

/// Environment for every git command that touches a remote. `GIT_ALLOW_PROTOCOL` is an allow-list,
/// so `ext` and `fd` stay unreachable even when a config file names them.
pub const NETWORK_ENV: [(&str, &str); 3] = [
    ("GIT_ALLOW_PROTOCOL", "https:http:ssh:git:file"),
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GCM_INTERACTIVE", "never"),
];

const URL_SCHEMES: [&str; 4] = ["https", "http", "ssh", "git"];

/// Accept `https://`, `http://`, `ssh://` and `git://` URLs and scp-style `user@host:path`.
/// Everything else is refused: `ext::`/`fd::`/`<helper>::` remote-helper syntax, `file:` and bare
/// local paths, option-looking values, and anything with control characters.
pub fn validate_remote_url(raw: &str) -> Result<(), String> {
    let url = raw.trim();
    if url.is_empty() {
        return Err("remote URL is empty".to_string());
    }
    if url.starts_with('-') {
        return Err("remote URL must not start with '-'".to_string());
    }
    let lower = url.to_ascii_lowercase();
    if lower.contains("::") {
        return Err("remote-helper URLs (ext::, fd::, ...) are not accepted".to_string());
    }
    if lower.starts_with("file:") {
        return Err("file: URLs are not accepted".to_string());
    }
    if url.chars().any(|c| c.is_control() || c == ' ') {
        return Err("remote URL must not contain spaces or control characters".to_string());
    }
    if let Some((scheme, rest)) = lower.split_once("://") {
        if !URL_SCHEMES.contains(&scheme) {
            return Err(format!("the {scheme}:// scheme is not accepted"));
        }
        let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let host = host.rsplit('@').next().unwrap_or_default();
        if host.is_empty() || host.starts_with('-') {
            return Err("remote URL has no valid host".to_string());
        }
        return Ok(());
    }
    // scp-like: [user@]host:path, with no slash before the colon (that would be a local path).
    match url.split_once(':') {
        Some((host_part, path)) if !path.is_empty() && !host_part.contains('/') => {
            let host = host_part.rsplit('@').next().unwrap_or_default();
            if host.is_empty() || host.starts_with('-') {
                return Err("remote URL has no valid host".to_string());
            }
            Ok(())
        }
        _ => Err("remote must be an https, ssh or git URL".to_string()),
    }
}

/// A remote the request names: either one of the repository's configured remote names, or a URL
/// that passes [`validate_remote_url`]. Names are restricted to the characters git itself allows
/// in practice and may not begin with `-`.
pub fn validate_remote(raw: &str, configured: &[String]) -> Result<String, String> {
    let remote = raw.trim();
    if !remote.starts_with('-') && configured.iter().any(|name| name == remote) {
        return Ok(remote.to_string());
    }
    if remote.contains("://") || remote.contains(':') || remote.contains('@') {
        validate_remote_url(remote)?;
        return Ok(remote.to_string());
    }
    if remote.is_empty() {
        return Err("remote is empty".to_string());
    }
    if remote.starts_with('-') {
        return Err("remote must not start with '-'".to_string());
    }
    Err(format!("no remote named '{remote}'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_remote_urls_pass() {
        for url in [
            "https://github.com/adulari/forge.git",
            "http://localhost:3000/x.git",
            "ssh://git@host.example/x.git",
            "git://host/x.git",
            "git@github.com:adulari/forge.git",
            "host:path/to/repo.git",
        ] {
            assert!(validate_remote_url(url).is_ok(), "{url}");
        }
    }

    #[test]
    fn code_running_and_local_transports_are_refused() {
        for url in [
            "ext::sh -c touch /tmp/pwned",
            "ext::sh -c 'id'",
            "EXT::sh -c id",
            "fd::3",
            "git-remote-foo::addr",
            "file:///etc/passwd",
            "FILE:/etc/passwd",
            "/srv/git/repo.git",
            "../repo.git",
            "--upload-pack=touch /tmp/x",
            "-oProxyCommand=id",
            "ssh://-oProxyCommand=id/x",
            "https:// host/x",
            "ftp://host/x",
            "",
            "   ",
            "https://\nhost/x",
            "https:///x",
        ] {
            assert!(validate_remote_url(url).is_err(), "{url:?}");
        }
    }

    #[test]
    fn configured_names_pass_and_everything_else_is_checked() {
        let names = vec!["origin".to_string(), "up-stream".to_string()];
        assert_eq!(validate_remote("origin", &names).as_deref(), Ok("origin"));
        assert!(validate_remote("nope", &names).is_err());
        assert!(validate_remote("-origin", &names).is_err());
        assert!(validate_remote("ext::sh -c id", &names).is_err());
        assert!(validate_remote("https://github.com/a/b.git", &names).is_ok());
    }
}
