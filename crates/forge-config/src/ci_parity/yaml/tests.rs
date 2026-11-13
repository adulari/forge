use super::*;

fn s(v: &str) -> Yaml {
    Yaml::Str(v.to_string())
}

fn ok(src: &str) -> Yaml {
    parse(src).unwrap_or_else(|e| panic!("{e}\n{src}"))
}

fn err(src: &str) -> String {
    parse(src).expect_err(src).msg
}

#[test]
fn maps_sequences_and_nesting() {
    let y = ok("a: 1\nb:\n  - x\n  - y: 2\n    z: 3\nc:\n- p\n- q\nd: {k: v, n: [1, 2]}\n");
    assert_eq!(y.get("a"), Some(&s("1")));
    let b = y.get("b").and_then(Yaml::as_seq).unwrap();
    assert_eq!(b[0], s("x"));
    assert_eq!(b[1].get("z"), Some(&s("3")));
    assert_eq!(y.get("c").and_then(Yaml::as_seq).unwrap().len(), 2);
    let d = y.get("d").unwrap();
    assert_eq!(d.get("k"), Some(&s("v")));
    assert_eq!(d.get("n").and_then(Yaml::as_seq).unwrap(), [s("1"), s("2")]);
}

#[test]
fn comments_and_quotes() {
    let y =
        ok("a: x # note\nb: \"q # not\" # note\nc: 'it''s'\nd: 'a#b'\n# whole\ne: echo '#' ok\n");
    assert_eq!(y.get("a"), Some(&s("x")));
    assert_eq!(y.get("b"), Some(&s("q # not")));
    assert_eq!(y.get("c"), Some(&s("it's")));
    assert_eq!(y.get("d"), Some(&s("a#b")));
    assert_eq!(y.get("e"), Some(&s("echo '#' ok")));
}

#[test]
fn non_ascii_plain_scalars_parse_without_panicking() {
    let y = ok("a: Build — release ✓\nb: Café ✓ # note\nc: [é, ü]\nd: ${{ x }} — ✓ # c\n");
    assert_eq!(y.get("a"), Some(&s("Build — release ✓")));
    assert_eq!(y.get("b"), Some(&s("Café ✓")));
    assert_eq!(y.get("c").and_then(Yaml::as_seq).unwrap(), [s("é"), s("ü")]);
    assert_eq!(y.get("d"), Some(&s("${{ x }} — ✓")));
}

#[test]
fn expressions_stay_verbatim_and_do_not_split_keys() {
    let y = ok("env:\n  A: ${{ secrets.X }}\n  B: ${{ a == 'b' && c || d }}\nif: ${{ github.event_name == 'push' }}\n${{ matrix.k }}: v\n");
    let env = y.get("env").unwrap();
    assert_eq!(env.get("A"), Some(&s("${{ secrets.X }}")));
    assert_eq!(env.get("B"), Some(&s("${{ a == 'b' && c || d }}")));
    assert_eq!(y.get("if"), Some(&s("${{ github.event_name == 'push' }}")));
    assert_eq!(y.get("${{ matrix.k }}"), Some(&s("v")));
}

#[test]
fn block_scalars_literal_folded_and_chomping() {
    let y = ok("a: |\n  one\n   two\n\n  three\nb: |-\n  x\n  y\nc: >\n  fold\n  me\n\n  para\nd: >-\n  a\n  b\ne: |+\n  k\n\nf: end\n");
    assert_eq!(y.get("a"), Some(&s("one\n two\n\nthree\n")));
    assert_eq!(y.get("b"), Some(&s("x\ny")));
    assert_eq!(y.get("c"), Some(&s("fold me\npara\n")));
    assert_eq!(y.get("d"), Some(&s("a b")));
    assert_eq!(y.get("e"), Some(&s("k\n\n")));
    assert_eq!(y.get("f"), Some(&s("end")));
}

#[test]
fn block_scalar_keeps_hash_lines_and_colons_as_text() {
    let y = ok("run: |\n  # not a comment\n  echo a: b\n  - not a list\nnext: 1\n");
    assert_eq!(
        y.get("run"),
        Some(&s("# not a comment\necho a: b\n- not a list\n"))
    );
    assert_eq!(y.get("next"), Some(&s("1")));
}

#[test]
fn block_scalar_in_a_sequence_item() {
    let y = ok("steps:\n  - run: |\n      a\n      b\n    name: n\n  - |\n    lone\n");
    let steps = y.get("steps").and_then(Yaml::as_seq).unwrap();
    assert_eq!(steps[0].get("run"), Some(&s("a\nb\n")));
    assert_eq!(steps[0].get("name"), Some(&s("n")));
    assert_eq!(steps[1], s("lone\n"));
}

