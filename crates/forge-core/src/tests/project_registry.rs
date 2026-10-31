use super::*;

#[test]
fn rename_hide_and_relocate_round_trip_through_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let new_home = dir.path().join("moved");
    std::fs::create_dir_all(&new_home).unwrap();
    let file = dir.path().join("projects.json");
    let mut registry = ProjectRegistry::default();
    registry
        .apply(
            "/old/app",
            &ProjectPatch {
                name: Some("  My App ".into()),
                disabled: Some(true),
                relocate_to: Some(new_home.display().to_string()),
            },
        )
        .unwrap();
    registry.save_to(&file).unwrap();
    let loaded = ProjectRegistry::load_from(&file);
    let entry = loaded.entry("/old/app").unwrap();
    assert_eq!(entry.name.as_deref(), Some("My App"));
    assert!(entry.disabled);
    assert_eq!(loaded.resolve("/old/app"), new_home.display().to_string());
}

#[test]
fn clearing_every_field_drops_the_entry() {
    let mut registry = ProjectRegistry::default();
    registry
        .apply(
            "/p",
            &ProjectPatch {
                disabled: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
    registry
        .apply(
            "/p",
            &ProjectPatch {
                disabled: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(registry.entries.is_empty());
}

#[test]
fn bad_relocations_and_relative_paths_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = ProjectRegistry::default();
    let to = |s: &str| ProjectPatch {
        relocate_to: Some(s.into()),
        ..Default::default()
    };
    assert!(registry.apply("relative", &to("/")).is_err());
    assert!(registry.apply("/p", &to("/does/not/exist")).is_err());
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let (a, b) = (a.display().to_string(), b.display().to_string());
    registry.apply(&a, &to(&b)).unwrap();
    assert!(registry.apply(&b, &to(&a)).is_err(), "would loop");
}

#[test]
fn resolve_terminates_on_a_cycle() {
    let mut registry = ProjectRegistry::default();
    registry.entries.entry("/a".into()).or_default().moved_to = Some("/b".into());
    registry.entries.entry("/b".into()).or_default().moved_to = Some("/a".into());
    let _ = registry.resolve("/a");
}

#[test]
fn merge_into_points_duplicates_at_the_kept_path() {
    let mut registry = ProjectRegistry::default();
    registry.merge_into("/keep", &["/keep".into(), "/dup".into()]);
    assert_eq!(registry.resolve("/dup"), "/keep");
    assert!(registry.entry("/keep").is_none());
}
