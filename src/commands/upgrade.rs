//! `decree upgrade`: print how to upgrade decree, as `rustup self update` and `uv self update`
//! are reached from the binary itself, but only print: the person runs the line. No network
//! call, no other program, no project needed (docs/reference/cli.md, Upgrading decree).

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
/// The official plugins are built into decree, so a new version may change them: `plugin list`
/// marks each one the project's copy differs from, and `plugin add --force` takes the new copy.
const AFTER: [(&str, Option<&str>); 4] = [
    ("decree skill", None),
    ("decree schema", Some("the project has .decree/schema/")),
    (
        "decree plugin list",
        Some("the project has .decree/plugins/: it marks `differs` each plugin the new version changed"),
    ),
    (
        "decree plugin add --force <name>",
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

pub fn run(format: Format) -> Result<(), DecreeError> {
    match format {
        Format::Text => {
            print!("{}", text());
            Ok(())
        }
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
