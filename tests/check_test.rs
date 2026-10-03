//! `decree check` against the fixtures in `tests/fixtures/check/` (spec section 5,
//! Validation). Each `<case>/pass/` and `<case>/fail/` is copied into a temp project as its
//! `.decree/`; `<case>/expected.txt` is the exact stdout of the failing one.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const CONFIG: &str = "max_attempts: 3\n";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Copy `src` to `dst` recursively; `fs::copy` keeps the execute bits.
fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to);
        } else {
            fs::copy(entry.path(), &to).unwrap();
        }
    }
}

/// A temp project whose `.decree/` is a copy of `tree`, with a default `config.yml` unless
/// the tree has its own.
fn project(tree: &Path) -> TempDir {
    let tmp = TempDir::new().unwrap();
    let decree = tmp.path().join(".decree");
    copy_dir(tree, &decree);
    if !decree.join("config.yml").exists() {
        fs::write(decree.join("config.yml"), CONFIG).unwrap();
    }
    tmp
}

/// Run `decree check` in `tmp`: (exit code, stdout).
fn check(tmp: &TempDir) -> (i32, String) {
    let out = cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .env("NO_COLOR", "1")
        .arg("check")
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
    )
}

/// The valid tree exits 0 and prints nothing; the invalid one exits 1 and prints exactly
/// `expected.txt`, which names the rule.
fn run_case(case: &str) {
    let dir = fixtures().join("check").join(case);
    let rule = case
        .split('-')
        .next()
        .unwrap()
        .to_uppercase()
        .replace("V0", "V");

    let (code, stdout) = check(&project(&dir.join("pass")));
    assert_eq!((code, stdout.as_str()), (0, ""), "{case}/pass");

    let (code, stdout) = check(&project(&dir.join("fail")));
    let expected = fs::read_to_string(dir.join("expected.txt")).unwrap();
    assert_eq!(code, 1, "{case}/fail: {stdout}");
    assert_eq!(stdout, expected, "{case}/fail");
    for line in stdout.lines() {
        assert!(line.ends_with(&format!("({rule})")), "{case}: {line}");
    }
}

#[test]
fn check_v1_name() {
    run_case("v01-name");
}

#[test]
fn check_v2_state_ids() {
    run_case("v02-state-ids");
}

#[test]
fn check_v3_initial() {
    run_case("v03-initial");
}

#[test]
fn check_v4_targets() {
    run_case("v04-targets");
}

#[test]
fn check_v5_failed() {
    run_case("v05-failed");
}

#[test]
fn check_v6_compound() {
    run_case("v06-compound");
}

#[test]
fn check_v7_final() {
    run_case("v07-final");
}

#[test]
fn check_v8_decisions() {
    run_case("v08-decisions");
}

#[test]
fn check_v9_input() {
    run_case("v09-input");
}

#[test]
fn check_v10_condition() {
    run_case("v10-condition");
}

#[test]
fn check_v11_reachable() {
    run_case("v11-reachable");
}

#[test]
fn check_v12_scripts() {
    run_case("v12-scripts");
}

#[test]
fn check_v13_emits() {
    run_case("v13-emits");
}

#[test]
fn check_v14_data() {
    run_case("v14-data");
}

#[test]
fn check_v15_done_state() {
    run_case("v15-done-state");
}

#[test]
fn check_v16_invokes() {
    run_case("v16-invokes");
}

#[test]
fn check_v17_internal() {
    run_case("v17-internal");
}

#[test]
fn check_v18_events() {
    run_case("v18-events");
}

#[test]
fn check_v19_scxml() {
    run_case("v19-scxml");
}

#[test]
fn check_v20_cycles() {
    run_case("v20-cycles");
}

#[test]
fn check_m1_migrations() {
    run_case("m1-migrations");
}

#[test]
fn check_m2_inbox() {
    run_case("m2-inbox");
}

#[test]
fn check_m3_cron() {
    run_case("m3-cron");
}

#[test]
fn check_several_invalid_machines_prints_one_line_per_error() {
    let dir = fixtures().join("check/many");
    let (code, stdout) = check(&project(&dir.join("fail")));
    let expected = fs::read_to_string(dir.join("expected.txt")).unwrap();
    assert_eq!(code, 1);
    assert_eq!(stdout, expected);
    // Four machines, each wrong in its own way: every error is its own line.
    let files: Vec<&str> = stdout
        .lines()
        .map(|l| l.split(':').next().unwrap())
        .collect();
    assert_eq!(files.len(), 6, "{stdout}");
    for id in ["a", "b", "c", "d"] {
        assert!(
            files.contains(&format!("machines/{id}.yml").as_str()),
            "{stdout}"
        );
    }
}

