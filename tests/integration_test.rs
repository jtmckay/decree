use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Helper: run decree in a temp directory.
fn decree_cmd(dir: &TempDir) -> Command {
    let mut cmd = cargo_bin_cmd!("decree");
    cmd.current_dir(dir.path());
    // Force non-TTY behavior + no color for predictable output
    cmd.env("NO_COLOR", "1");
    cmd
}

// --- decree init ---

/// After `init`, `.decree/` holds exactly the section 3 entries.
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
            "config.yml",
            "cron",
            "graph",
            "inbox",
            "machines",
            "migrations",
            "processed.md",
            "runs",
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
    assert!(decree.join("graph/claude_router.md").is_file());
    assert!(decree.join("graph/system.md").is_file());
}

#[test]
fn test_init_config_has_required_fields() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir).arg("init").assert().success();

    let config = fs::read_to_string(dir.path().join(".decree/config.yml")).unwrap();

    assert!(config.contains("default_router: "));
    assert!(!config.contains("commands:"));
    assert!(config.contains("max_attempts: 3"));
    assert!(config.contains("max_depth: 10"));
    assert!(config.contains("max_log_size: 2097152"));
    assert!(config.contains("default_machine: develop"));
    for removed in [
        "hooks",
        "routines",
        "shared_routines",
        "default_routine",
        "routine_source",
    ] {
        assert!(!config.contains(&format!("{removed}:")), "{removed}");
    }
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
    let config = fs::read_to_string(dir.path().join(".decree/config.yml")).unwrap();
    assert!(config.starts_with("default_router: opencode_router "));
    assert!(dir
        .path()
        .join(".decree/machines/opencode_router.yml")
        .is_file());
    // Without --permissions, no permissions file is written.
    assert!(!dir.path().join("opencode.json").exists());
}

#[test]
fn test_init_existing_decree_exits_2_and_changes_nothing() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();
    fs::write(dir.path().join(".decree/config.yml"), "edited: true\n").unwrap();
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
fn test_init_ai_claude_writes_claude_router_and_check_passes() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir)
        .args(["init", "--ai", "claude"])
        .write_stdin("")
        .assert()
        .success();

    let decree = dir.path().join(".decree");
    assert!(decree.join("machines/claude_router.yml").is_file());
    assert!(decree.join("scripts/claude_router/ask_claude.sh").is_file());
    assert!(decree.join("graph/claude_router.md").is_file());
    let config = fs::read_to_string(decree.join("config.yml")).unwrap();
    assert!(config.starts_with("default_router: claude_router "));
    assert!(!config.contains("ai_router"));
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
        let config = fs::read_to_string(decree.join("config.yml")).unwrap();
        assert!(config.starts_with(&format!("default_router: {ai}_router ")));
        let machine = fs::read_to_string(decree.join(format!("machines/{ai}_router.yml"))).unwrap();
        assert!(
            machine.contains(&format!("invoke: ask_{ai}\n")),
            "{machine}"
        );
        assert!(decree
            .join(format!("scripts/{ai}_router/ask_{ai}.sh"))
            .is_file());
        assert!(!decree.join("machines/claude_router.yml").exists());
        assert_check_passes(&dir);
    }
}

