//! `load()` reports where a bad config value is. Own integration binary: it redirects
//! `XDG_CONFIG_HOME` and the working directory, which would race unit tests in a shared process.

#[test]
fn a_type_error_names_file_line_and_column_and_unknown_keys_do_not_fail() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", dir.path());
    std::env::set_var("HOME", dir.path());
    std::env::set_current_dir(dir.path()).unwrap();
    let forge_dir = dir.path().join("forge");
    std::fs::create_dir_all(&forge_dir).unwrap();
    let path = forge_dir.join("config.toml");

    std::fs::write(&path, "permission_mode = \"default\"\nnot_a_real_key = 1\n").unwrap();
    forge_config::load().expect("an unknown key warns but still loads");

    std::fs::write(&path, "# c\n\n[mesh]\ncredit_mode = 5\n").unwrap();
    let message = forge_config::load().unwrap_err().to_string();
    assert!(message.contains("config.toml:4:"), "{message}");

    std::fs::write(&path, "permission_mode = \"default\"\n[mesh\n").unwrap();
    let message = forge_config::load().unwrap_err().to_string();
    assert!(
        message.contains("line 2") || message.contains(":2:"),
        "{message}"
    );
}
