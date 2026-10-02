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

#[test]
fn test_init_creates_directory_structure() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir)
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("Decree initialized successfully"));

    let decree = dir.path().join(".decree");

    // Required directories
    assert!(decree.join("routines").is_dir());
    assert!(decree.join("cron").is_dir());
    assert!(decree.join("inbox").is_dir());
    assert!(decree.join("inbox/dead").is_dir());
    assert!(decree.join("outbox").is_dir());
    assert!(decree.join("outbox/dead").is_dir());
    assert!(decree.join("runs").is_dir());
    assert!(decree.join("migrations").is_dir());

    // Required files
    assert!(decree.join("config.yml").is_file());
    assert!(decree.join(".gitignore").is_file());
    assert!(decree.join("router.md").is_file());
    assert!(decree.join("processed.md").is_file());

    // Routine templates
    assert!(decree.join("routines/develop.sh").is_file());
    assert!(decree.join("routines/rust-develop.sh").is_file());
}

#[test]
fn test_init_config_has_required_fields() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir).arg("init").assert().success();

    let config = fs::read_to_string(dir.path().join(".decree/config.yml")).unwrap();

    assert!(config.contains("ai_router:"));
    assert!(config.contains("ai_interactive:"));
    assert!(config.contains("max_attempts: 3"));
    assert!(config.contains("max_depth: 10"));
    assert!(config.contains("max_log_size: 2097152"));
    assert!(config.contains("default_routine: develop"));
    assert!(config.contains("hooks:"));
    assert!(config.contains("beforeAll:"));
    assert!(config.contains("afterAll:"));
    assert!(config.contains("# beforeEach: \"git-baseline\""));
    assert!(config.contains("# afterEach: \"git-stash-changes\""));
}

#[test]
fn test_init_config_has_commented_alternatives() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir).arg("init").assert().success();

    let config = fs::read_to_string(dir.path().join(".decree/config.yml")).unwrap();

    // At least two of the three backends should appear (one uncommented, others commented)
    let ai_lines: Vec<&str> = config.lines().filter(|l| l.contains("ai_router")).collect();
    // Should have one active + at least one commented alternative
    assert!(
        ai_lines.len() >= 2,
        "Expected multiple ai_router entries, got: {ai_lines:?}"
    );
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
    assert!(config.contains("  ai_router: \"opencode run {prompt}\"\n"));
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

#[test]
fn test_init_ai_claude_sets_ai_router() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir)
        .args(["init", "--ai", "claude"])
        .write_stdin("")
        .assert()
        .success();

    let config = fs::read_to_string(dir.path().join(".decree/config.yml")).unwrap();
    assert!(config.contains("  ai_router: \"claude -p {prompt}\"\n"));
    assert!(config.contains("  # ai_router: \"opencode run {prompt}\"\n"));
    // The decree skill is installed for the chosen backend.
    assert!(dir.path().join(".claude/skills").is_dir());
    assert!(!dir.path().join(".claude/settings.json").exists());
}

#[test]
fn test_init_ai_opencode_and_copilot_set_ai_router() {
    for (ai, router) in [
        ("opencode", "opencode run {prompt}"),
        ("copilot", "copilot -p {prompt}"),
    ] {
        let dir = TempDir::new().unwrap();
        decree_cmd(&dir)
            .args(["init", "--ai", ai])
            .assert()
            .success();
        let config = fs::read_to_string(dir.path().join(".decree/config.yml")).unwrap();
        assert!(config.contains(&format!("  ai_router: \"{router}\"\n")));
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
fn test_init_routines_are_executable() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir).arg("init").assert().success();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let develop = dir.path().join(".decree/routines/develop.sh");
        let mode = fs::metadata(&develop).unwrap().permissions().mode();
        assert!(mode & 0o111 != 0, "develop.sh should be executable");
    }
}

#[test]
fn test_init_gitignore_content() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir).arg("init").assert().success();

    let gitignore = fs::read_to_string(dir.path().join(".decree/.gitignore")).unwrap();
    assert!(gitignore.contains("inbox/"));
    assert!(gitignore.contains("outbox/"));
    assert!(gitignore.contains("runs/"));
}

