//! `decree upgrade`: print how to upgrade decree, then offer to run the README's install line,
//! as `rustup self update` and `uv self update` are reached from the binary itself. It runs the
//! line only on a `y` (or `--yes`); otherwise no network call and no other program. No project
//! needed (docs/reference/cli.md, Upgrading decree).

use crate::cli::Format;
use crate::commands::print_json;
use crate::error::DecreeError;

/// The repository releases and `cargo install --git` come from.
const REPOSITORY: &str = "https://github.com/jtmckay/decree";

/// The install line of the README, the only place in the source that names `install.sh`'s
/// URL; `tests/upgrade_test.rs` checks the README's line is the same.
const INSTALL: &str =
    "curl -fsSL https://raw.githubusercontent.com/jtmckay/decree/main/install.sh | sh";

/// Stands for a release tag in the pinned lines.
const TAG: &str = "<tag>";

/// What to say about a second copy of decree on `PATH`.
const TWO_COPIES: &str = "If `which -a decree` lists two copies, the one first on PATH runs, \
                          so remove the other (`cargo uninstall decree`).";

/// What to run in each project afterwards: the command, and when it is needed.
/// The official plugins are built into decree, so a new version may change them: `list`
/// marks each one the project's copy differs from, and `decree install --force` takes the new copy.
const AFTER: [(&str, Option<&str>); 4] = [
    (
        "decree install --force skill",
        Some("the project has the skill plugin: the next daemon start writes the new skill"),
    ),
    ("decree schema", Some("the project has .decree/schema/")),
    (
        "decree list",
        Some("the project has .decree/plugins/: it marks `differs` each plugin the new version changed"),
    ),
    (
        "decree install --force <name>",
        Some("you want the new version's copy of a plugin marked `differs`: it prints the edits it drops"),
    ),
];

/// The running version.
fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn pinned() -> String {
    format!("{INSTALL} -s -- {TAG}")
}

fn cargo() -> String {
    format!("cargo install --git {REPOSITORY} --tag {TAG} --force")
}

pub fn run(yes: bool, format: Format) -> Result<(), DecreeError> {
    match format {
        Format::Text => {
            print!("{}", text());
            println!();
            if !yes {
                use std::io::Write;
                print!("Run the install line now ({INSTALL})? [y/N] ");
                std::io::stdout().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !matches!(answer.trim().to_lowercase().as_str(), "y" | "yes") {
                    if answer.is_empty() {
                        println!();
                    }
                    println!("Nothing was installed.");
                    return Ok(());
                }
            }
            install()
        }
        Format::Json if yes => Err(DecreeError::Other(
            "--yes runs the install line, whose output is not JSON; run `decree upgrade --yes` without --format json".into(),
        )),
        Format::Json => print_json(&serde_json::json!({
            "version": version(),
            "install": INSTALL,
            "pinned": pinned(),
            "cargo": cargo(),
            "cargo_note": TWO_COPIES,
            "releases": format!("{REPOSITORY}/releases"),
            "after": AFTER
                .iter()
                .map(|(command, when)| match when {
                    Some(when) => serde_json::json!({ "command": command, "when": when }),
                    None => serde_json::json!({ "command": command }),
                })
                .collect::<Vec<_>>(),
        })),
    }
}

fn text() -> String {
    let mut out = format!(
        "decree {version}\n\
         \n\
         Install the newest release:\n  {INSTALL}\n\
         Or a given release ({TAG} such as v{version}; releases: {REPOSITORY}/releases):\n  {pinned}\n\
         \n\
         If this copy came from `cargo install`, the line above does not replace it:\n  {cargo}\n\
         {TWO_COPIES}\n\
         \n\
         Then, in each project:\n",
        version = version(),
        pinned = pinned(),
        cargo = cargo(),
    );
    for (command, when) in AFTER {
        match when {
            Some(when) => out.push_str(&format!("  {command}  (if {when})\n")),
            None => out.push_str(&format!("  {command}\n")),
        }
    }
    out
}

/// Run the install line with `sh`, its output going straight through.
fn install() -> Result<(), DecreeError> {
    println!("Running: {INSTALL}");
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(INSTALL)
        .status()
        .map_err(|e| DecreeError::Other(format!("cannot run sh: {e}")))?;
    if !status.success() {
        return Err(DecreeError::Other(format!(
            "the install line failed ({status}); nothing in the projects changed"
        )));
    }
    println!("Installed. Run `decree --version` to see the new version, then the steps above in each project.");
    Ok(())
}
