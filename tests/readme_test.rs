//! README and help (spec section 11, M5.5): both describe 0.5 only, and every
//! command in the README runs as written on a fresh project.

use assert_cmd::cargo::cargo_bin_cmd;

/// 0.4 concepts that must not appear in 0.5's help or README outside the 0.4 to 0.5
/// table (spec sections 3 and 10).
const OLD_TERMS: &[&str] = &[
    "routine",
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

/// The 11 commands of spec section 8, plus `--version`.
const COMMANDS: &[&str] = &[
    "decree init",
    "decree process",
    "decree daemon",
    "decree check",
    "decree graph",
    "decree emit",
    "decree status",
    "decree tail",
    "decree retry",
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
fn test_help_mentions_no_0_4_concept() {
    let help = help_text();
    for term in OLD_TERMS {
        assert!(!help.contains(term), "help still mentions `{term}`");
    }
}
