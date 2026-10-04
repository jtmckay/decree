use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

mod common;
use common::write_script;

/// Helper: run decree in a temp directory.
fn decree_cmd(dir: &TempDir) -> Command {
    let mut cmd = cargo_bin_cmd!("decree");
    cmd.current_dir(dir.path());
    // Force non-TTY behavior + no color for predictable output
    cmd.env("NO_COLOR", "1");
    cmd
}

// --- decree init ---

/// After `init`, `.decree/` holds exactly the entries of docs/reference/README.md, File layout.
#[test]
fn test_init_creates_directory_structure() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir)
        .args(["init", "--ai", "claude"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Decree initialized successfully"));

    let decree = dir.path().join(".decree");
    let mut entries: Vec<String> = fs::read_dir(&decree)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    assert_eq!(
        entries,
        [
            ".gitignore",
            "cron",
            "graph",
            "inbox",
            "machines",
            "migrations",
            "processed.md",
            "runs",
            "schema",
            "scripts"
        ]
    );
    for d in [
        "cron",
        "graph",
        "inbox",
        "machines",
        "migrations",
        "runs",
        "scripts",
    ] {
        assert!(decree.join(d).is_dir(), "{d}");
    }
    for d in ["cron", "inbox", "migrations", "runs"] {
        assert_eq!(fs::read_dir(decree.join(d)).unwrap().count(), 0, "{d}");
    }
    assert!(decree.join("graph/router.md").is_file());
    assert!(decree.join("graph/system.md").is_file());
}

#[test]
fn test_init_processed_md_is_empty() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir).arg("init").assert().success();

    let content = fs::read_to_string(dir.path().join(".decree/processed.md")).unwrap();
    assert!(content.is_empty());
}