/// Run the `ask_claude` that `decree init --ai claude` writes, with a stub `claude` on
/// `PATH` that prints `stub_reply`: (exit code, `reply.json` if written, stderr).
fn run_ask_claude(stub_reply: &str) -> (i32, Option<String>, String) {
    use std::os::unix::fs::PermissionsExt;
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
    fs::write(
        &stub,
        format!(
            "#!/usr/bin/env bash\n[ \"$1\" = -p ] || exit 9\ncat > {bin}/prompt.txt\ncat {bin}/reply.txt\n",
            bin = bin.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
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
    let out = std::process::Command::new(
        dir.path()
            .join(".decree/scripts/claude_router/ask_claude.sh"),
    )
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
        .stdout(predicate::str::contains("decree 0.4.2"));
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

// --- decree routine (non-TTY) ---

/// A project with the 0.4 routines `routine` and `verify` read until M5.3 deletes them;
/// `init` no longer writes them.
fn init_routines(dir: &TempDir) {
    decree_cmd(dir).arg("init").assert().success();
    let decree = dir.path().join(".decree");
    let routines = decree.join("routines");
    fs::create_dir_all(&routines).unwrap();
    let templates =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scripts/v0_4_2");
    for name in ["develop.sh", "rust-develop.sh"] {
        fs::copy(templates.join(name), routines.join(name)).unwrap();
    }
    let mut config = fs::read_to_string(decree.join("config.yml")).unwrap();
    config.push_str(
        "hooks:\n  beforeAll: \"\"\n  afterAll: \"\"\n  beforeEach: \"\"\n  afterEach: \"\"\n",
    );
    fs::write(decree.join("config.yml"), config).unwrap();
}

#[test]
fn test_routine_no_args_non_tty_lists_routines() {
    let dir = TempDir::new().unwrap();
    init_routines(&dir);

    decree_cmd(&dir)
        .arg("routine")
        .assert()
        .success()
        .stdout(predicate::str::contains("develop"))
        .stdout(predicate::str::contains("rust-develop"));
}

#[test]
fn test_routine_named_non_tty_shows_detail() {
    let dir = TempDir::new().unwrap();
    init_routines(&dir);

    decree_cmd(&dir)
        .args(["routine", "develop"])
        .assert()
        .success()
        .stdout(predicate::str::contains("develop"))
        .stdout(predicate::str::contains(".decree/routines/develop.sh"));
}

#[test]
fn test_routine_unknown_with_close_match() {
    let dir = TempDir::new().unwrap();
    init_routines(&dir);

    decree_cmd(&dir)
        .args(["routine", "devlop"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown routine 'devlop'"))
        .stderr(predicate::str::contains("Did you mean 'develop'?"));
}

#[test]
fn test_routine_unknown_no_close_match() {
    let dir = TempDir::new().unwrap();
    init_routines(&dir);

    decree_cmd(&dir)
        .args(["routine", "xyznonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown routine 'xyznonexistent'"))
        .stderr(predicate::str::contains("Available routines:"));
}

#[test]
fn test_routine_no_routines() {
    let dir = TempDir::new().unwrap();
    init_routines(&dir);

    // Remove all routine files
    let routines_dir = dir.path().join(".decree/routines");
    for entry in fs::read_dir(&routines_dir).unwrap() {
        let entry = entry.unwrap();
        fs::remove_file(entry.path()).unwrap();
    }

    decree_cmd(&dir)
        .arg("routine")
        .assert()
        .success()
        .stdout(predicate::str::contains("No routines found"));
}

#[test]
fn test_routine_detail_shows_description() {
    let dir = TempDir::new().unwrap();
    init_routines(&dir);

    decree_cmd(&dir)
        .args(["routine", "develop"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Default routine that delegates work to an AI assistant",
        ));
}

#[test]
fn test_routine_nested_directory() {
    let dir = TempDir::new().unwrap();
    init_routines(&dir);

    // Create a nested routine
    let nested_dir = dir.path().join(".decree/routines/deploy");
    fs::create_dir_all(&nested_dir).unwrap();
    fs::write(
        nested_dir.join("staging.sh"),
        "#!/usr/bin/env bash\n# Deploy Staging\n#\n# Deploy to staging environment.\nset -euo pipefail\n\nif [ \"${DECREE_PRE_CHECK:-}\" = \"true\" ]; then\n    exit 0\nfi\n\necho \"deploying\"\n",
    )
    .unwrap();

    decree_cmd(&dir)
        .arg("routine")
        .assert()
        .success()
        .stdout(predicate::str::contains("deploy/staging"));
}

#[test]
fn test_routine_custom_params_shown_in_detail() {
    let dir = TempDir::new().unwrap();
    init_routines(&dir);

    // Create a routine with custom params
    fs::write(
        dir.path().join(".decree/routines/transcribe.sh"),
        "#!/usr/bin/env bash\n# Transcribe\n#\n# Transcribes audio using OpenAI Whisper.\nset -euo pipefail\n\nmessage_file=\"${message_file:-}\"\n\nif [ \"${DECREE_PRE_CHECK:-}\" = \"true\" ]; then\n    command -v whisper >/dev/null 2>&1 || { echo \"whisper not found\" >&2; exit 1; }\n    exit 0\nfi\n\noutput_file=\"${output_file:-}\"\nmodel=\"${model:-large}\"\n\necho \"transcribing\"\n",
    )
    .unwrap();

    decree_cmd(&dir)
        .args(["routine", "transcribe"])
        .assert()
        .success()
        .stdout(predicate::str::contains("output_file"))
        .stdout(predicate::str::contains("model"))
        .stdout(predicate::str::contains("[default: \"large\"]"));
}

// --- decree verify ---

#[test]
fn test_verify_all_pass() {
    let dir = TempDir::new().unwrap();
    init_routines(&dir);

    // Create a simple routine that always passes pre-check
    fs::write(
        dir.path().join(".decree/routines/simple.sh"),
        "#!/usr/bin/env bash\n# Simple\n#\n# A simple routine.\n\nif [ \"${DECREE_PRE_CHECK:-}\" = \"true\" ]; then\n    exit 0\nfi\n\necho done\n",
    )
    .unwrap();

    // Remove routines that require AI commands (which won't exist in test env)
    fs::remove_file(dir.path().join(".decree/routines/develop.sh")).unwrap();
    fs::remove_file(dir.path().join(".decree/routines/rust-develop.sh")).unwrap();

    decree_cmd(&dir)
        .arg("verify")
        .assert()
        .success()
        .stdout(predicate::str::contains("simple"))
        .stdout(predicate::str::contains("PASS"))
        .stdout(predicate::str::contains("1 of 1 routines ready"));
}

#[test]
fn test_verify_some_fail() {
    let dir = TempDir::new().unwrap();
    init_routines(&dir);

    // Create a passing routine
    fs::write(
        dir.path().join(".decree/routines/good.sh"),
        "#!/usr/bin/env bash\n# Good\n#\n# Always passes.\n\nif [ \"${DECREE_PRE_CHECK:-}\" = \"true\" ]; then\n    exit 0\nfi\n\necho done\n",
    )
    .unwrap();

    // Create a failing routine
    fs::write(
        dir.path().join(".decree/routines/bad.sh"),
        "#!/usr/bin/env bash\n# Bad\n#\n# Always fails pre-check.\n\nif [ \"${DECREE_PRE_CHECK:-}\" = \"true\" ]; then\n    echo \"missing-tool not found\" >&2; exit 1\nfi\n\necho done\n",
    )
    .unwrap();

    // Remove default routines
    fs::remove_file(dir.path().join(".decree/routines/develop.sh")).unwrap();
    fs::remove_file(dir.path().join(".decree/routines/rust-develop.sh")).unwrap();

    decree_cmd(&dir)
        .arg("verify")
        .assert()
        .code(3)
        .stdout(predicate::str::contains("good"))
        .stdout(predicate::str::contains("PASS"))
        .stdout(predicate::str::contains("bad"))
        .stdout(predicate::str::contains("FAIL"))
        .stdout(predicate::str::contains("missing-tool not found"))
        .stdout(predicate::str::contains("1 of 2 routines ready"));
}

#[test]
fn test_verify_no_routines() {
    let dir = TempDir::new().unwrap();
    init_routines(&dir);

    // Remove all routines
    let routines_dir = dir.path().join(".decree/routines");
    for entry in fs::read_dir(&routines_dir).unwrap() {
        let entry = entry.unwrap();
        fs::remove_file(entry.path()).unwrap();
    }

    decree_cmd(&dir)
        .arg("verify")
        .assert()
        .success()
        .stdout(predicate::str::contains("No routines found"));
}

#[test]
fn test_verify_shows_fail_reason() {
    let dir = TempDir::new().unwrap();
    init_routines(&dir);

    fs::write(
        dir.path().join(".decree/routines/checker.sh"),
        "#!/usr/bin/env bash\n# Checker\n#\n# Checks deps.\n\nif [ \"${DECREE_PRE_CHECK:-}\" = \"true\" ]; then\n    echo \"kubectl not found\" >&2; exit 1\nfi\n\necho done\n",
    )
    .unwrap();

    // Remove defaults
    fs::remove_file(dir.path().join(".decree/routines/develop.sh")).unwrap();
    fs::remove_file(dir.path().join(".decree/routines/rust-develop.sh")).unwrap();

    decree_cmd(&dir)
        .arg("verify")
        .assert()
        .code(3)
        .stdout(predicate::str::contains("FAIL: kubectl not found"));
}

// --- decree verify with hooks ---

#[test]
fn test_verify_hooks_pass() {
    let dir = TempDir::new().unwrap();
    init_routines(&dir);

    // Remove default routines (they require AI tools)
    fs::remove_file(dir.path().join(".decree/routines/develop.sh")).unwrap();
    fs::remove_file(dir.path().join(".decree/routines/rust-develop.sh")).unwrap();

    // Create a simple routine and a hook routine
    fs::write(
        dir.path().join(".decree/routines/simple.sh"),
        "#!/usr/bin/env bash\n# Simple\n#\n# A simple routine.\n\nif [ \"${DECREE_PRE_CHECK:-}\" = \"true\" ]; then\n    exit 0\nfi\n\necho done\n",
    )
    .unwrap();

    fs::write(
        dir.path().join(".decree/routines/pre-flight.sh"),
        "#!/usr/bin/env bash\n# Pre Flight\n#\n# Hook routine.\n\nif [ \"${DECREE_PRE_CHECK:-}\" = \"true\" ]; then\n    exit 0\nfi\n\necho hook\n",
    )
    .unwrap();

    // Configure the hook in config
    let config = fs::read_to_string(dir.path().join(".decree/config.yml")).unwrap();
    let config = config.replace("beforeEach: \"\"", "beforeEach: \"pre-flight\"");
    fs::write(dir.path().join(".decree/config.yml"), config).unwrap();

    decree_cmd(&dir)
        .arg("verify")
        .assert()
        .success()
        .stdout(predicate::str::contains("Hook pre-checks:"))
        .stdout(predicate::str::contains("pre-flight (beforeEach)"))
        .stdout(predicate::str::contains("PASS"));
}

#[test]
fn test_verify_hooks_missing_routine() {
    let dir = TempDir::new().unwrap();
    init_routines(&dir);

    // Remove default routines
    fs::remove_file(dir.path().join(".decree/routines/develop.sh")).unwrap();
    fs::remove_file(dir.path().join(".decree/routines/rust-develop.sh")).unwrap();

    // Create a simple passing routine
    fs::write(
        dir.path().join(".decree/routines/simple.sh"),
        "#!/usr/bin/env bash\n# Simple\n#\n# A simple routine.\n\nif [ \"${DECREE_PRE_CHECK:-}\" = \"true\" ]; then\n    exit 0\nfi\n\necho done\n",
    )
    .unwrap();

    // Configure a hook that references a non-existent routine
    let config = fs::read_to_string(dir.path().join(".decree/config.yml")).unwrap();
    let config = config.replace("beforeAll: \"\"", "beforeAll: \"nonexistent-hook\"");
    fs::write(dir.path().join(".decree/config.yml"), config).unwrap();

    decree_cmd(&dir)
        .arg("verify")
        .assert()
        .code(3)
        .stdout(predicate::str::contains("Hook pre-checks:"))
        .stdout(predicate::str::contains("nonexistent-hook (beforeAll)"))
        .stdout(predicate::str::contains("routine not found"));
}

#[test]
fn test_verify_no_hooks_configured_no_hook_section() {
    let dir = TempDir::new().unwrap();
    init_routines(&dir);

    // Remove default routines
    fs::remove_file(dir.path().join(".decree/routines/develop.sh")).unwrap();
    fs::remove_file(dir.path().join(".decree/routines/rust-develop.sh")).unwrap();

    fs::write(
        dir.path().join(".decree/routines/simple.sh"),
        "#!/usr/bin/env bash\n# Simple\n#\n# A simple routine.\n\nif [ \"${DECREE_PRE_CHECK:-}\" = \"true\" ]; then\n    exit 0\nfi\n\necho done\n",
    )
    .unwrap();

    // Default config has empty hooks — "Hook pre-checks:" should NOT appear
    decree_cmd(&dir)
        .arg("verify")
        .assert()
        .success()
        .stdout(predicate::str::contains("1 of 1 routines ready"))
        .stdout(predicate::str::contains("Hook pre-checks:").not());
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

// --- decree skill ---

#[test]
fn test_skill_claude_project_creates_file() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir)
        .args([
            "skill", "--scope", "project", "--target", "claude", "--skill", "decree",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed"))
        .stdout(predicate::str::contains(".claude/skills/decree/SKILL.md"));

    assert!(dir.path().join(".claude/skills/decree/SKILL.md").is_file());
}

#[test]
fn test_skill_copilot_project_creates_file() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir)
        .args([
            "skill", "--scope", "project", "--target", "copilot", "--skill", "decree",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed"))
        .stdout(predicate::str::contains(".github/skills/decree/SKILL.md"));

    assert!(dir.path().join(".github/skills/decree/SKILL.md").is_file());
}

#[test]
fn test_skill_user_copilot_unsupported() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir)
        .args([
            "skill", "--scope", "user", "--target", "copilot", "--skill", "decree",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not supported"));
}

#[test]
fn test_skill_claude_user_scope() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir)
        .args([
            "skill", "--scope", "user", "--target", "claude", "--skill", "decree",
        ])
        .env("HOME", dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed"));

    assert!(dir.path().join(".claude/skills/decree/SKILL.md").is_file());
}

#[test]
fn test_skill_already_up_to_date_claude() {
    let dir = TempDir::new().unwrap();

    // First install
    decree_cmd(&dir)
        .args([
            "skill", "--scope", "project", "--target", "claude", "--skill", "decree",
        ])
        .assert()
        .success();

    // Second install — same content
    decree_cmd(&dir)
        .args([
            "skill", "--scope", "project", "--target", "claude", "--skill", "decree",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Already up to date"));

    // File should still exist and be unchanged
    assert!(dir.path().join(".claude/skills/decree/SKILL.md").is_file());
}

#[test]
fn test_skill_already_up_to_date_copilot() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir)
        .args([
            "skill", "--scope", "project", "--target", "copilot", "--skill", "decree",
        ])
        .assert()
        .success();

    decree_cmd(&dir)
        .args([
            "skill", "--scope", "project", "--target", "copilot", "--skill", "decree",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Already up to date"));
}

#[test]
fn test_skill_conflict_no_force_exits_nonzero() {
    let dir = TempDir::new().unwrap();

    // Write a modified file
    fs::create_dir_all(dir.path().join(".claude/skills/decree")).unwrap();
    fs::write(
        dir.path().join(".claude/skills/decree/SKILL.md"),
        "custom content that differs from bundled template\n",
    )
    .unwrap();

    decree_cmd(&dir)
        .args([
            "skill", "--scope", "project", "--target", "claude", "--skill", "decree",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("conflict"))
        .stderr(predicate::str::contains("already exists"));

    // File must NOT have been overwritten
    let content = fs::read_to_string(dir.path().join(".claude/skills/decree/SKILL.md")).unwrap();
    assert_eq!(
        content,
        "custom content that differs from bundled template\n"
    );
}

#[test]
fn test_skill_conflict_with_force_overwrites() {
    let dir = TempDir::new().unwrap();

    fs::create_dir_all(dir.path().join(".claude/skills/decree")).unwrap();
    fs::write(
        dir.path().join(".claude/skills/decree/SKILL.md"),
        "custom content\n",
    )
    .unwrap();

    decree_cmd(&dir)
        .args([
            "skill", "--scope", "project", "--target", "claude", "--skill", "decree", "--force",
        ])
        .assert()
        .success();

    let content = fs::read_to_string(dir.path().join(".claude/skills/decree/SKILL.md")).unwrap();
    assert_ne!(content, "custom content\n");
    assert!(content.contains("Decree"));
}

#[test]
fn test_skill_claude_content_has_required_sections() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir)
        .args([
            "skill", "--scope", "project", "--target", "claude", "--skill", "decree",
        ])
        .assert()
        .success();

    let skill_dir = dir.path().join(".claude/skills/decree");
    let content = fs::read_to_string(skill_dir.join("SKILL.md")).unwrap();
    let ref_migrations = fs::read_to_string(skill_dir.join("reference/migrations.md")).unwrap();

    assert!(content.contains("mmutab"), "must cover immutability");
    assert!(content.contains("Given"), "must cover Given/When/Then");
    assert!(content.contains("When"), "must cover Given/When/Then");
    assert!(content.contains("Then"), "must cover Given/When/Then");
    assert!(
        content.to_lowercase().contains("day-sized")
            || ref_migrations.to_lowercase().contains("day-sized"),
        "must mention day-sized"
    );
    assert!(
        content.contains("smallest feasible") || ref_migrations.contains("smallest feasible"),
        "must mention smallest feasible chunks"
    );
    assert!(
        content.contains(".decree/migrations") || ref_migrations.contains(".decree/migrations"),
        "must specify migration directory"
    );
}

#[test]
fn test_skill_copilot_content_has_required_sections() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir)
        .args([
            "skill", "--scope", "project", "--target", "copilot", "--skill", "decree",
        ])
        .assert()
        .success();

    let skill_dir = dir.path().join(".github/skills/decree");
    let content = fs::read_to_string(skill_dir.join("SKILL.md")).unwrap();
    let ref_migrations = fs::read_to_string(skill_dir.join("reference/migrations.md")).unwrap();

    assert!(content.contains("mmutab"), "must cover immutability");
    assert!(
        content.to_lowercase().contains("migration")
            || ref_migrations.to_lowercase().contains("migration"),
        "must describe the migration contract"
    );
    assert!(
        content.contains("smallest feasible") || ref_migrations.contains("smallest feasible"),
        "must mention smallest feasible chunks"
    );
    assert!(
        content.to_lowercase().contains("bypass")
            || ref_migrations.to_lowercase().contains("bypass"),
        "must warn against bypassing the workflow"
    );
}

#[test]
fn test_skill_installed_content_matches_bundled_template_claude() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir)
        .args([
            "skill", "--scope", "project", "--target", "claude", "--skill", "decree",
        ])
        .assert()
        .success();

    let installed = fs::read_to_string(dir.path().join(".claude/skills/decree/SKILL.md")).unwrap();

    // The installed file should contain the same content as what the command embeds.
    // We verify this by checking a stable unique phrase from the bundled template.
    assert!(installed.contains("Decree is an AI orchestrator"));
}

#[test]
fn test_skill_installed_content_matches_bundled_template_copilot() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir)
        .args([
            "skill", "--scope", "project", "--target", "copilot", "--skill", "decree",
        ])
        .assert()
        .success();

    let installed = fs::read_to_string(dir.path().join(".github/skills/decree/SKILL.md")).unwrap();

    assert!(installed.contains("Decree is an AI orchestrator"));
}

