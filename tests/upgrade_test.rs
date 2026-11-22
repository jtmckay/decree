//! `decree upgrade` (docs/reference/cli.md, Upgrading decree): prints the running version,
//! the README's install line, its pinned form, the `cargo install --git … --force` line with
//! the `which -a decree` note, and what to run in each project; exit 0 with or without a
//! `.decree/`; then it asks before running the install line, and runs it only on `y` or `--yes`.

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
        "decree install --force skill  (if the project has the skill plugin: the next daemon \
         start writes the new skill)"
            .into(),
    ] {
        assert!(
            lines.contains(&want.as_str()),
            "no line `{want}`:\n{stdout}"
        );
    }
    assert!(stdout.contains("`which -a decree`"), "{stdout}");
    // AC (A10): the after-steps say `list` shows which plugins the new version changed,
    // and `decree install --force <name>` takes the new copy.
    assert!(
        lines.iter().any(|l| l.starts_with("decree list")
            && l.contains("`differs`")
            && l.contains("new version changed")),
        "{stdout}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("decree install --force <name>")
                && l.contains("new version's copy")),
        "{stdout}"
    );
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
/// (the URL names the branch the install script is read from).
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
        assert_eq!(
            after,
            [
                "decree install --force skill",
                "decree schema",
                "decree list",
                "decree install --force <name>"
            ]
        );
    }
}

/// `decree upgrade <args>` in `dir`, with `stdin`, and a `PATH` whose `curl` is a stub that
/// prints a script saying it ran: (exit code, stdout, stderr).
fn upgrade_with(dir: &Path, args: &[&str], stdin: &str) -> (i32, String, String) {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let curl = bin.join("curl");
    std::fs::write(&curl, "#!/bin/sh\necho 'echo stub install ran'\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = cargo_bin_cmd!("decree")
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .args(["upgrade"])
        .args(args)
        .write_stdin(stdin)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

/// It asks before running the install line: no answer, or anything but `y`, runs nothing.
#[test]
fn asks_and_runs_nothing_without_a_yes() {
    for answer in ["", "n\n", "maybe\n"] {
        let tmp = dir(false);
        let (code, stdout, stderr) = upgrade_with(tmp.path(), &[], answer);
        assert_eq!(code, 0, "{stderr}");
        let install = readme_install_line();
        assert!(
            stdout.contains(&format!("Run the install line now ({install})? [y/N] ")),
            "{stdout}"
        );
        assert!(stdout.ends_with("Nothing was installed.\n"), "{stdout}");
        assert!(!stdout.contains("stub install ran"), "{stdout}");
    }
}

/// `y` at the prompt, or `--yes`, runs the README's install line with `sh`.
#[test]
fn runs_the_install_line_on_y_or_yes() {
    for (args, answer) in [(&[][..], "y\n"), (&["--yes"][..], "")] {
        let tmp = dir(false);
        let (code, stdout, stderr) = upgrade_with(tmp.path(), args, answer);
        assert_eq!(code, 0, "{stderr}");
        assert!(
            stdout.contains(&format!("Running: {}\n", readme_install_line())),
            "{stdout}"
        );
        assert!(stdout.contains("stub install ran\n"), "{stdout}");
        assert!(stdout.contains("Installed."), "{stdout}");
    }
    let tmp = dir(false);
    let (code, stdout, stderr) = upgrade_with(tmp.path(), &["--yes", "--format", "json"], "");
    assert_eq!((code, stdout.as_str()), (1, ""), "{stderr}");
    assert!(stderr.contains("--yes runs the install line"), "{stderr}");
}