/// Snapshot of every file under `root` with its contents, for "changes nothing" checks.
fn snapshot(root: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<(std::path::PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.push((path.clone(), Vec::new()));
                walk(&path, out);
            } else {
                out.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

#[test]
fn test_init_stdin_closed_asks_nothing() {
    let dir = TempDir::new().unwrap();

    // Empty PATH: nothing is detected, so the 0.4.2 multi-backend selector would
    // have been the only remaining prompt path besides permissions/overwrite.
    let output = decree_cmd(&dir)
        .arg("init")
        .write_stdin("")
        .env("PATH", dir.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!all.contains('?'), "init asked something: {all}");
    assert!(!all.contains("[y/N]") && !all.contains("[Y/n]"));

    // Without --ai and with nothing on PATH, the backend is opencode.
    assert!(dir
        .path()
        .join(".decree/scripts/router/ask_opencode.sh")
        .is_file());
    // Without --permissions, no permissions file is written.
    assert!(!dir.path().join("opencode.json").exists());
}

#[test]
fn test_init_existing_decree_exits_2_and_changes_nothing() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();
    fs::write(dir.path().join(".decree/processed.md"), "edited\n").unwrap();
    let before = snapshot(dir.path());

    decree_cmd(&dir)
        .args(["init", "--ai", "claude", "--permissions"])
        .write_stdin("")
        .assert()
        .code(2)
        .stderr(predicate::str::contains(".decree/ already exists"));

    assert_eq!(snapshot(dir.path()), before);
}

#[test]
fn test_init_existing_empty_decree_dir_exits_2() {
    let dir = TempDir::new().unwrap();
    fs::create_dir(dir.path().join(".decree")).unwrap();

    decree_cmd(&dir).arg("init").assert().code(2);

    assert_eq!(fs::read_dir(dir.path().join(".decree")).unwrap().count(), 0);
}

/// `decree check` in `dir`: it exits 0 and prints nothing, not even a stale-graph warning.
fn assert_check_passes(dir: &TempDir) {
    let out = decree_cmd(dir).arg("check").output().unwrap();
    assert_eq!(
        (
            out.status.code(),
            out.stdout.as_slice(),
            out.stderr.as_slice()
        ),
        (Some(0), &b""[..], &b""[..]),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn test_init_ai_claude_writes_router_and_check_passes() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir)
        .args(["init", "--ai", "claude"])
        .write_stdin("")
        .assert()
        .success();

    let decree = dir.path().join(".decree");
    assert!(!decree.join("config.yml").exists());
    assert!(decree.join("machines/router.yml").is_file());
    assert!(decree.join("scripts/router/ask_claude.sh").is_file());
    assert!(decree.join("graph/router.md").is_file());
    assert_check_passes(&dir);
    // The decree skill is installed for the chosen backend.
    assert!(dir.path().join(".claude/skills").is_dir());
    assert!(!dir.path().join(".claude/settings.json").exists());
}

#[test]
fn test_init_ai_opencode_and_copilot_write_their_routers() {
    for ai in ["opencode", "copilot"] {
        let dir = TempDir::new().unwrap();
        decree_cmd(&dir)
            .args(["init", "--ai", ai])
            .assert()
            .success();
        let decree = dir.path().join(".decree");
        assert!(!decree.join("config.yml").exists());
        let machine = fs::read_to_string(decree.join("machines/router.yml")).unwrap();
        assert!(machine.contains("name: router\n"), "{machine}");
        assert!(
            machine.contains(&format!("script: {{ name: ask_{ai}, max_attempts: 2 }}\n")),
            "{machine}"
        );
        let scripts: Vec<String> = fs::read_dir(decree.join("scripts/router"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(scripts, [format!("ask_{ai}.sh")]);
        assert_check_passes(&dir);
    }
}

/// Run the `ask_claude` that `decree init --ai claude` writes, with a stub `claude` on
/// `PATH` that prints `stub_reply`: (exit code, `reply.json` if written, stderr).
fn run_ask_claude(stub_reply: &str) -> (i32, Option<String>, String) {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir)
        .args(["init", "--ai", "claude"])
        .assert()
        .success();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    fs::write(bin.join("reply.txt"), stub_reply).unwrap();
    // The stub reads the prompt from stdin, as `claude -p` does, and keeps it.
    let stub = bin.join("claude");
    write_script(
        &stub,
        &format!(
            "#!/usr/bin/env bash\n[ \"$1\" = -p ] || exit 9\ncat > {bin}/prompt.txt\ncat {bin}/reply.txt\n",
            bin = bin.display()
        ),
    );
    let run = dir.path().join(".decree/runs/r1");
    fs::create_dir_all(&run).unwrap();
    let request = serde_json::json!({
        "v": 1, "machine": "feature", "machine_description": "Implement one feature.",
        "state": "triage", "state_description": "",
        "question": "Should we implement again or split the work?",
        "options": [
            {"event": "retry", "description": "Implement again."},
            {"event": "split", "description": "Split the work."},
        ],
        "input": "1 test failed", "message_body": "# Task\n", "history": ["verify: fail"],
    });
    fs::write(run.join("request.json"), request.to_string()).unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let out = std::process::Command::new(dir.path().join(".decree/scripts/router/ask_claude.sh"))
        .current_dir(dir.path())
        .env("PATH", path)
        .env("DECREE_REQUEST", run.join("request.json"))
        .env("DECREE_REPLY", run.join("reply.json"))
        .output()
        .unwrap();
    let prompt = fs::read_to_string(bin.join("prompt.txt")).unwrap();
    assert!(prompt.contains("Question: Should we implement again or split the work?\n"));
    assert!(prompt.contains("- retry: Implement again.\n- split: Split the work."));
    (
        out.status.code().unwrap(),
        fs::read_to_string(run.join("reply.json")).ok(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn test_ask_claude_takes_the_fenced_json_after_prose() {
    let cases = [
        // Prose, then a fenced one-line object.
        "The failure is one off-by-one; fixing it is local.\n\n```json\n{\"event\": \"retry\", \"reason\": \"Local fix.\", \"confidence\": 0.86}\n```\n",
        // A fenced object over several lines, with an earlier object in the prose.
        "Not {\"event\": \"split\"} but:\n```\n{\n  \"event\": \"retry\",\n  \"reason\": \"Local fix.\",\n  \"confidence\": 0.86\n}\n```\nThanks.\n",
    ];
    for stub_reply in cases {
        let (code, reply, stderr) = run_ask_claude(stub_reply);
        assert_eq!(code, 0, "{stderr}");
        assert_eq!(
            reply.as_deref(),
            Some("{\"event\":\"retry\",\"reason\":\"Local fix.\",\"confidence\":0.86}\n"),
            "{stub_reply}"
        );
    }
}

#[test]
fn test_ask_claude_fails_on_a_reply_that_is_not_an_option() {
    for (stub_reply, want) in [
        (
            "```json\n{\"event\": \"merge\"}\n```\n",
            "not an option: merge",
        ),
        ("I would retry.\n", "not an option: "),
    ] {
        let (code, reply, stderr) = run_ask_claude(stub_reply);
        assert_eq!(code, 1, "{stub_reply}");
        assert!(reply.is_none(), "{stub_reply}");
        assert!(stderr.contains(want), "{stderr}");
    }
}

/// The router `init` writes, run by decree, picks the option when the model replies with
/// a bare JSON object: the script's last stdout line must not be read as its event.
#[test]
fn test_default_router_through_decree_takes_a_bare_json_reply() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir)
        .args(["init", "--ai", "claude"])
        .assert()
        .success();
    fs::write(
        dir.path().join(".decree/machines/pick.yml"),
        "name: pick
description: Pick retry or split.
initial: triage
states:
  triage:
    invoke:
      model:
        question: Retry or split?
    transitions:
      retry: { target: retried, description: Implement again. }
      split: { target: split_up, description: Split the work. }
  retried: { final: true }
  split_up: { final: true }
  failed: { final: true }
",
    )
    .unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    write_script(
        &bin.join("claude"),
        "#!/usr/bin/env bash\ncat > /dev/null\necho '{\"event\": \"split\", \"reason\": \"Too big.\", \"confidence\": 0.9}'\n",
    );
    let out = decree_cmd(&dir)
        .args(["emit", "--machine", "pick"])
        .write_stdin("# Task\n")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let id = String::from_utf8(out).unwrap().trim().to_string();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    decree_cmd(&dir)
        .arg("process")
        .env("PATH", path)
        .assert()
        .code(0);
    let message =
        fs::read_to_string(dir.path().join(".decree/runs").join(&id).join("message.md")).unwrap();
    assert!(message.contains("state: split_up\n"), "{message}");
}

#[test]
fn test_init_ai_rejects_unknown_backend() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir)
        .args(["init", "--ai", "gpt"])
        .assert()
        .code(2);

    assert!(!dir.path().join(".decree").exists());
}

#[test]
fn test_init_permissions_writes_claude_settings() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir)
        .args(["init", "--ai", "claude", "--permissions"])
        .write_stdin("")
        .assert()
        .success();

    let settings = fs::read_to_string(dir.path().join(".claude/settings.json")).unwrap();
    assert!(settings.contains("\"Write\"") && settings.contains("\"Edit\""));
}

