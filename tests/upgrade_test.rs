//! `decree upgrade` (docs/reference/cli.md, Upgrading decree): prints the running version,
//! the README's install line, its pinned form, the `cargo install --git … --force` line with
//! the `which -a decree` note, and what to run in each project; exit 0 with or without a
//! `.decree/`. It only prints: no network call, no other program.

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

#[path = "common/schema.rs"]
mod schema;

/// `decree <args>` in `dir` with an empty `PATH`, so it could run no other program:
/// (exit code, stdout, stderr).
fn run(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = cargo_bin_cmd!("decree")
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("PATH", "")
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

/// The README's install line: the line of its first ```bash block that runs `install.sh`.
fn readme_install_line() -> String {
    let readme =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md")).unwrap();
    let lines: Vec<&str> = readme
        .lines()
        .skip_while(|l| l.trim_end() != "```bash")
        .skip(1)
        .take_while(|l| l.trim_end() != "```")
        .filter(|l| l.ends_with("/install.sh | sh"))
        .collect();
    assert_eq!(
        lines.len(),
        1,
        "README's first bash block has one install line"
    );
    lines[0].to_string()
}

/// A temp directory with a `.decree/` in it, or without.
fn dir(project: bool) -> TempDir {
    let tmp = TempDir::new().unwrap();
    if project {
        std::fs::create_dir(tmp.path().join(".decree")).unwrap();
    }
    tmp
}

#[test]
fn prints_how_to_upgrade_in_an_empty_directory() {
    let tmp = dir(false);
    let (code, stdout, stderr) = run(tmp.path(), &["upgrade"]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stderr, "");
    let version = env!("CARGO_PKG_VERSION");
    let install = readme_install_line();
    let lines: Vec<&str> = stdout.lines().map(str::trim).collect();
    assert_eq!(lines[0], format!("decree {version}"), "{stdout}");
    for want in [
        install.clone(),
        format!("{install} -s -- <tag>"),
        "cargo install --git https://github.com/jtmckay/decree --tag <tag> --force".into(),
        "decree skill".into(),
    ] {
        assert!(
            lines.contains(&want.as_str()),
            "no line `{want}`:\n{stdout}"
        );
    }
    assert!(stdout.contains("`which -a decree`"), "{stdout}");
    assert!(stdout.contains("`cargo uninstall decree`"), "{stdout}");
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("decree schema") && l.contains(".decree/schema/")),
        "{stdout}"
    );
    // It wrote nothing.
    assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
}

#[test]
fn prints_the_same_inside_a_project() {
    let outside = run(dir(false).path(), &["upgrade"]);
    let inside = run(dir(true).path(), &["upgrade"]);
    assert_eq!(outside, inside);
}

/// The README's install line appears verbatim in the output, so the two change together
/// (when `v0.5` becomes `main` in the URL).
#[test]
fn prints_the_readmes_install_line() {
    let (_, stdout, _) = run(dir(false).path(), &["upgrade"]);
    let install = readme_install_line();
    assert!(
        stdout.lines().any(|l| l.trim() == install),
        "`decree upgrade` does not print the README's `{install}`:\n{stdout}"
    );
}

/// `--format json` validates against `upgrade.schema.json` and has the same content as text,
/// with or without a `.decree/`.
#[test]
fn json_output_matches_its_schema() {
    let validator = schema::validator(schema::CLI_UPGRADE_SCHEMA);
    let (_, text, _) = run(dir(false).path(), &["upgrade"]);
    for project in [false, true] {
        let (code, stdout, stderr) = run(dir(project).path(), &["upgrade", "--format", "json"]);
        assert_eq!(code, 0, "{stderr}");
        let doc: Value = serde_json::from_str(&stdout).unwrap();
        let errors = schema::errors(&validator, &doc);
        assert!(errors.is_empty(), "{errors:?}\n{doc:#}");
        assert_eq!(doc["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(doc["install"], readme_install_line().as_str());
        for key in ["install", "pinned", "cargo", "cargo_note", "releases"] {
            let value = doc[key].as_str().unwrap();
            assert!(text.contains(value), "text lacks {key} `{value}`:\n{text}");
        }
        let after: Vec<&str> = doc["after"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["command"].as_str().unwrap())
            .collect();
        assert_eq!(after, ["decree skill", "decree schema"]);
    }
}
