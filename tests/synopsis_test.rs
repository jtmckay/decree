//! Bare `decree` (docs/reference/cli.md): prints `src/templates/synopsis.md` byte for byte and
//! exits 0, inside a project or not, and reads, writes and runs nothing. The synopsis's steps
//! run as written, and its install line is the README's and `decree upgrade`'s.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use tempfile::TempDir;

mod common;

const SYNOPSIS: &str = include_str!("../src/templates/synopsis.md");

/// `decree <args>` in `dir`: (exit code, stdout, stderr).
fn run(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = cargo_bin_cmd!("decree")
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

/// Every path under `dir`, relative and sorted, so a test can tell nothing changed.
fn tree(dir: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            out.push(path.strip_prefix(root).unwrap().display().to_string());
            if path.is_dir() {
                walk(root, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

#[test]
fn bare_decree_prints_the_synopsis_outside_a_project() {
    let dir = TempDir::new().unwrap();
    let (code, stdout, stderr) = run(dir.path(), &[]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, SYNOPSIS);
    assert_eq!(stderr, "");
    assert!(tree(dir.path()).is_empty(), "{:?}", tree(dir.path()));
}

#[test]
fn bare_decree_with_no_color_prints_the_synopsis() {
    let dir = TempDir::new().unwrap();
    let (code, stdout, _) = run(dir.path(), &["--no-color"]);
    assert_eq!(code, 0);
    assert_eq!(stdout, SYNOPSIS);
}

/// A pending migration stays pending: no run, no `processed.md` line, no file written.
#[test]
fn bare_decree_in_a_project_starts_no_run() {
    let dir = TempDir::new().unwrap();
    assert_eq!(run(dir.path(), &["init"]).0, 0);
    let decree = dir.path().join(".decree");
    fs::write(
        decree.join("migrations/01-a.md"),
        "---\nmachine: develop\n---\n# A\n",
    )
    .unwrap();
    let before = tree(dir.path());

    let (code, stdout, stderr) = run(dir.path(), &[]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, SYNOPSIS);
    assert_eq!(stderr, "");
    assert_eq!(tree(dir.path()), before);
    assert!(fs::read_dir(decree.join("runs")).map_or(true, |mut d| d.next().is_none()));

    let (_, status, _) = run(dir.path(), &["status"]);
    assert!(status.contains("migrations/: 1 pending"), "{status}");
}

/// The synopsis's numbered steps, by number: the indented lines under each, trimmed.
fn steps() -> std::collections::BTreeMap<u32, Vec<String>> {
    let mut steps: Vec<(u32, Vec<String>)> = Vec::new();
    for line in SYNOPSIS.lines() {
        let numbered = line
            .split_once(". ")
            .and_then(|(n, _)| n.parse::<u32>().ok())
            .filter(|_| !line.starts_with(' '));
        match (numbered, steps.last_mut()) {
            (Some(n), _) => steps.push((n, Vec::new())),
            (None, Some((_, lines))) if line.starts_with(' ') => {
                lines.push(line.trim().to_string())
            }
            _ => {}
        }
    }
    let numbers: Vec<u32> = steps.iter().map(|(n, _)| *n).collect();
    assert_eq!(numbers, [1, 2, 3, 4, 5], "the synopsis's steps");
    steps.into_iter().collect()
}

/// The first `` `…` `` span of `line`.
fn quoted(line: &str) -> Option<&str> {
    let rest = line.split_once('`')?.1;
    Some(rest.split_once('`')?.0)
}

/// The one file under `.decree/` a step names.
fn step_file(lines: &[String]) -> String {
    let files: Vec<&str> = lines
        .iter()
        .filter_map(|l| quoted(l))
        .filter(|q| q.starts_with(".decree/"))
        .collect();
    assert_eq!(files.len(), 1, "a step names one file: {lines:?}");
    files[0].to_string()
}

/// Steps 1 to 5 of the synopsis in an empty directory, as `tests/readme_test.rs` runs the
/// README: the commands of steps 1 and 5 as written, and the files steps 2 to 4 name, with the
/// least each needs. The `hello` run ends `done`.
#[test]
fn synopsis_steps_run_as_written() {
    let steps = steps();
    let dir = TempDir::new().unwrap();
    let decree = |line: &str| {
        let args: Vec<&str> = line.split_whitespace().skip(1).collect();
        let (code, stdout, stderr) = run(dir.path(), &args);
        assert_eq!(code, 0, "`{line}` failed:\n{stdout}\n{stderr}");
    };

    // 1. Set up a project.
    assert_eq!(steps[&1], ["decree init"]);
    decree(&steps[&1][0]);

    // 2. A script: exit 0 is `done`.
    let script = step_file(&steps[&2]);
    let path = dir.path().join(&script);
    fs::write(&path, "#!/usr/bin/env bash\necho hello\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    let script_name = Path::new(&script).file_stem().unwrap().to_str().unwrap();

    // 3. A machine that runs it.
    let machine = step_file(&steps[&3]);
    let machine_name = Path::new(&machine).file_stem().unwrap().to_str().unwrap();
    assert_eq!(machine_name, "hello");
    fs::write(
        dir.path().join(&machine),
        format!(
            "name: {machine_name}\ndescription: Greet, then finish.\ninitial: greet\nstates:\n  \
             greet:\n    invoke: {script_name}\n    transitions: {{ done: done }}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        ),
    )
    .unwrap();

    // 4. A message for that machine.
    let message = step_file(&steps[&4]);
    fs::write(
        dir.path().join(&message),
        format!("---\nmachine: {machine_name}\n---\n# Say hello\n"),
    )
    .unwrap();

    // 5. The commands, as written. `decree tail` without an id follows runs until stopped,
    // so it is the one left out.
    // Each line starts with the command, in backticks or not: `decree <command>`.
    let commands: Vec<String> = steps[&5]
        .iter()
        .map(|l| l.trim_start_matches('`'))
        .filter(|l| l.starts_with("decree "))
        .map(|l| l.split(['`', ' ']).take(2).collect::<Vec<_>>().join(" "))
        .collect();
    for want in [
        "decree check",
        "decree graph",
        "decree process",
        "decree status",
    ] {
        assert!(commands.iter().any(|c| c == want), "step 5 has no `{want}`");
    }
    for command in commands.iter().filter(|c| *c != "decree tail") {
        decree(command);
    }

    let stem = Path::new(&message).file_stem().unwrap().to_str().unwrap();
    let (code, stdout, stderr) = run(dir.path(), &["status", stem, "--format", "json"]);
    assert_eq!(code, 0, "{stderr}");
    let doc: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["state"], "done", "{stdout}");
    assert_eq!(doc["status"], "finished", "{stdout}");
    assert_eq!(doc["machine"], "hello", "{stdout}");
    let id = common::migration_run(&dir.path().join(".decree"), "01-hello.md");
    assert_eq!(doc["id"], id.as_str());
}

/// The synopsis, the README's first ```bash block and `decree upgrade` all show one install
/// line.
#[test]
fn synopsis_readme_and_upgrade_show_one_install_line() {
    let lines: Vec<&str> = SYNOPSIS
        .lines()
        .map(str::trim)
        .filter(|l| l.ends_with("/install.sh | sh"))
        .collect();
    assert_eq!(
        lines.len(),
        1,
        "the synopsis has one install line: {lines:?}"
    );
    let synopsis = lines[0];

    let readme = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md")).unwrap();
    let readme_lines: Vec<&str> = readme
        .lines()
        .skip_while(|l| l.trim_end() != "```bash")
        .skip(1)
        .take_while(|l| l.trim_end() != "```")
        .filter(|l| l.ends_with("/install.sh | sh"))
        .collect();
    assert_eq!(readme_lines, [synopsis], "README and synopsis differ");

    let (_, upgrade, _) = run(TempDir::new().unwrap().path(), &["upgrade"]);
    assert!(
        upgrade.lines().any(|l| l.trim() == synopsis),
        "`decree upgrade` does not print the synopsis's `{synopsis}`:\n{upgrade}"
    );
}
