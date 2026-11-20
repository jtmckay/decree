//! `decree check` on `examples/project/`, the `tests/fixtures/feature/` project and the
//! `tests/fixtures/escalation/` ladder (docs/reference/machines.md, Validation): it passes
//! without a warning, warns when `graph/` is out of date, and names the rule when a machine
//! is broken. The rule-by-rule cases are in `validation_test.rs`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

mod common;

/// `<project>/.decree`, `project` relative to the repository.
fn decree_dir(project: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(project)
        .join(".decree")
}

/// The small example project: `hello`, `develop` and `deploy`.
const PROJECT: &str = "examples/project";

/// The `feature` fixture: nesting, a check, a model's choice, a person, `data`, `emits`,
/// `onentry` and `onexit`, and `ship`, which invokes two machines.
const FEATURE: &str = "tests/fixtures/feature";

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

/// Copy a project's `.decree/` as a fresh clone has it: no `cache/`, and of each
/// `machines/<name>/` only what git tracks (`graph.md`, `order/`, `*.git.txt`), so what is
/// installed from `plugins/` is installed again.
fn copy_clone(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().into_owned();
        let to = dst.join(&name);
        match name.as_str() {
            "cache" => {}
            "machines" if src.join("plugins").is_dir() => {
                for m in fs::read_dir(entry.path()).unwrap() {
                    let m = m.unwrap();
                    let graph = m.path().join("graph.md");
                    if graph.is_file() {
                        fs::create_dir_all(to.join(m.file_name())).unwrap();
                        fs::copy(&graph, to.join(m.file_name()).join("graph.md")).unwrap();
                    }
                    let order = m.path().join("order");
                    if order.is_dir() {
                        copy_dir(&order, &to.join(m.file_name()).join("order"));
                    }
                }
            }
            _ if entry.file_type().unwrap().is_dir() => copy_dir(&entry.path(), &to),
            _ => {
                fs::copy(entry.path(), &to).unwrap();
            }
        }
    }
}

/// Run `decree check` in a copy of `project` (relative to the repository): (exit code,
/// stdout, stderr).
fn check_project(project: &str, edit: impl FnOnce(&Path)) -> (i32, String, String) {
    let tmp = TempDir::new().unwrap();
    copy_clone(&decree_dir(project), &tmp.path().join(".decree"));
    // A project with plugins (examples/project) has its machines installed first, as any
    // command does, so `edit` changes the installed copy, as a person edits it.
    if tmp.path().join(".decree/plugins").is_dir() {
        common::decree_at(tmp.path())
            .args(["plugin", "install"])
            .assert()
            .success();
    }
    edit(&tmp.path().join(".decree"));
    let out = common::decree_at(tmp.path())
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
    for project in [PROJECT, FEATURE, ESCALATION] {
        assert_eq!(
            check_project(project, |_| {}),
            (0, String::new(), String::new()),
            "{project}"
        );
    }
}