#[test]
fn test_init_permissions_writes_opencode_json() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir)
        .args(["init", "--ai", "opencode", "--permissions"])
        .assert()
        .success();

    assert!(dir.path().join("opencode.json").is_file());
}

#[test]
fn test_init_permissions_keeps_existing_settings() {
    let dir = TempDir::new().unwrap();
    fs::create_dir(dir.path().join(".claude")).unwrap();
    fs::write(dir.path().join(".claude/settings.json"), "{}\n").unwrap();

    decree_cmd(&dir)
        .args(["init", "--ai", "claude", "--permissions"])
        .assert()
        .success()
        .stdout(predicate::str::contains("already exists"));

    assert_eq!(
        fs::read_to_string(dir.path().join(".claude/settings.json")).unwrap(),
        "{}\n"
    );
}

#[test]
fn test_init_gitignore_content() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir).arg("init").assert().success();

    let gitignore = fs::read_to_string(dir.path().join(".decree/.gitignore")).unwrap();
    assert_eq!(gitignore, "inbox/\nruns/\n");
}

// --- decree (bare) without .decree/ ---

#[test]
fn test_bare_decree_without_project_fails() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not inside a decree project"));
}

// --- decree status ---

#[test]
fn test_status_empty_project() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();

    decree_cmd(&dir).arg("status").assert().success().stdout(
        "Runs: 0\n  active: 0\n  waiting: 0\n  pending: 0\n  interrupted: 0\n  finished: 0\n\
         Queued:\n  inbox/: 0\n  migrations/: 0 pending\n",
    );
}

