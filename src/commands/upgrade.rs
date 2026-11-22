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
/// `decree install` writes the new version's schemas and installs each plugin as its file
/// says. The official plugins are built into decree, so a new version may change them: `list`
/// marks each one the project's copy differs from, and `decree install --force` takes the new copy.
const AFTER: [(&str, Option<&str>); 3] = [
    (
        "decree install",
        Some("always: it writes this version's .decree/schema/ and installs every plugin as its file says"),
    ),
    (
        "decree list",
        Some("if you want to see each official plugin the new version changed: it marks them `differs`"),
    ),
    (
        "decree install --force <name>",
        Some("if you want the new version's copy of a plugin marked `differs`, such as skill: it prints the edits it drops"),
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
            install()?;
            install_here(yes)
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
            Some(when) => out.push_str(&format!("  {command}  ({when})\n")),
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
    println!("Installed. Now run `decree install` in each project, then the other steps above.");
    Ok(())
}

/// After the install line: offer to run the new `decree install` in the project around the
/// working directory, if there is one (`yes` runs it without asking). It runs `decree` from
/// `PATH`, the copy just installed, not this one.
fn install_here(yes: bool) -> Result<(), DecreeError> {
    let Some(root) = crate::error::find_project_root() else {
        return Ok(());
    };
    if !yes {
        use std::io::Write;
        print!("Run `decree install` in {} now? [y/N] ", root.display());
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim().to_lowercase().as_str(), "y" | "yes") {
            if answer.is_empty() {
                println!();
            }
            println!("Run `decree install` in each project before its next daemon start.");
            return Ok(());
        }
    }
    let status = std::process::Command::new("decree")
        .arg("install")
        .current_dir(&root)
        .status()
        .map_err(|e| DecreeError::Other(format!("cannot run decree install: {e}")))?;
    if !status.success() {
        return Err(DecreeError::Other(format!(
            "decree install failed in {} ({status})",
            root.display()
        )));
    }
    println!("Run `decree install` in each other project too.");
    Ok(())
}