/// The docs/reference/machines.md examples: `examples/project`'s three machines, alone, and
/// `release`, which composes two of them, with a stub `scripts/<name>` for each script.
#[test]
fn check_passes_on_the_reference_examples() {
    let tmp = TempDir::new().unwrap();
    let decree = tmp.path().join(".decree");
    fs::create_dir_all(decree.join("machines")).unwrap();
    for name in ["hello", "develop", "deploy"] {
        common::write_file(
            &decree.join(format!("machines/{name}/machine.yml")),
            common::plugin_machine(&format!("{PROJECT}/.decree/plugins/{name}.md")),
        );
    }
    let reference = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/reference/machines.md"),
    )
    .unwrap();
    let release = reference
        .split("```yaml\n")
        .filter_map(|block| block.split_once("```").map(|(yaml, _)| yaml))
        .find(|yaml| yaml.contains("\nname: release\n"))
        .unwrap();
    common::write_file(&decree.join("machines/release/machine.yml"), release);
    let scripts = decree.join("scripts");
    fs::create_dir_all(&scripts).unwrap();
    for name in [
        "greet",
        "implement",
        "test",
        "build",
        "notify_person",
        "ship",
    ] {
        let path = scripts.join(name);
        fs::write(&path, "#!/usr/bin/env bash\nexit 0\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let out = common::decree_at(tmp.path())
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
    let (code, stdout, stderr) = check_project(PROJECT, |decree| {
        fs::remove_file(decree.join("machines/hello/graph.md")).unwrap();
        fs::write(decree.join("machines/deploy/graph.md"), "# deploy\n").unwrap();
        common::write_file(&decree.join("machines/old/graph.md"), "# old\n");
    });
    assert_eq!((code, stdout.as_str()), (0, ""));
    assert_eq!(
        stderr,
        "warning: machines/deploy/graph.md: out of date; run `decree graph`\n\
         warning: machines/hello/graph.md: missing; run `decree graph`\n\
         warning: machines/old/graph.md: no machine draws it; run `decree graph`\n"
    );
}

/// What a machine's `state/` holds that its `state_files:` does not declare is a warning,
/// not an error (docs/reference/machines.md, State).
#[test]
fn check_warns_about_undeclared_state_files() {
    let (code, stdout, stderr) = check_project(PROJECT, |decree| {
        let hello = decree.join("machines/hello/machine.yml");
        let text = fs::read_to_string(&hello).unwrap().replace(
            "initial: greet\n",
            "state_files:\n  seen.tsv: Who was greeted. greet reads and appends.\ninitial: greet\n",
        );
        fs::write(&hello, text).unwrap();
        let hello = decree.join("machines/hello/state");
        fs::create_dir_all(hello.join("cache")).unwrap();
        fs::write(hello.join("seen.tsv"), "ada\n").unwrap();
        fs::write(hello.join("other.txt"), "x\n").unwrap();
        common::write_file(&decree.join("machines/deploy/state/count"), "1\n");
    });
    assert_eq!((code, stdout.as_str()), (0, ""));
    let state: Vec<&str> = stderr.lines().filter(|l| l.contains("/state/")).collect();
    assert_eq!(
        state,
        [
            "warning: machines/deploy/state/count: is not declared in deploy's `state_files:`",
            "warning: machines/hello/state/cache: is not declared in hello's `state_files:`",
            "warning: machines/hello/state/other.txt: is not declared in hello's `state_files:`",
        ],
        "{stderr}"
    );
}

/// A machine that still says `store:` names the key that replaced it (V19).
#[test]
fn check_names_state_files_for_an_old_store_key() {
    let (code, stdout, _) = check_project(PROJECT, |decree| {
        let hello = decree.join("machines/hello/machine.yml");
        let text = fs::read_to_string(&hello).unwrap().replace(
            "initial: greet\n",
            "store:\n  seen.tsv: Who.\ninitial: greet\n",
        );
        fs::write(&hello, text).unwrap();
    });
    assert_eq!(code, 1);
    assert_eq!(
        stdout,
        "machines/hello/machine.yml: store: is now state_files: (the folder is machines/<name>/state/, $DECREE_STATE_DIR) (V19)\n"
    );
}

/// Write `migrations/<name>` for `hello` for each of `names`, and `processed.md` listing `ran`.
fn queue_migrations(decree: &Path, names: &[&str], ran: &[&str]) {
    for name in names {
        fs::write(
            decree.join("migrations").join(name),
            "---\nmachine: hello\n---\n# x\n",
        )
        .unwrap();
    }
    let mut ledger = fs::read_to_string(decree.join("processed.md")).unwrap();
    for name in ran {
        ledger.push_str(&format!("{name}\n"));
    }
    fs::write(decree.join("processed.md"), ledger).unwrap();
}

/// Run `decree check --format <format>` in a copy of `examples/project` with `01-a.md` and
/// `03-c.md` run and `02-b.md` pending: (exit code, stdout).
fn check_inserted_migration(format: &str) -> (i32, String) {
    let tmp = TempDir::new().unwrap();
    copy_clone(&decree_dir(PROJECT), &tmp.path().join(".decree"));
    queue_migrations(
        &tmp.path().join(".decree"),
        &["01-a.md", "02-b.md", "03-c.md"],
        &["01-a.md", "03-c.md"],
    );
    common::decree_at(tmp.path())
        .args(["plugin", "install"])
        .assert()
        .success();
    let out = common::decree_at(tmp.path())
        .args(["check", "--format", format])
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
    )
}

