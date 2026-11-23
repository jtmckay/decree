use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

mod common;
use common::write_script;

/// Helper: run decree in a temp directory.
fn decree_cmd(dir: &TempDir) -> Command {
    let mut cmd = common::decree_at(dir.path());
    // Force non-TTY behavior + no color for predictable output
    cmd.env("NO_COLOR", "1");
    cmd
}

// --- decree init ---

/// After `init`, `.decree/` holds exactly the entries of docs/reference/README.md, File layout:
/// the default plugins `ai`, `order`, `prune` and `skill` (D86), installed into `machines/`, their graphs and
/// install records, and no `scripts/`.
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
            ".env.example",
            ".gitignore",
            "cache",
            "inbox",
            "machines",
            "plugins",
            "runs",
            "schema",
            "system.md",
        ]
    );
    for d in ["inbox", "runs"] {
        assert_eq!(fs::read_dir(decree.join(d)).unwrap().count(), 0, "{d}");
    }
    let mut plugins: Vec<String> = fs::read_dir(decree.join("plugins"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    plugins.sort();
    assert_eq!(
        plugins,
        [
            "decree_ai.md",
            "decree_order.md",
            "decree_prune.md",
            "decree_skill.md"
        ]
    );
    for name in ["decree_order", "decree_prune", "decree_skill"] {
        let dir = decree.join("machines").join(name);
        assert!(dir.join("machine.yml").is_file(), "{name}");
        assert!(dir.join("graph.md").is_file(), "{name}");
    }
    assert!(!decree.join("scripts").exists());
    assert!(decree.join("system.md").is_file());
    assert!(!decree.join("lib").exists());
}

/// The official plugins `init` lists: every one, by name.
const OFFICIAL: [&str; 18] = [
    "decree_ai",
    "decree_comfy",
    "decree_develop",
    "decree_develop_by_size",
    "decree_develop_node",
    "decree_develop_react",
    "decree_develop_rust",
    "decree_gliner_router",
    "decree_infra",
    "decree_lease",
    "decree_llm_router",
    "decree_newsletter",
    "decree_order",
    "decree_prune",
    "decree_qa",
    "decree_registry",
    "decree_router",
    "decree_skill",
];

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

/// AC (A10): `init` in an empty directory exits 0 without reading stdin; `plugins/`,
/// `machines/` and `scripts/` hold no plugin files; it lists every official plugin and the
/// `decree install` line; and `decree check` passes.
#[test]
fn test_init_stdin_closed_asks_nothing() {
    let dir = TempDir::new().unwrap();

    // Empty PATH: nothing is detected, and init asks nothing.
    let output = decree_cmd(&dir)
        .arg("init")
        .write_stdin("")
        .env("PATH", dir.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let all = format!("{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(!all.contains('?'), "init asked something: {all}");
    assert!(!all.contains("[y/N]") && !all.contains("[Y/n]"));

    let decree = dir.path().join(".decree");
    assert_eq!(fs::read_dir(decree.join("plugins")).unwrap().count(), 4);
    assert!(!decree.join("scripts").exists());
    // Every official plugin init did not add, one line each with its description, then the
    // line that adds one.
    let listed: Vec<&str> = stdout
        .lines()
        .skip_while(|l| !l.starts_with("Official plugins"))
        .skip(1)
        .take_while(|l| l.starts_with("  "))
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    let others: Vec<&str> = OFFICIAL
        .into_iter()
        .filter(|n| !["decree_ai", "decree_order", "decree_prune", "decree_skill"].contains(n))
        .collect();
    assert_eq!(listed, others, "{stdout}");
    assert!(
        stdout
            .lines()
            .last()
            .unwrap()
            .contains("`decree install decree_develop`"),
        "{stdout}"
    );
    // Without --ai and with nothing on PATH, AI_CLI is left for the plugins to detect.
    let env = fs::read_to_string(decree.join(".env.example")).unwrap();
    assert!(env.contains("\n# AI_CLI=claude\n"), "{env}");
    // Without --permissions, no permissions file is written.
    assert!(!dir.path().join("opencode.json").exists());
    assert_check_passes(&dir);
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

/// `--ai` chooses the skill and `AI_CLI` in `.env.example`; no other file names the backend.
#[test]
fn test_init_ai_claude_writes_skill_and_ai_cli_and_check_passes() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir)
        .args(["init", "--ai", "claude"])
        .write_stdin("")
        .assert()
        .success();

    let decree = dir.path().join(".decree");
    let env = fs::read_to_string(decree.join(".env.example")).unwrap();
    assert!(env.ends_with("\nAI_CLI=claude\n"), "{env}");
    assert_check_passes(&dir);
    // The decree skill is installed for the chosen backend.
    assert!(dir.path().join(".claude/skills").is_dir());
    assert!(!dir.path().join(".claude/settings.json").exists());
}

#[test]
fn test_init_ai_opencode_and_copilot_write_their_ai_cli() {
    for ai in ["opencode", "copilot"] {
        let dir = TempDir::new().unwrap();
        decree_cmd(&dir)
            .args(["init", "--ai", ai])
            .assert()
            .success();
        let decree = dir.path().join(".decree");
        let env = fs::read_to_string(decree.join(".env.example")).unwrap();
        assert!(env.ends_with(&format!("\nAI_CLI={ai}\n")), "{env}");
        assert_check_passes(&dir);
    }
}

/// Run the `router` plugin's `ask.sh`, with `AI_CLI=claude` and a stub `claude` on `PATH`
/// that prints `stub_reply`: (exit code, `reply.json` if written, stderr).
fn run_ask_claude(stub_reply: &str) -> (i32, Option<String>, String) {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir)
        .args(["init", "--ai", "claude"])
        .assert()
        .success();
    decree_cmd(&dir)
        .args(["install", "decree_router"])
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
    let out = std::process::Command::new(
        dir.path()
            .join(".decree/machines/decree_router/scripts/ask.sh"),
    )
    .current_dir(dir.path())
    .env("PATH", path)
    .env("AI_CLI", "claude")
    .env_remove("DECREE_DATA_AI_CLI")
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

/// The `router` plugin, run by decree, picks the option when the model replies with a bare
/// JSON object: the script's last stdout line must not be read as its event.
#[test]
fn test_default_router_through_decree_takes_a_bare_json_reply() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir)
        .args(["init", "--ai", "claude"])
        .assert()
        .success();
    decree_cmd(&dir)
        .args(["install", "decree_router"])
        .assert()
        .success();
    common::write_file(
        &dir.path().join(".decree/machines/pick/machine.yml"),
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
    );
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
        .args(["daemon", "--once"])
        .env("PATH", path)
        .env("AI_CLI", "claude")
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

    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join("opencode.json")).unwrap())
            .unwrap();
    assert_eq!(json["permission"]["external_directory"], "deny");
    assert_eq!(json["share"], "manual");
    assert!(json.get("autoshare").is_none(), "deprecated: {json}");
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

