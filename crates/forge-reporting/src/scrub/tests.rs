use super::*;

fn id() -> Identity {
    Identity {
        home: Some("/home/alice".to_string()),
        user: Some("alice".to_string()),
        host: Some("alice-laptop".to_string()),
    }
}

#[test]
fn home_becomes_tilde_and_other_absolute_paths_keep_only_the_basename() {
    let out = scrub_with(
        "failed to open /home/alice/work/secret-project/src/main.rs and /etc/ssl/private/key.pem",
        &id(),
    );
    assert!(out.contains("~/work/secret-project/src/main.rs"), "{out}");
    assert!(out.contains("key.pem"));
    assert!(!out.contains("/home/alice"));
    assert!(!out.contains("/etc/ssl"));
}

#[test]
fn windows_paths_keep_only_the_basename() {
    let out = scrub_with(
        r"cannot read C:\Users\alice\Documents\notes\todo.txt now",
        &id(),
    );
    assert!(out.contains("todo.txt"));
    assert!(!out.contains("Users"));
}

#[test]
fn secrets_are_redacted_by_shape_and_by_keyword() {
    let key = "sk-proj-abcdefghijklmnopqrstuvwxyz0123456789";
    let gh = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    let out = scrub_with(
        &format!(
            "auth failed with {key} and {gh}; Authorization: Bearer abcdef1234567890xyz; api_key=hunter2hunter2; token: \"s3cr3t-value\""
        ),
        &id(),
    );
    for leaked in [
        key,
        gh,
        "abcdef1234567890xyz",
        "hunter2hunter2",
        "s3cr3t-value",
    ] {
        assert!(!out.contains(leaked), "leaked {leaked}: {out}");
    }
}

#[test]
fn long_opaque_tokens_are_redacted_but_plain_words_are_not() {
    let out = scrub_with(
        "id 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08 and a_very_long_identifier_name_without_digits_at_all",
        &id(),
    );
    assert!(!out.contains("9f86d081884c"));
    assert!(out.contains("a_very_long_identifier_name_without_digits_at_all"));
}

#[test]
fn registered_redactor_runs_first() {
    set_redactor(|text| text.replace("plain-looking-secret", "<known>"));
    let out = scrub_with("value plain-looking-secret here", &id());
    assert!(out.contains("<known>"));
    *REDACTOR.write().unwrap() = None;
}

#[test]
fn urls_keep_only_a_public_host() {
    let out = scrub_with(
        "GET https://user:pw@api.example.com:8443/v1/chat?key=abc#frag failed; also http://10.0.0.5:3000/x and http://box.local/y and https://alice-laptop.example.org/z",
        &id(),
    );
    assert!(out.contains("https://api.example.com"), "{out}");
    assert!(!out.contains("pw"));
    assert!(!out.contains("/v1/chat"));
    assert!(!out.contains("10.0.0.5"));
    assert!(!out.contains("box.local"));
    assert!(!out.contains("alice-laptop"));
    assert!(out.contains("<internal-host>"));
}

#[test]
fn emails_user_and_host_names_are_removed() {
    let out = scrub_with(
        "mail alice@example.com from alice on alice-laptop via 192.168.1.20",
        &id(),
    );
    for leaked in ["alice@example.com", "alice-laptop", "192.168.1.20"] {
        assert!(!out.contains(leaked), "leaked {leaked}: {out}");
    }
    assert!(!out.to_lowercase().contains("alice"), "{out}");
}

#[test]
fn messages_lose_quoted_content_backticks_and_blobs() {
    let out = scrub_message_with(
        r#"called Result::unwrap() on an Err value: Parse("my secret prompt about the merger") for `fn deploy_to_prod()` args {"cmd": "rm -rf /", "content": "file body"} and 'single quoted text'"#,
        &id(),
        300,
    );
    for leaked in [
        "merger",
        "deploy_to_prod",
        "rm -rf",
        "file body",
        "single quoted",
    ] {
        assert!(!out.contains(leaked), "leaked {leaked}: {out}");
    }
    assert!(out.contains("Result::unwrap()"));
}

#[test]
fn apostrophes_in_prose_do_not_swallow_the_sentence() {
    let out = scrub_message_with("can't parse the plan, isn't valid", &id(), 300);
    assert!(out.contains("can't parse the plan"), "{out}");
}

#[test]
fn truncation_marks_the_cut_and_flattens_whitespace() {
    let out = truncate(&"word ".repeat(200), 40);
    assert!(out.chars().count() <= 40);
    assert!(out.ends_with('…'));
    assert!(!out.contains('\n'));
}

#[test]
fn locations_are_normalised_to_in_repo_form() {
    assert_eq!(
        normalize_location("/home/alice/forge/crates/forge-core/src/a.rs:12:5"),
        "forge-core/src/a.rs:12"
    );
    assert_eq!(
        normalize_location("/home/alice/.cargo/registry/src/idx-1/tokio-1.0/src/x/y.rs:7"),
        "src/x/y.rs:7"
    );
    assert_eq!(
        normalize_location("crates/forge-cli/src/main.rs:3"),
        "forge-cli/src/main.rs:3"
    );
}