const INSERTED: &str = "migration 02-b.md sorts before 03-c.md, which already ran; it runs next. \
     To keep the order, pad numbers to the width the project needs (`001-`) or continue with a \
     letter prefix (`A01-`), which sorts after digits";

/// A pending migration that sorts before one that ran is a warning, not an error
/// (docs/reference/messages.md, Migrations): it runs next, after work that sorts later.
#[test]
fn check_warns_about_a_migration_inserted_before_ones_that_ran() {
    let (code, stdout, stderr) = check_project(PROJECT, |decree| {
        queue_migrations(
            decree,
            &["01-a.md", "02-b.md", "03-c.md"],
            &["01-a.md", "03-c.md"],
        );
    });
    assert_eq!((code, stdout.as_str()), (0, ""));
    assert_eq!(
        stderr,
        format!("warning: migrations/02-b.md: {INSERTED}\n"),
        "{stderr}"
    );
}

#[test]
fn check_json_lists_a_migration_inserted_before_ones_that_ran() {
    let (code, stdout) = check_inserted_migration("json");
    assert_eq!(code, 0, "{stdout}");
    let doc: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["valid"], true);
    assert_eq!(
        doc["warnings"],
        serde_json::json!([{ "file": "migrations/02-b.md", "message": INSERTED }])
    );
}