/// AC (A10, D86): the `.gitignore` `init` writes keeps installed machine folders out of git,
/// but each machine's `order/`, `graph.md` and `*.git.*` state files.
#[test]
fn test_init_gitignore_content() {
    let dir = TempDir::new().unwrap();

    decree_cmd(&dir).arg("init").assert().success();

    let gitignore = fs::read_to_string(dir.path().join(".decree/.gitignore")).unwrap();
    for line in [
        "inbox/",
        "runs/",
        "schema/",
        ".env*",
        "!.env.example",
        "attention/",
        "process.log",
        "cache/",
        "ai-use.md",
        "machines/**",
        "!machines/**/",
        "!machines/*/order/**",
        "!machines/*/graph.md",
        "!machines/**/*.git.*",
    ] {
        assert!(gitignore.lines().any(|l| l == line), "{line}:\n{gitignore}");
    }
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
        .stdout("Runs: 0\nQueued:\n  inbox/: 0\n");
}

#[test]
fn test_status_lists_queued_messages() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();

    let decree = dir.path().join(".decree");
    fs::write(decree.join("inbox/a.md"), "Do a.\n").unwrap();
    fs::write(decree.join("inbox/.b.md.tmp"), "Being written.\n").unwrap();

    decree_cmd(&dir)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Queued:\n  inbox/: 1\n    a.md\n"));
}

#[test]
fn test_status_unknown_id_says_so_and_exits_1() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();

    decree_cmd(&dir)
        .args(["status", "nope"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("no run nope"));
}

#[test]
fn test_status_cron_lists_files_and_next_fire_time() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir).arg("init").assert().success();
    // The default plugins' cron files: `order` at daemon start, `prune` weekly.
    let out = decree_cmd(&dir)
        .args(["status", "--cron"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let out = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 4, "{out}");
    assert!(
        lines[1].starts_with("decree_order/boot.md") && lines[1].ends_with("daemon start"),
        "{out}"
    );
    assert!(
        lines[2].starts_with("decree_prune/weekly.md") && lines[2].contains("0 3 * * Sun"),
        "{out}"
    );
    assert!(
        lines[3].starts_with("decree_skill/boot.md") && lines[3].ends_with("daemon start"),
        "{out}"
    );
    common::drop_default_plugins(dir.path());
    decree_cmd(&dir)
        .args(["status", "--cron"])
        .assert()
        .success()
        .stdout("No cron files.\n");

    fs::create_dir_all(dir.path().join(".decree/machines/hello/cron")).unwrap();
    fs::write(
        dir.path().join(".decree/machines/hello/cron/hourly.md"),
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
    assert!(lines[1].starts_with("hello/hourly.md"), "{out}");
    assert!(lines[1].contains("0 * * * *") && lines[1].contains("hello"));
    assert!(lines[1].contains(":00 (in "), "{out}");
}

// --- decree --version ---

#[test]
fn test_version_flag() {
    cargo_bin_cmd!("decree")
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(concat!(
            "decree ",
            env!("CARGO_PKG_VERSION")
        )));
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

    let mut cmd = common::decree_at(dir.path());
    cmd.arg("status");
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
        // No removed concept in the skill.
        for term in ["outbox", "hooks", "router.md"] {
            assert!(!text.to_lowercase().contains(term), "{ai}: {term}");
        }
        for name in ["machines", "messages", "runs", "scripts"] {
            let path = skill.join(format!("reference/{name}.md"));
            assert!(path.is_file(), "{ai}: {name}");
        }
    }
}

