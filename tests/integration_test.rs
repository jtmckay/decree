use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Helper: run decree in a temp directory.
fn decree_cmd(dir: &TempDir) -> Command {
    let mut cmd = Command::from(cargo_bin_cmd!("decree"));
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
    assert!(decree.join("prompts").is_dir());
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

    // Prompt templates
    assert!(decree.join("prompts/migration.md").is_file());
    assert!(decree.join("prompts/sow.md").is_file());
    assert!(decree.join("prompts/routine.md").is_file());

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
    assert!(config.contains("max_retries: 3"));
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
    let ai_lines: Vec<&str> = config
        .lines()
        .filter(|l| l.contains("ai_router"))
        .collect();
    // Should have one active + at least one commented alternative
    assert!(ai_lines.len() >= 2, "Expected multiple ai_router entries, got: {ai_lines:?}");
}

#[test]
fn test_init_processed_md_is_empty() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir).arg("init").assert().success();

    let content = fs::read_to_string(dir.path().join(".decree/processed.md")).unwrap();
    assert!(content.is_empty());
}

#[test]
fn test_init_rerun_non_tty_overwrites() {
    let dir = TempDir::new().unwrap();

    // First init
    decree_cmd(&dir).arg("init").assert().success();

    // Second init (non-TTY, should proceed with overwrite)
    decree_cmd(&dir)
        .arg("init")
        .assert()
        .success()
        .stderr(predicate::str::contains("already configured"));
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
fn test_init_sow_prompt_content() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir).arg("init").assert().success();

    let sow = fs::read_to_string(dir.path().join(".decree/prompts/sow.md")).unwrap();
    assert!(sow.contains("# Statement of Work Template"));
    assert!(sow.contains("Jobs to Be Done"));
    assert!(sow.contains("Acceptance Criteria"));
}

#[test]
fn test_init_migration_prompt_content() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir).arg("init").assert().success();

    let migration = fs::read_to_string(dir.path().join(".decree/prompts/migration.md")).unwrap();
    assert!(migration.contains("# Migration Template"));
    assert!(migration.contains("{migrations}"));
    assert!(migration.contains("{processed}"));
}

#[test]
fn test_init_routine_prompt_content() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir).arg("init").assert().success();

    let routine = fs::read_to_string(dir.path().join(".decree/prompts/routine.md")).unwrap();
    assert!(routine.contains("# Routine Authoring Guide"));
    assert!(routine.contains("{routines}"));
    assert!(routine.contains("Pre-Check Section"));
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
    fs::write(
        dir.path().join(".decree/processed.md"),
        "01-add-auth.md\n",
    )
    .unwrap();

    decree_cmd(&dir)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Processed: 1 of 3"))
        .stdout(predicate::str::contains("Next: 02-add-db.md"));
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
    Command::from(cargo_bin_cmd!("decree"))
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("decree 0.2.0"));
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
        .stdout(predicate::str::contains("Default routine that delegates work to an AI assistant"));
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
    Command::from(cargo_bin_cmd!("decree"))
        .arg("nonexistent")
        .env("NO_COLOR", "1")
        .assert()
        .code(2);
}

// --- Config deserialization from init output ---

#[test]
fn test_init_config_is_valid_yaml() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();

    let config_path = dir.path().join(".decree/config.yml");
    let config: decree::config::AppConfig =
        decree::config::AppConfig::load(&config_path).unwrap();

    assert_eq!(config.max_retries, 3);
    assert_eq!(config.max_depth, 10);
    assert_eq!(config.max_log_size, 2_097_152);
    assert_eq!(config.default_routine, "develop");
}