#[test]
fn test_init_routines_use_ai_cmd_placeholder() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir).arg("init").assert().success();

    let develop = fs::read_to_string(dir.path().join(".decree/routines/develop.sh")).unwrap();
    let rust_develop =
        fs::read_to_string(dir.path().join(".decree/routines/rust-develop.sh")).unwrap();

    // {AI_CMD} should have been replaced with a real command name
    assert!(
        !develop.contains("{AI_CMD}"),
        "develop.sh should not contain raw {{AI_CMD}} placeholder"
    );
    assert!(
        !rust_develop.contains("{AI_CMD}"),
        "rust-develop.sh should not contain raw {{AI_CMD}} placeholder"
    );
}

#[test]
fn test_init_routines_have_precheck() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir).arg("init").assert().success();

    let develop = fs::read_to_string(dir.path().join(".decree/routines/develop.sh")).unwrap();
    let rust_develop =
        fs::read_to_string(dir.path().join(".decree/routines/rust-develop.sh")).unwrap();

    assert!(
        develop.contains("DECREE_PRE_CHECK"),
        "develop.sh must have pre-check section"
    );
    assert!(
        rust_develop.contains("DECREE_PRE_CHECK"),
        "rust-develop.sh must have pre-check section"
    );
}

#[test]
fn test_init_precheck_prints_to_stderr() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir).arg("init").assert().success();

    let develop = fs::read_to_string(dir.path().join(".decree/routines/develop.sh")).unwrap();
    let rust_develop =
        fs::read_to_string(dir.path().join(".decree/routines/rust-develop.sh")).unwrap();

    assert!(
        develop.contains(">&2"),
        "develop.sh pre-check failures must print to stderr"
    );
    assert!(
        rust_develop.contains(">&2"),
        "rust-develop.sh pre-check failures must print to stderr"
    );
}

#[test]
fn test_init_router_md_placement_and_content() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir).arg("init").assert().success();

    // router.md lives at .decree/router.md, NOT in prompts/
    assert!(dir.path().join(".decree/router.md").is_file());
    assert!(!dir.path().join(".decree/prompts/router.md").exists());

    let router = fs::read_to_string(dir.path().join(".decree/router.md")).unwrap();
    assert!(router.contains("{routines}"));
    assert!(router.contains("{message}"));
}

#[test]
fn test_init_routines_have_description_headers() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir).arg("init").assert().success();

    let develop = fs::read_to_string(dir.path().join(".decree/routines/develop.sh")).unwrap();
    let rust_develop =
        fs::read_to_string(dir.path().join(".decree/routines/rust-develop.sh")).unwrap();

    // Both must have description comment headers for `decree routine` extraction
    assert!(develop.contains("# Develop\n"));
    assert!(rust_develop.contains("# Rust Develop\n"));
}

#[test]
fn test_init_routines_reference_message_dir() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir).arg("init").assert().success();

    let develop = fs::read_to_string(dir.path().join(".decree/routines/develop.sh")).unwrap();
    let rust_develop =
        fs::read_to_string(dir.path().join(".decree/routines/rust-develop.sh")).unwrap();

    assert!(
        develop.contains("${message_dir}"),
        "develop.sh must reference ${{message_dir}} for prior attempt context"
    );
    assert!(
        rust_develop.contains("${message_dir}"),
        "rust-develop.sh must reference ${{message_dir}} for prior attempt context"
    );
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

    decree_cmd(&dir)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Migrations:"))
        .stdout(predicate::str::contains("Processed: 0 of 0"))
        .stdout(predicate::str::contains("Inbox:"))
        .stdout(predicate::str::contains("Pending: 0 messages"))
        .stdout(predicate::str::contains("Recent Activity"));
}

#[test]
fn test_status_with_migrations() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();

    // Create some migration files
    let migrations = dir.path().join(".decree/migrations");
    fs::write(migrations.join("01-add-auth.md"), "# Add auth").unwrap();
    fs::write(migrations.join("02-add-db.md"), "# Add DB").unwrap();
    fs::write(migrations.join("03-add-api.md"), "# Add API").unwrap();

    // Mark one as processed
    fs::write(dir.path().join(".decree/processed.md"), "01-add-auth.md\n").unwrap();

    decree_cmd(&dir)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Processed: 1 of 3"))
        .stdout(predicate::str::contains("Next: 02-add-db.md"));
}

