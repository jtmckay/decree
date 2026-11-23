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
        "decree install  (always: it writes this version's .decree/schema/ and installs \
         every plugin as its file says)"
            .into(),
    ] {
        assert!(
            lines.contains(&want.as_str()),
            "no line `{want}`:\n{stdout}"
        );
    }
    assert!(stdout.contains("`which -a decree`"), "{stdout}");
    // AC (A10): the after-steps say `list` shows which plugins the new version changed,
    // and `decree install --official <name>` takes the new copy.
    assert!(
        lines.iter().any(|l| l.starts_with("decree list")
            && l.contains("`differs`")
            && l.contains("new version changed")),
        "{stdout}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("decree install --official <name>")
                && l.contains("new version's copy")),
        "{stdout}"
    );
    assert!(stdout.contains("`cargo uninstall decree`"), "{stdout}");
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("decree install  (always") && l.contains(".decree/schema/")),
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
                "decree install",
                "decree list",
                "decree install --official <name>"
            ]
        );
    }
}

/// `decree upgrade <args>` in `dir`, with `stdin`, and a `PATH` whose `curl` is a stub that
/// prints a script saying it ran: (exit code, stdout, stderr).
fn upgrade_with(dir: &Path, args: &[&str], stdin: &str) -> (i32, String, String) {
    upgrade_with_decree(
        dir,
        args,
        stdin,
        "#!/bin/sh\necho \"stub decree $1 ran in $(pwd)\"\n",
    )
}

/// `upgrade_with`, with `decree_stub` as the `decree` on PATH.
fn upgrade_with_decree(
    dir: &Path,
    args: &[&str],
    stdin: &str,
    decree_stub: &str,
) -> (i32, String, String) {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let curl = bin.join("curl");
    std::fs::write(&curl, "#!/bin/sh\necho 'echo stub install ran'\n").unwrap();
    // The new copy on PATH, whose `decree install` the prompt after the install line runs.
    let decree = bin.join("decree");
    std::fs::write(&decree, decree_stub).unwrap();
    use std::os::unix::fs::PermissionsExt;
    for stub in [&curl, &decree] {
        std::fs::set_permissions(stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
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

/// After the install line, in a project, it offers to run the new copy's `decree install`
/// there (with `--yes`, without asking), and says to run it in every other project.
#[test]
fn offers_decree_install_in_the_project_after_installing() {
    for (args, answer, ran) in [
        (&[][..], "y\ny\n", true),
        (&[][..], "y\nn\n", false),
        (&["--yes"][..], "", true),
    ] {
        let tmp = dir(true);
        let (code, stdout, stderr) = upgrade_with(tmp.path(), args, answer);
        assert_eq!(code, 0, "{stderr}");
        assert!(stdout.contains("stub install ran\n"), "{stdout}");
        assert_eq!(
            stdout.contains(&format!(
                "Run `decree install` in {} now? [y/N] ",
                tmp.path().display()
            )),
            args.is_empty(),
            "{stdout}"
        );
        assert_eq!(
            stdout.contains("stub decree install ran in "),
            ran,
            "{stdout}"
        );
        let next = if ran {
            "Run `decree install` in each other project too."
        } else {
            "Run `decree install` in each project before its next daemon start."
        };
        assert!(stdout.ends_with(&format!("{next}\n")), "{stdout}");
    }
    // Outside a project there is nothing to offer.
    let tmp = dir(false);
    let (_, stdout, _) = upgrade_with(tmp.path(), &["--yes"], "");
    assert!(!stdout.contains("stub decree"), "{stdout}");
}

/// The new copy's `decree install` gets no stdin (it never waits on a pipe), and when it
/// fails, the upgrade still succeeds, with a warning: decree itself was installed.
#[test]
fn a_failing_project_install_is_a_warning() {
    let tmp = dir(true);
    let stub = "#!/bin/sh\nif [ \"$1\" = install ]; then cat >/dev/null; echo broken >&2; exit 3; fi\necho 'decree 9.9.9'\n";
    let (code, stdout, stderr) = upgrade_with_decree(tmp.path(), &["--yes"], "", stub);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.contains("with the `decree` on PATH (decree 9.9.9)"),
        "{stdout}"
    );
    assert!(
        stderr.contains("warning: decree install failed in"),
        "{stderr}"
    );
}