#[test]
fn test_skill_no_prompts_when_flags_provided() {
    let dir = TempDir::new().unwrap();

    // In non-TTY mode (tests always are), providing flags must not require any input.
    // The test succeeds if the process completes without hanging.
    decree_cmd(&dir)
        .args([
            "skill", "--scope", "project", "--target", "claude", "--skill", "decree",
        ])
        .assert()
        .success();
}

#[test]
fn test_skill_preserves_unrelated_files_in_claude_dir() {
    let dir = TempDir::new().unwrap();

    // Create an unrelated file in .claude/
    fs::create_dir_all(dir.path().join(".claude")).unwrap();
    fs::write(dir.path().join(".claude/settings.json"), "{}").unwrap();

    decree_cmd(&dir)
        .args([
            "skill", "--scope", "project", "--target", "claude", "--skill", "decree",
        ])
        .assert()
        .success();

    // Unrelated file must still exist
    assert!(dir.path().join(".claude/settings.json").is_file());
    assert_eq!(
        fs::read_to_string(dir.path().join(".claude/settings.json")).unwrap(),
        "{}"
    );
}

// --- decree skill --all and --skill flags ---

#[test]
fn test_skill_all_installs_two_claude_skills() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir)
        .args(["skill", "--scope", "project", "--target", "claude", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("6 skill file(s) installed"));

    assert!(dir.path().join(".claude/skills/decree/SKILL.md").is_file());
    assert!(dir.path().join(".claude/skills/sow/SKILL.md").is_file());
}