#[test]
fn multi_line_plain_quoted_and_flow_values() {
    let y = ok("a: one\n  two\n  three\nb: \"x\n  y\"\nc: [a,\n  b,\n  c]\nd:\n  deep value\n");
    assert_eq!(y.get("a"), Some(&s("one two three")));
    assert_eq!(y.get("b"), Some(&s("x y")));
    assert_eq!(y.get("c").and_then(Yaml::as_seq).unwrap().len(), 3);
    assert_eq!(y.get("d"), Some(&s("deep value")));
}

#[test]
fn flow_with_expressions_and_empty_values() {
    let y = ok("os: [${{ matrix.a }}, self-hosted]\ne: {}\nl: []\nn:\nm: ~\n");
    assert_eq!(
        y.get("os").and_then(Yaml::as_seq).unwrap(),
        [s("${{ matrix.a }}"), s("self-hosted")]
    );
    assert_eq!(y.get("e"), Some(&Yaml::Map(vec![])));
    assert_eq!(y.get("l"), Some(&Yaml::Seq(vec![])));
    assert_eq!(y.get("n"), Some(&Yaml::Null));
    assert_eq!(y.get("m"), Some(&s("~")));
}

#[test]
fn leading_document_marker_and_crlf_are_fine() {
    let y = ok("---\r\na: 1\r\nb:\r\n  - x\r\n");
    assert_eq!(y.get("a"), Some(&s("1")));
}

#[test]
fn unsupported_constructs_are_errors_not_guesses() {
    assert!(err("a: &x 1\n").contains("anchors"));
    assert!(err("a: *x\n").contains("anchors"));
    assert!(err("a: !!str 1\n").contains("tags"));
    assert!(err("<<: {a: 1}\n").contains("merge"));
    assert!(err("? a\n: b\n").contains("explicit"));
    assert!(err("a: 1\n---\nb: 2\n").contains("multiple"));
    assert!(err("a: 1\na: 2\n").contains("duplicate"));
    assert!(err("a:\n\tb: 1\n").contains("tab"));
    assert!(err("a: |2\n  x\n").contains("indentation"));
    assert!(err("a: \"open\n").contains("unterminated"));
    assert!(err("a: [1, 2\n").contains("unterminated"));
    assert!(err("a: x\n  b: y\n").contains("mapping entry"));
    assert!(
        err("a: 1\n b: 2\n").contains("indentation") || err("a: 1\n b: 2\n").contains("mapping")
    );
}

#[test]
fn error_carries_the_line() {
    let e = parse("a: 1\nb: 2\nb: 3\n").unwrap_err();
    assert_eq!(e.line, 3);
}

#[test]
fn every_workflow_in_this_repo_either_parses_or_is_reported() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows");
    let mut parsed = 0;
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "yml") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        match parse(&text) {
            Ok(y) => {
                assert!(y.get("jobs").is_some(), "{} has no jobs", path.display());
                parsed += 1;
            }
            Err(e) => panic!("{}: {e}", path.display()),
        }
    }
    assert!(parsed >= 10);
}

#[test]
fn ci_yml_shapes_survive() {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows/ci.yml"),
    )
    .unwrap();
    let y = ok(&text);
    assert_eq!(
        y.get("env").unwrap().get("RUSTFLAGS"),
        Some(&s("-D warnings"))
    );
    let jobs = y.get("jobs").unwrap();
    let clippy = jobs.get("clippy").unwrap();
    let runs_on = clippy.get("runs-on").and_then(Yaml::as_seq).unwrap();
    assert!(runs_on.contains(&s("linux")));
    let steps = clippy.get("steps").and_then(Yaml::as_seq).unwrap();
    assert!(steps
        .iter()
        .any(|st| st.get("run") == Some(&s("bash scripts/ci/rust-checks.sh"))));
    let on = y.get("on").unwrap();
    assert!(on.get("pull_request").is_some() && on.get("schedule").is_some());
    let guard = jobs
        .get("CI")
        .unwrap()
        .get("steps")
        .and_then(Yaml::as_seq)
        .unwrap();
    let script = guard[0].get("run").and_then(Yaml::as_str).unwrap();
    assert!(script.contains("for entry in") && script.contains("${{ needs.fmt.result }}"));
}

#[test]
fn deep_nesting_is_an_error_not_a_stack_overflow() {
    let n = 100_000;
    let flow = format!("jobs: {}{}", "[".repeat(n), "]".repeat(n));
    let flow_map = format!("jobs: {}{}", "{a: ".repeat(n), "}".repeat(n));
    let block = format!("{}x", "- ".repeat(n));
    let handle = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            [flow, flow_map, block]
                .iter()
                .map(|src| parse(src).map(|_| ()))
                .collect::<Vec<_>>()
        })
        .unwrap();
    for result in handle.join().unwrap() {
        assert!(result.unwrap_err().msg.contains("nesting too deep"));
    }
    let nested = format!("a: {}1{}", "[".repeat(32), "]".repeat(32));
    assert!(parse(&nested).is_ok());
}
