//! `decree check` on `examples/feature/` and the `tests/fixtures/escalation/` ladder
//! (docs/reference/machines.md, Validation): it passes without a warning, warns when `graph/` is out of date, and names the
//! rule when an example machine is broken. The rule-by-rule cases are in `validation_test.rs`.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// `<project>/.decree`, `project` relative to the repository.
fn decree_dir(project: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(project)
        .join(".decree")
}

/// `examples/<name>/.decree`.
fn example(name: &str) -> PathBuf {
    decree_dir(&format!("examples/{name}"))
}

/// The escalation ladder (`file_document`): two checks, two models and a person, with its
/// recorded run.
const ESCALATION: &str = "tests/fixtures/escalation";

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

/// Run `decree check` in a copy of `project` (relative to the repository): (exit code,
/// stdout, stderr).
fn check_project(project: &str, edit: impl FnOnce(&Path)) -> (i32, String, String) {
    let tmp = TempDir::new().unwrap();
    copy_dir(&decree_dir(project), &tmp.path().join(".decree"));
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
fn check_passes_on_the_recorded_projects_without_a_warning() {
    for project in ["examples/feature", ESCALATION] {
        assert_eq!(
            check_project(project, |_| {}),
            (0, String::new(), String::new()),
            "{project}"
        );
    }
}

/// The docs/reference/machines.md examples, alone, with a stub `scripts/<name>` for each script.
#[test]
fn check_passes_on_the_reference_examples() {
    let tmp = TempDir::new().unwrap();
    let decree = tmp.path().join(".decree");
    fs::create_dir_all(decree.join("machines")).unwrap();
    for name in ["hello", "deploy", "ship", "feature", "router"] {
        let file = format!("machines/{name}.yml");
        fs::copy(example("feature").join(&file), decree.join(&file)).unwrap();
    }
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
    let out = cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .env("NO_COLOR", "1")
        .arg("check")
        .output()
        .unwrap();
    assert_eq!(
        (out.status.code(), out.stdout.as_slice()),
        (Some(0), &b""[..])
    );
}

#[test]
fn check_warns_when_the_graph_files_are_out_of_date() {
    let (code, stdout, stderr) = check_project("examples/feature", |decree| {
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

/// A new machine makes `graph/` stale until `decree graph` rewrites it.
#[test]
fn check_warns_until_the_graph_is_rewritten() {
    let tmp = TempDir::new().unwrap();
    copy_dir(&example("feature"), &tmp.path().join(".decree"));
    fs::write(
        tmp.path().join(".decree/machines/greet.yml"),
        "name: greet\ndescription: Run hello.\ninitial: work\nstates:\n  \
         work:\n    invoke: { machine: hello }\n    transitions: { done: done }\n  \
         done: { final: true }\n  failed: { final: true }\n",
    )
    .unwrap();
    let run = |cmd: &str| {
        let out = cargo_bin_cmd!("decree")
            .current_dir(tmp.path())
            .env("NO_COLOR", "1")
            .arg(cmd)
            .output()
            .unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    };
    let (code, stderr) = run("check");
    assert_eq!(code, 0);
    assert!(
        stderr.contains("warning: graph/greet.md: missing"),
        "{stderr}"
    );
    assert_eq!(run("graph").0, 0);
    assert_eq!(run("check"), (0, String::new()));
}

#[test]
fn check_warns_when_the_graph_directory_is_missing() {
    let (code, _, stderr) = check_project("examples/feature", |decree| {
        fs::remove_dir_all(decree.join("graph")).unwrap()
    });
    assert_eq!(code, 0);
    assert_eq!(stderr.lines().count(), 8, "{stderr}");
    assert!(
        stderr.contains("warning: graph/system.md: missing"),
        "{stderr}"
    );
}

/// The docs/reference/machines.md messages for the shapes this model rejects.
#[test]
fn check_rejects_the_old_shapes_with_the_reference_messages() {
    let cases = [
        (
            "feature.yml",
            "        transitions: { done: verify }\n",
            "        transitions: { done: { target: verify, cond: \"visits.implement < 2\" } }\n",
            "machines/feature.yml: work.implement: transition `done`: cond on a transition is not supported: make the decision a state with invoke: { check: ... } (V19)",
        ),
        (
            "deploy.yml",
            "        question: Ship this build?\n",
            "",
            "machines/deploy.yml: approval: a `person` state needs a `question`: what is being decided (V8)",
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
        let (code, stdout, _) = check_project("examples/feature", |decree| {
            let path = decree.join("machines").join(file);
            let text = fs::read_to_string(&path).unwrap();
            assert!(text.contains(from), "{file}: {from}");
            fs::write(&path, text.replacen(from, to, 1)).unwrap();
        });
        assert_eq!(code, 1, "{expected}");
        assert_eq!(stdout, format!("{expected}\n"));
    }
    // A router state inside a compound state, as in the V19 case of `validation_test.rs`.
    let (code, stdout, _) = check_project("examples/feature", |decree| {
        fs::write(
            decree.join("machines/b.yml"),
            "name: b\ndescription: A router state inside a compound state.\ninitial: work\n\
             states:\n  work:\n    initial: step\n    transitions: { done.state.work: done }\n    \
             states:\n      step:\n        invoke: work\n        router: llm\n        transitions:\n          \
             pass: { target: fin, description: The work is finished. }\n          \
             retry: { target: step, description: Run the work again. }\n      fin: { final: true }\n  \
             done: { final: true }\n  failed: { final: true }\n",
        )
        .unwrap();
    });
    assert_eq!(code, 1);
    assert_eq!(
        stdout,
        "machines/b.yml: work.step: router on a state is not supported: make the decision a state with invoke: { model: { question: ... } } (V19)\n"
    );
}

/// The escalation conditions in `file_document` broken one at a time (V10).
#[test]
fn check_rejects_bad_escalation_conditions_with_v10() {
    let cases = [
        (
            "file: { type: string, default: \"\" }",
            "file: { type: int, default: 0 }",
            "machines/file_document.yml: by_name: check: `matches` needs string data, but `file` is int (V10)",
        ),
        (
            "confidence: big_model",
            "confidence: read_text",
            "machines/file_document.yml: worth_asking: check: `confidence` names `read_text`, which is not a `model` state (V10)",
        ),
        (
            "at_least: 0.4",
            "at_least: 1.5",
            "machines/file_document.yml: worth_asking: check: `confidence` compares to a number from 0 to 1, not 1.5 (V10)",
        ),
    ];
    for (from, to, expected) in cases {
        let (code, stdout, _) = check_project(ESCALATION, |decree| {
            let path = decree.join("machines/file_document.yml");
            let text = fs::read_to_string(&path).unwrap();
            assert!(text.contains(from), "{from}");
            fs::write(&path, text.replacen(from, to, 1)).unwrap();
        });
        assert_eq!(code, 1, "{expected}");
        assert_eq!(stdout, format!("{expected}\n"));
    }
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