#[test]
fn check_sarif_lists_a_migration_inserted_before_ones_that_ran() {
    let (code, stdout) = check_inserted_migration("sarif");
    assert_eq!(code, 0, "{stdout}");
    let log: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let results = log["runs"][0]["results"].as_array().unwrap();
    assert_eq!(results.len(), 1, "{stdout}");
    let w = &results[0];
    assert_eq!(w["level"], "warning");
    assert!(w.get("ruleId").is_none(), "{w}");
    assert_eq!(w["message"]["text"], INSERTED);
    assert_eq!(
        w["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
        ".decree/migrations/02-b.md"
    );
}

/// `100-b.md` sorts before `99-a.md` in byte order: the warning says how to name
/// migrations so they keep their order (docs/reference/messages.md, Migrations).
#[test]
fn check_warns_that_100_sorts_before_99_and_says_how_to_keep_the_order() {
    let (code, stdout, stderr) = check_project(PROJECT, |decree| {
        queue_migrations(decree, &["99-a.md", "100-b.md"], &["99-a.md"]);
    });
    assert_eq!((code, stdout.as_str()), (0, ""));
    assert_eq!(
        stderr,
        "warning: migrations/100-b.md: migration 100-b.md sorts before 99-a.md, which already \
         ran; it runs next. To keep the order, pad numbers to the width the project needs \
         (`001-`) or continue with a letter prefix (`A01-`), which sorts after digits\n",
        "{stderr}"
    );
}

/// A letter prefix sorts after any digits: `A01-b.md` after `99-a.md` draws no warning,
/// and `decree process --dry-run` lists it next.
#[test]
fn a_letter_prefix_continues_after_numbered_migrations() {
    let tmp = TempDir::new().unwrap();
    copy_clone(&decree_dir(PROJECT), &tmp.path().join(".decree"));
    queue_migrations(
        &tmp.path().join(".decree"),
        &["99-a.md", "A01-b.md"],
        &["99-a.md"],
    );
    common::decree_at(tmp.path())
        .args(["plugin", "install"])
        .assert()
        .success();
    let check = common::decree_at(tmp.path())
        .env("NO_COLOR", "1")
        .arg("check")
        .output()
        .unwrap();
    let stderr = String::from_utf8(check.stderr).unwrap();
    assert_eq!(check.status.code(), Some(0), "{stderr}");
    assert_eq!(stderr, "", "no inserted-migration warning");
    let dry = common::decree_at(tmp.path())
        .env("NO_COLOR", "1")
        .args(["process", "--dry-run"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(dry.stdout).unwrap();
    assert_eq!(dry.status.code(), Some(0), "{stdout}");
    let next = stdout
        .lines()
        .find(|line| line.contains(".md"))
        .unwrap_or_default();
    assert!(next.contains("A01-b.md"), "{stdout}");
    assert!(!stdout.contains("99-a.md"), "{stdout}");
}

/// A pending migration after every one that ran is the normal case: no warning.
#[test]
fn check_does_not_warn_about_a_migration_after_the_ones_that_ran() {
    let (code, stdout, stderr) = check_project(PROJECT, |decree| {
        queue_migrations(decree, &["01-a.md", "02-b.md"], &["01-a.md"]);
    });
    assert_eq!((code, stdout.as_str(), stderr.as_str()), (0, "", ""));
}

/// A new machine makes the graph stale until `decree graph` rewrites it.
#[test]
fn check_warns_until_the_graph_is_rewritten() {
    let tmp = TempDir::new().unwrap();
    copy_clone(&decree_dir(PROJECT), &tmp.path().join(".decree"));
    common::decree_at(tmp.path())
        .args(["plugin", "install"])
        .assert()
        .success();
    fs::create_dir_all(tmp.path().join(".decree/machines")).unwrap();
    common::write_file(
        &tmp.path().join(".decree/machines/greet/machine.yml"),
        "name: greet\ndescription: Run hello.\ninitial: work\nstates:\n  \
         work:\n    invoke: { machine: hello }\n    transitions: { done: done }\n  \
         done: { final: true }\n  failed: { final: true }\n",
    );
    let run = |cmd: &str| {
        let out = common::decree_at(tmp.path())
            .env("NO_COLOR", "1")
            .arg(cmd)
            .output()
            .unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    };
    // Kept as a plugin, as git ignores machines/ in a project with plugins/.
    common::decree_at(tmp.path())
        .args(["plugin", "export", "greet"])
        .assert()
        .success();
    let (code, stderr) = run("check");
    assert_eq!(code, 0);
    assert!(
        stderr.contains("warning: machines/greet/graph.md: missing"),
        "{stderr}"
    );
    assert_eq!(run("graph").0, 0);
    assert_eq!(run("check"), (0, String::new()));
}

#[test]
fn check_warns_when_the_graph_directory_is_missing() {
    let (code, _, stderr) = check_project(PROJECT, |decree| {
        for m in ["hello", "develop", "deploy"] {
            fs::remove_file(decree.join("machines").join(m).join("graph.md")).unwrap();
        }
        fs::remove_file(decree.join("system.md")).unwrap();
    });
    assert_eq!(code, 0);
    assert_eq!(stderr.lines().count(), 4, "{stderr}");
    assert!(stderr.contains("warning: system.md: missing"), "{stderr}");
}

/// The machine `name` as docs/reference/machines.md shows it, written by hand.
fn docs_machine(name: &str) -> String {
    let reference = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/reference/machines.md"),
    )
    .unwrap();
    reference
        .split("```yaml\n")
        .filter_map(|block| block.split_once("```").map(|(yaml, _)| yaml))
        .find(|yaml| yaml.lines().any(|l| l == format!("name: {name}")))
        .unwrap_or_else(|| panic!("docs/reference/machines.md shows no `{name}`"))
        .to_string()
}

/// The docs/reference/machines.md messages for the shapes this model rejects.
#[test]
fn check_rejects_the_old_shapes_with_the_reference_messages() {
    let cases = [
        (
            FEATURE,
            "feature.yml",
            "        transitions: { done: verify }\n",
            "        transitions: { done: { target: verify, cond: \"visits.implement < 2\" } }\n",
            "machines/feature/machine.yml: work.implement: transition `done`: cond on a transition is not supported: make the decision a state with invoke: { check: ... } (V19)",
        ),
        (
            PROJECT,
            "deploy.yml",
            "        question: Ship this build?\n",
            "",
            "machines/deploy/machine.yml: approval: a `reply` state needs a `question`: what is being decided (V8)",
        ),
        (
            PROJECT,
            "deploy.yml",
            "reject:  { target: rejected, description: Do not ship. }",
            "reject:  rejected",
            "machines/deploy/machine.yml: approval: option `reject` needs a `description`: write it as `reject: { target: rejected, description: ... }` (V8)",
        ),
        (
            FEATURE,
            "ship.yml",
            "invoke: { machine: deploy }",
            "invoke: { machine: ship }",
            "machines/ship/machine.yml: release: machine `ship` invokes itself: ship -> ship; a machine never invokes itself, directly or through others (V20)",
        ),
    ];
    for (project, file, from, to, expected) in cases {
        let (code, stdout, _) = check_project(project, |decree| {
            let path = decree
                .join("machines")
                .join(file.trim_end_matches(".yml"))
                .join("machine.yml");
            // examples/project's machines are plugins; the edit is to the machine as written
            // by hand, as docs/reference/machines.md shows it.
            let text = if project == PROJECT {
                docs_machine(file.trim_end_matches(".yml"))
            } else {
                fs::read_to_string(&path).unwrap()
            };
            assert!(text.contains(from), "{file}: {from}");
            fs::write(&path, text.replacen(from, to, 1)).unwrap();
        });
        assert_eq!(code, 1, "{expected}");
        assert_eq!(stdout, format!("{expected}\n"));
    }
    // A router state inside a compound state, as in the V19 case of `validation_test.rs`.
    let (code, stdout, _) = check_project(PROJECT, |decree| {
        common::write_file(&decree.join("machines/b/machine.yml"), "name: b\ndescription: A router state inside a compound state.\ninitial: work\n\
             states:\n  work:\n    initial: step\n    transitions: { done.state.work: done }\n    \
             states:\n      step:\n        invoke: work\n        router: llm\n        transitions:\n          \
             pass: { target: fin, description: The work is finished. }\n          \
             retry: { target: step, description: Run the work again. }\n      fin: { final: true }\n  \
             done: { final: true }\n  failed: { final: true }\n");
    });
    assert_eq!(code, 1);
    assert_eq!(
        stdout,
        "machines/b/machine.yml: work.step: router on a state is not supported: make the decision a state with invoke: { model: { question: ... } } (V19)\n"
    );
}

/// The escalation conditions in `file_document` broken one at a time (V10).
#[test]
fn check_rejects_bad_escalation_conditions_with_v10() {
    let cases = [
        (
            "file: { type: string, default: \"\" }",
            "file: { type: int, default: 0 }",
            "machines/file_document/machine.yml: by_name: check: `matches` needs string data, but `file` is int (V10)",
        ),
        (
            "confidence: big_model",
            "confidence: read_text",
            "machines/file_document/machine.yml: worth_asking: check: `confidence` names `read_text`, which is not a `model` state (V10)",
        ),
        (
            "at_least: 0.4",
            "at_least: 1.5",
            "machines/file_document/machine.yml: worth_asking: check: `confidence` compares to a number from 0 to 1, not 1.5 (V10)",
        ),
    ];
    for (from, to, expected) in cases {
        let (code, stdout, _) = check_project(ESCALATION, |decree| {
            let path = decree.join("machines/file_document/machine.yml");
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
    let out = common::decree_at(tmp.path()).arg("check").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
}