/// The skill `init` writes is regular files with the content of the `skill` plugin's
/// `scripts/skill/`, even though this repository's own `.claude/skills/decree` is a symlink.
#[test]
fn test_init_writes_skill_as_regular_files_matching_the_template() {
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
                fs::read_to_string(&written).unwrap(),
                common::plugin_script("plugins/decree_skill.md", &format!("skill/{name}")),
                "{ai}: {name}"
            );
        }
    }
}

/// The skill `init` writes says to describe the outcome, not the steps, beside "one concern per
/// migration", and its worked example message follows the body shape.
#[test]
fn test_init_skill_says_describe_the_outcome_not_the_steps() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir)
        .args(["init", "--ai", "claude"])
        .assert()
        .success();
    let text = fs::read_to_string(dir.path().join(".claude/skills/decree/SKILL.md")).unwrap();
    for want in [
        "**One concern per message**",
        "**Describe the outcome, not the steps.**",
        "**Outcome** (belongs in a message): behaviour a user or another program can observe",
        "**Steps** (left out): which functions, modules or files to change, in what order",
        "never as a suggestion",
        "if a different correct implementation would fail it, it is a\n    step.",
    ] {
        assert!(text.contains(want), "{want:?} not in SKILL.md");
    }
    let example = text.split("## Worked example").nth(1).unwrap();
    let message = example.split("```").nth(1).unwrap();
    for heading in [
        "## Overview",
        "## Outcome",
        "## Constraints",
        "## Acceptance Criteria",
    ] {
        assert!(message.contains(heading), "{heading} not in:\n{message}");
    }
    assert!(!text.contains("## Requirements"), "{message}");
}

/// The skill's reference on writing a message gives the body's shape and migration 110's step
/// rewritten as an outcome.
#[test]
fn test_init_skill_messages_reference_gives_the_body_shape_and_a_step_rewritten() {
    let dir = TempDir::new().unwrap();
    decree_cmd(&dir)
        .args(["init", "--ai", "claude"])
        .assert()
        .success();
    let text = fs::read_to_string(
        dir.path()
            .join(".claude/skills/decree/reference/messages.md"),
    )
    .unwrap();
    let section = text
        .split("### Writing a good message")
        .nth(1)
        .unwrap()
        .split("\n## ")
        .next()
        .unwrap();
    let shape = [
        "  ## Overview\n",
        "  ## Outcome\n",
        "  ## Constraints\n",
        "  ## Acceptance Criteria\n",
    ];
    let at: Vec<usize> = shape
        .iter()
        .map(|h| section.find(h).unwrap_or_else(|| panic!("{h:?}")))
        .collect();
    assert!(at.windows(2).all(|w| w[0] < w[1]), "{at:?}");
    for want in [
        "**Describe the outcome, not the steps.**",
        "**Step:** The warning has its own rule id, following the existing warnings.",
        "**Outcome:** `decree check` shows this warning and still exits 0, in text, JSON and\n  > SARIF.",
    ] {
        assert!(section.contains(want), "{want:?} not in:\n{section}");
    }
}

/// decree never reads a message's body: one with `## Requirements` and no `## Outcome` passes
/// `check` and runs as any other.
#[test]
fn test_message_with_requirements_heading_checks_and_runs() {
    let dir = TempDir::new().unwrap();
    let decree = dir.path().join(".decree");
    fs::create_dir_all(decree.join("machines")).unwrap();
    fs::create_dir_all(decree.join("scripts")).unwrap();
    common::write_file(&decree.join("machines/hello/machine.yml"), "name: hello\ndescription: Run one script.\ninitial: greet\nstates:\n  greet:\n    invoke: greet\n    \
         transitions: { done: done }\n  done: { final: true }\n  failed: { final: true }\n");
    write_script(
        &decree.join("scripts/greet.sh"),
        "#!/bin/sh\ngrep -q '^## Requirements' \"$DECREE_MESSAGE\"\n",
    );
    common::write_file(
        &decree.join("inbox/01-old-shape.md"),
        "---\nmachine: hello\n---\n# Old shape\n\n## Requirements\n\n1. Say hello.\n\n\
         ## Acceptance Criteria\n\n- **Given** nothing **When** it runs **Then** it exits 0\n",
    );
    decree_cmd(&dir).arg("graph").assert().success();
    assert_check_passes(&dir);
    decree_cmd(&dir)
        .args(["daemon", "--once", "--quiet"])
        .assert()
        .code(0);
    let run = common::run_of(&decree, "01-old-shape.md");
    let events = fs::read_to_string(decree.join("runs").join(run).join("events.jsonl")).unwrap();
    assert!(events.contains("\"to\":\"done\""), "{events}");
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