// --- decree status dead-letter timestamp ---

#[test]
fn test_status_dead_letter_no_timestamp_when_empty() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();

    decree_cmd(&dir)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Dead-lettered: 0 messages"))
        .stdout(predicate::str::contains("oldest").not());
}

#[test]
fn test_status_dead_letter_shows_oldest_timestamp() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();

    let dead_dir = dir.path().join(".decree/inbox/dead");
    fs::write(dead_dir.join("D0001-1200-migration-one-0.md"), "dead msg 1").unwrap();
    fs::write(dead_dir.join("D0001-1201-migration-two-0.md"), "dead msg 2").unwrap();
    fs::write(
        dead_dir.join("D0001-1202-migration-three-0.md"),
        "dead msg 3",
    )
    .unwrap();

    decree_cmd(&dir)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Dead-lettered: 3 messages"))
        .stdout(predicate::str::contains("oldest:"));
}

// --- decree log ---

#[test]
fn test_log_no_runs() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();

    decree_cmd(&dir)
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("No runs found"));
}

#[test]
fn test_log_shows_most_recent_non_tty() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();

    // Create a run directory with a log
    let run_dir = dir.path().join(".decree/runs/D0001-1432-test-0");
    fs::create_dir_all(&run_dir).unwrap();
    fs::write(run_dir.join("routine.log"), "Hello from the log\n").unwrap();

    decree_cmd(&dir)
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello from the log"));
}

#[test]
fn test_log_with_specific_id() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();

    // Create two run directories
    let run1 = dir.path().join(".decree/runs/D0001-1432-alpha-0");
    let run2 = dir.path().join(".decree/runs/D0001-1435-beta-0");
    fs::create_dir_all(&run1).unwrap();
    fs::create_dir_all(&run2).unwrap();
    fs::write(run1.join("routine.log"), "Alpha log\n").unwrap();
    fs::write(run2.join("routine.log"), "Beta log\n").unwrap();

    decree_cmd(&dir)
        .args(["log", "D0001-1435"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Beta log"));
}

#[test]
fn test_log_not_found() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();

    decree_cmd(&dir)
        .args(["log", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("message not found"));
}

#[test]
fn test_log_multiple_attempts() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();

    let run_dir = dir.path().join(".decree/runs/D0001-1432-multi-0");
    fs::create_dir_all(&run_dir).unwrap();
    fs::write(run_dir.join("routine.log"), "Attempt 1\n").unwrap();
    fs::write(run_dir.join("routine-2.log"), "Attempt 2\n").unwrap();

    decree_cmd(&dir)
        .args(["log", "D0001-1432-multi-0"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Attempt 1"))
        .stdout(predicate::str::contains("Attempt 2"))
        .stdout(predicate::str::contains("Attempt 1"))
        .stdout(predicate::str::contains("Attempt 2"));
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

#[test]
fn test_routine_no_args_non_tty_lists_routines() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();

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
    decree_cmd(&dir).arg("init").assert().success();

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
    decree_cmd(&dir).arg("init").assert().success();

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
    decree_cmd(&dir).arg("init").assert().success();

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
    decree_cmd(&dir).arg("init").assert().success();

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
    decree_cmd(&dir).arg("init").assert().success();

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
    decree_cmd(&dir).arg("init").assert().success();

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
    decree_cmd(&dir).arg("init").assert().success();

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
    decree_cmd(&dir).arg("init").assert().success();

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
    decree_cmd(&dir).arg("init").assert().success();

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
    decree_cmd(&dir).arg("init").assert().success();

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
    decree_cmd(&dir).arg("init").assert().success();

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
    decree_cmd(&dir).arg("init").assert().success();

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
    decree_cmd(&dir).arg("init").assert().success();

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
    decree_cmd(&dir).arg("init").assert().success();

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
    assert_eq!(config["default_routine"].as_str(), Some("develop"));
}