#[test]
fn test_status_lists_queued_messages_and_pending_migrations() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();

    let decree = dir.path().join(".decree");
    for name in ["01-add-auth.md", "02-add-db.md", "03-add-api.md"] {
        fs::write(decree.join("migrations").join(name), "# Migration\n").unwrap();
    }
    fs::write(decree.join("processed.md"), "01-add-auth.md\n").unwrap();
    fs::write(decree.join("inbox/a.md"), "Do a.\n").unwrap();
    fs::write(decree.join("inbox/.b.md.tmp"), "Being written.\n").unwrap();

    decree_cmd(&dir)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Queued:\n  inbox/: 1\n    a.md\n  migrations/: 2 pending\n    02-add-db.md\n    03-add-api.md\n",
        ));
}

#[test]
fn test_status_unknown_id_says_so_and_exits_0() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();

    decree_cmd(&dir)
        .args(["status", "nope"])
        .assert()
        .success()
        .stderr(predicate::str::contains("no run nope"));
}

#[test]
fn test_status_cron_lists_files_and_next_fire_time() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();
    decree_cmd(&dir)
        .args(["status", "--cron"])
        .assert()
        .success()
        .stdout("No cron files.\n");

    fs::write(
        dir.path().join(".decree/cron/hourly.md"),
        "---\ncron: \"0 * * * *\"\nmachine: hello\n---\nHourly task.\n",
    )
    .unwrap();
    let out = decree_cmd(&dir)
        .args(["status", "--cron"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let out = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "{out}");
    assert!(lines[0].starts_with("CRON FILE"), "{out}");
    assert!(lines[0].contains("MACHINE") && lines[0].ends_with("NEXT RUN"));
    assert!(lines[1].starts_with("hourly.md"), "{out}");
    assert!(lines[1].contains("0 * * * *") && lines[1].contains("hello"));
    assert!(lines[1].contains(":00 (in "), "{out}");
}

#[test]
fn test_log_and_cron_list_are_removed() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();
    decree_cmd(&dir).arg("log").assert().code(2);
    decree_cmd(&dir).args(["cron", "list"]).assert().code(2);
}

// --- decree --version ---

#[test]
fn test_version_flag() {
    cargo_bin_cmd!("decree")
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("decree 0.5.0"));
}

// --- decree --no-color ---

#[test]
fn test_no_color_flag_accepted() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();

    decree_cmd(&dir)
        .args(["--no-color", "status"])
        .assert()
        .success();
}

fn has_ansi(bytes: &[u8]) -> bool {
    bytes.contains(&0x1b)
}

/// Run `decree status` in an initialized project and return stdout + stderr.
fn status_output(configure: impl FnOnce(&mut Command)) -> Vec<u8> {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();

    let mut cmd = cargo_bin_cmd!("decree");
    cmd.current_dir(dir.path()).arg("status");
    configure(&mut cmd);
    let out = cmd.assert().success().get_output().clone();
    [out.stdout, out.stderr].concat()
}

#[test]
fn test_status_no_color_env_has_no_ansi() {
    let out = status_output(|cmd| {
        cmd.env("NO_COLOR", "1").env_remove("CLICOLOR_FORCE");
    });
    assert!(!has_ansi(&out), "{}", String::from_utf8_lossy(&out));
}

#[test]
fn test_status_forced_color_has_ansi() {
    // Control for the test above: `colored` does emit escapes when forced,
    // so the absence of escapes under NO_COLOR is meaningful.
    let out = status_output(|cmd| {
        cmd.env_remove("NO_COLOR").env("CLICOLOR_FORCE", "1");
    });
    assert!(has_ansi(&out), "{}", String::from_utf8_lossy(&out));
}

#[test]
fn test_no_color_flag_overrides_forced_color() {
    let out = status_output(|cmd| {
        cmd.arg("--no-color")
            .env_remove("NO_COLOR")
            .env("CLICOLOR_FORCE", "1");
    });
    assert!(!has_ansi(&out), "{}", String::from_utf8_lossy(&out));
}

// --- removed 0.4 routine commands ---

/// `routine`, `verify` and `routine-sync` are gone (docs/reference/cli.md).
#[test]
fn test_removed_routine_commands_exit_2() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();
    for args in [
        &["routine"][..],
        &["routine", "develop"],
        &["verify"],
        &["routine-sync"],
    ] {
        decree_cmd(&dir)
            .args(args)
            .assert()
            .code(2)
            .stderr(predicate::str::contains("unrecognized subcommand"));
    }
}

