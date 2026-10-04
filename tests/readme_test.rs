//! README and help (docs/reference/cli.md, docs/reference/README.md): both describe 0.5 only, and every
//! command in the README runs as written on a fresh project.

use assert_cmd::cargo::cargo_bin_cmd;

/// Concepts decree no longer has, which must not appear in its help or README.
/// (`tests/examples_test.rs` scans both for older terms.)
const OLD_TERMS: &[&str] = &[
    "outbox",
    "hooks",
    "router.md",
    "dead/",
    "run.json",
    "DECREE_HOOK",
    "DECREE_PRE_CHECK",
    "message_file",
    "beforeEach",
];

/// The 12 commands of docs/reference/cli.md, `process --retry`, and `--version`.
const COMMANDS: &[&str] = &[
    "decree init",
    "decree process",
    "decree daemon",
    "decree check",
    "decree graph",
    "decree schema",
    "decree emit",
    "decree status",
    "decree tail",
    "decree process --retry",
    "decree prune",
    "decree event",
    "decree help",
    "decree --version",
];

fn help_text() -> String {
    let out = cargo_bin_cmd!("decree").arg("help").assert().success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

#[test]
fn test_help_lists_every_command() {
    let help = help_text();
    for cmd in COMMANDS {
        assert!(help.contains(cmd), "help does not mention `{cmd}`");
    }
}

#[test]
fn test_help_describes_the_three_blocks_and_links_docs() {
    let help = help_text();
    for word in [
        "Messages",
        "Machines",
        "Scripts",
        "docs/routers.md",
        "docs/services.md",
    ] {
        assert!(help.contains(word), "help does not mention {word}");
    }
}

#[test]
fn test_help_mentions_no_removed_concept() {
    let help = help_text();
    for term in OLD_TERMS {
        assert!(!help.contains(term), "help still mentions `{term}`");
    }
}

fn readme() -> String {
    std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md")).unwrap()
}

/// Every ```bash block of the README, in order.
fn readme_bash_blocks() -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in readme().lines() {
        match current.as_mut() {
            None if line.trim_end() == "```bash" => current = Some(String::new()),
            None => {}
            Some(_) if line.trim_end() == "```" => blocks.push(current.take().unwrap()),
            Some(block) => {
                block.push_str(line);
                block.push('\n');
            }
        }
    }
    assert!(current.is_none(), "README has an unclosed ```bash block");
    blocks
}

#[test]
fn test_readme_describes_the_three_blocks_and_links_docs() {
    let text = readme();
    for word in [
        "Messages",
        "Machines",
        "Scripts",
        "docs/routers.md",
        "docs/services.md",
    ] {
        assert!(text.contains(word), "README does not mention {word}");
    }
    for cmd in COMMANDS {
        assert!(text.contains(cmd), "README does not mention `{cmd}`");
    }
}

#[test]
fn test_readme_mentions_no_removed_concept() {
    let text = readme();
    for term in OLD_TERMS {
        assert!(!text.contains(term), "README still mentions `{term}`");
    }
}

/// Every command in the README runs as written on a fresh
/// project. All ```bash blocks run in order in one shell, in an empty temp
/// directory, with this build of decree first on PATH. `cargo install` is the one
/// line skipped: it needs the network, and the build under test stands in for it.
#[test]
fn test_every_readme_command_runs_as_written() {
    let blocks = readme_bash_blocks();
    assert!(
        blocks.iter().any(|b| b.lines().any(|l| l == "decree init")),
        "README's commands start with `decree init`"
    );
    let mut script = String::from("set -euo pipefail\n");
    for block in &blocks {
        for line in block.lines() {
            if !line.starts_with("cargo install ") {
                script.push_str(line);
                script.push('\n');
            }
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let bin_dir = std::path::Path::new(env!("CARGO_BIN_EXE_decree"))
        .parent()
        .unwrap()
        .to_path_buf();
    let path = std::env::join_paths(
        std::iter::once(bin_dir).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let out = std::process::Command::new("bash")
        .arg("-c")
        .arg(&script)
        .current_dir(dir.path())
        .env("PATH", path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "README commands failed ({}):\nstdout:\n{}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // The walkthrough did what the README says it does.
    let decree = dir.path().join(".decree");
    let processed = std::fs::read_to_string(decree.join("processed.md")).unwrap();
    assert!(processed.lines().any(|l| l == "01-hello.md"), "{processed}");
    let message = std::fs::read_to_string(decree.join("runs/01-hello/message.md")).unwrap();
    assert!(message.contains("state: done"), "{message}");
    assert!(decree.join("graph/hello.md").is_file());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("decree event 01-hello.w3 approve"),
        "{stdout}"
    );
}