#[test]
fn test_skill_skill_flag_installs_specific_skill() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir)
        .args([
            "skill", "--scope", "project", "--target", "claude", "--skill", "sow",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed"))
        .stdout(predicate::str::contains(".claude/skills/sow/SKILL.md"));

    assert!(dir.path().join(".claude/skills/sow/SKILL.md").is_file());
    assert!(!dir.path().join(".claude/skills/decree/SKILL.md").exists());
}

#[test]
fn test_skill_non_tty_without_skill_flag_errors() {
    let dir = TempDir::new().unwrap();

    // Tests run in non-TTY mode; without --skill or --all this should fail
    decree_cmd(&dir)
        .args(["skill", "--scope", "project", "--target", "claude"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("non-TTY"));
}

// --- Config deserialization from init output ---

#[test]
fn test_init_config_is_valid_yaml() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();

    // The typed load is a unit test in `commands::init`; the crate exposes no internals.
    let contents = fs::read_to_string(dir.path().join(".decree/config.yml")).unwrap();
    let config: serde_norway::Value = serde_norway::from_str(&contents).unwrap();

    assert_eq!(config["max_attempts"].as_u64(), Some(3));
    assert_eq!(config["max_depth"].as_u64(), Some(10));
    assert_eq!(config["max_log_size"].as_u64(), Some(2_097_152));
    assert_eq!(config["default_machine"].as_str(), Some("develop"));
}