// --- exit codes ---

#[test]
fn test_unknown_subcommand_exit_code_2() {
    cargo_bin_cmd!("decree")
        .arg("nonexistent")
        .env("NO_COLOR", "1")
        .assert()
        .code(2);
}

// --- decree skill, written by init ---

#[test]
fn test_init_writes_decree_skill_for_claude_and_copilot() {
    for (ai, skill_dir) in [
        ("claude", ".claude/skills/decree"),
        ("copilot", ".github/skills/decree"),
    ] {
        let dir = TempDir::new().unwrap();
        decree_cmd(&dir)
            .args(["init", "--ai", ai])
            .assert()
            .success()
            .stdout(predicate::str::contains(format!(
                "Decree skill: ./{skill_dir} (5 written, 0 existing kept)"
            )));
        let skill = dir.path().join(skill_dir);
        let text = fs::read_to_string(skill.join("SKILL.md")).unwrap();
        // No 0.4 concept in the skill.
        for term in ["routine", "outbox", "hooks", "router.md"] {
            assert!(!text.to_lowercase().contains(term), "{ai}: {term}");
        }
        for name in ["machines", "messages", "runs", "scripts"] {
            let path = skill.join(format!("reference/{name}.md"));
            assert!(path.is_file(), "{ai}: {name}");
        }
    }
}

/// The skill `init` writes is regular files with the content of `src/templates/skills/decree/`,
/// even though this repository's own `.claude/skills/decree` is a symlink to that directory.
#[test]
fn test_init_writes_skill_as_regular_files_matching_the_template() {
    let template =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/templates/skills/decree");
    let names = [
        "SKILL.md",
        "reference/machines.md",
        "reference/messages.md",
        "reference/runs.md",
        "reference/scripts.md",
    ];
    for (ai, skill_dir) in [
        ("claude", ".claude/skills/decree"),
        ("copilot", ".github/skills/decree"),
    ] {
        let dir = TempDir::new().unwrap();
        decree_cmd(&dir)
            .args(["init", "--ai", ai])
            .assert()
            .success();
        let mut path = dir.path().to_path_buf();
        for part in skill_dir.split('/') {
            path.push(part);
            let kind = fs::symlink_metadata(&path).unwrap().file_type();
            assert!(kind.is_dir(), "{ai}: {} is not a directory", path.display());
        }
        for name in names {
            let written = path.join(name);
            let kind = fs::symlink_metadata(&written).unwrap().file_type();
            assert!(kind.is_file(), "{ai}: {name} is not a regular file");
            assert_eq!(
                fs::read(&written).unwrap(),
                fs::read(template.join(name)).unwrap(),
                "{ai}: {name}"
            );
        }
    }
}

#[test]
fn test_init_opencode_writes_no_skill() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir)
        .args(["init", "--ai", "opencode"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Decree skill").not());
    assert!(!dir.path().join(".claude").exists());
    assert!(!dir.path().join(".github").exists());
}

/// `init` never overwrites a skill file; here `SKILL.md` exists and `.decree/` does not.
#[test]
fn test_init_keeps_existing_skill_files() {
    let dir = TempDir::new().unwrap();
    let skill = dir.path().join(".claude/skills/decree");
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "custom\n").unwrap();
    fs::write(dir.path().join(".claude/settings.json"), "{}").unwrap();

    decree_cmd(&dir)
        .args(["init", "--ai", "claude"])
        .assert()
        .success()
        .stdout(predicate::str::contains("(4 written, 1 existing kept)"));

    assert_eq!(
        fs::read_to_string(skill.join("SKILL.md")).unwrap(),
        "custom\n"
    );
    assert!(skill.join("reference/machines.md").is_file());
    assert_eq!(
        fs::read_to_string(dir.path().join(".claude/settings.json")).unwrap(),
        "{}"
    );
}

/// The `skill` command is gone (docs/reference/cli.md).
#[test]
fn test_skill_command_is_removed() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir)
        .args(["skill", "--scope", "project", "--target", "claude"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unrecognized subcommand 'skill'"));
    assert!(!dir.path().join(".claude").exists());
}