#[test]
fn check_passes_on_the_section_5_examples() {
    let tmp = TempDir::new().unwrap();
    let decree = tmp.path().join(".decree");
    copy_dir(&fixtures().join("machines"), &decree.join("machines"));
    fs::remove_dir_all(decree.join("machines/step")).unwrap();
    fs::write(
        decree.join("config.yml"),
        format!("{CONFIG}default_router: claude_router\n"),
    )
    .unwrap();
    let scripts = decree.join("scripts");
    fs::create_dir_all(&scripts).unwrap();
    for name in [
        "greet",
        "build",
        "ask_person",
        "ship",
        "git_baseline",
        "notify",
        "precheck",
        "implement",
        "snapshot",
        "collect_logs",
        "verify",
        "spawn",
        "commit",
        "ask_claude",
    ] {
        let path = scripts.join(name);
        fs::write(&path, "#!/usr/bin/env bash\nexit 0\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert_eq!(check(&tmp), (0, String::new()));
}

/// Run `decree check` in a copy of `mock/`: (exit code, stdout, stderr).
fn check_mock(edit: impl FnOnce(&Path)) -> (i32, String, String) {
    let tmp = TempDir::new().unwrap();
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("mock/.decree"),
        &tmp.path().join(".decree"),
    );
    edit(&tmp.path().join(".decree"));
    let out = cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .env("NO_COLOR", "1")
        .arg("check")
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn check_passes_on_mock_without_a_warning() {
    assert_eq!(check_mock(|_| {}), (0, String::new(), String::new()));
}

#[test]
fn check_warns_when_the_graph_files_are_out_of_date() {
    let (code, stdout, stderr) = check_mock(|decree| {
        let graph = decree.join("graph");
        fs::remove_file(graph.join("hello.md")).unwrap();
        fs::write(graph.join("deploy.md"), "# deploy\n").unwrap();
        fs::write(graph.join("old.md"), "# old\n").unwrap();
    });
    assert_eq!((code, stdout.as_str()), (0, ""));
    assert_eq!(
        stderr,
        "warning: graph/deploy.md: out of date; run `decree graph`\n\
         warning: graph/hello.md: missing; run `decree graph`\n\
         warning: graph/old.md: no machine draws it; run `decree graph`\n"
    );
}

#[test]
fn check_warns_when_the_graph_directory_is_missing() {
    let (code, _, stderr) = check_mock(|decree| fs::remove_dir_all(decree.join("graph")).unwrap());
    assert_eq!(code, 0);
    assert_eq!(stderr.lines().count(), 10, "{stderr}");
    assert!(
        stderr.contains("warning: graph/system.md: missing"),
        "{stderr}"
    );
}

/// The section 5 messages for the shapes this model rejects (migration 47).
#[test]
fn check_rejects_the_old_shapes_with_the_section_5_messages() {
    let cases = [
        (
            "feature.yml",
            "        transitions: { done: verify }\n",
            "        transitions: { done: { target: verify, cond: \"visits.implement < 2\" } }\n",
            "machines/feature.yml: work.implement: transition `done`: cond on a transition is not supported: make the decision a state with invoke: { check: ... } (V19)",
        ),
        (
            "deploy.yml",
            "question: \"Ship this build?\", ",
            "",
            "machines/deploy.yml: approval: a `choose: person` state needs a `question`: what is being decided (V8)",
        ),
        (
            "deploy.yml",
            "reject:  { target: rejected, description: Do not ship. }",
            "reject:  rejected",
            "machines/deploy.yml: approval: option `reject` needs a `description`: write it as `reject: { target: rejected, description: ... }` (V8)",
        ),
        (
            "ship.yml",
            "invoke: { machine: deploy }",
            "invoke: { machine: ship }",
            "machines/ship.yml: release: machine `ship` invokes itself: ship -> ship; a machine never invokes itself, directly or through others (V20)",
        ),
    ];
    for (file, from, to, expected) in cases {
        let (code, stdout, _) = check_mock(|decree| {
            let path = decree.join("machines").join(file);
            let text = fs::read_to_string(&path).unwrap();
            assert!(text.contains(from), "{file}: {from}");
            fs::write(&path, text.replacen(from, to, 1)).unwrap();
        });
        assert_eq!(code, 1, "{expected}");
        assert_eq!(stdout, format!("{expected}\n"));
    }
    // A router state, from the V19 fixture that holds one.
    let (code, stdout, _) = check_mock(|decree| {
        fs::copy(
            fixtures().join("check/v19-scxml/fail/machines/b.yml"),
            decree.join("machines/b.yml"),
        )
        .unwrap();
    });
    assert_eq!(code, 1);
    assert_eq!(
        stdout,
        "machines/b.yml: work.step: router on a state is not supported: make the decision a state with invoke: { choose: model, question: ... } (V19)\n"
    );
}

#[test]
fn check_outside_a_project_fails() {
    let tmp = TempDir::new().unwrap();
    let out = cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .arg("check")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
}
