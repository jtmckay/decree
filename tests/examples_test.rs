//! Every project in `examples/` is a 0.5 project.
//! `decree check` passes in each without a warning, no 0.4 word remains in the
//! examples, `src/`, `docs/reference/`, `README.md` or `tests/`, and each README's
//! commands run as written, with a stub on `PATH` for the services they call.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod common;
use common::write_script;

/// Words from decree 0.4 that must not appear in `src/`, `docs/reference/`, `README.md`,
/// `tests/` or `examples/`, matched as whole words, ignoring case. `docs/decisions.md` and
/// `docs/code-review.md` keep the history, and this file holds the list.
/// Versions are named precisely (`0.4.2`, `decree 0.4`, `v0.4`), so a number such as a
/// confidence threshold of `0.4` or a ComfyUI workflow's `"version": 0.4` is not one.
const VERSION_TERMS: &[&str] = &[
    "routine",
    "config.yml",
    "migrate-0.4",
    "0.4.2",
    "decree 0.4",
    "v0.4",
];

/// 0.4 concepts that must not appear in `examples/` either.
const OLD_TERMS: &[&str] = &["routines", "outbox", "hooks", "ai_router"];

/// This file, which holds the word lists.
const THIS_FILE: &str = "tests/examples_test.rs";

/// The example frozen partway through its life: it commits their queues, recorded runs
/// and ledger so they can be read, and `replay_test.rs` replays the runs. The other examples
/// start fresh.
const RECORDED: &[&str] = &["project"];

fn repo() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn examples() -> PathBuf {
    repo().join("examples")
}

/// The examples that are decree projects, in name order.
fn projects() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(examples())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join(".decree").is_dir())
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whole-word matches of `term` in `line`, ignoring case, as `rg -w -i` finds them.
fn has_word(line: &str, term: &str) -> bool {
    let line = line.to_lowercase();
    line.match_indices(term).any(|(i, _)| {
        let before = line[..i].chars().next_back();
        let after = line[i + term.len()..].chars().next();
        !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
    })
}

#[test]
fn every_example_passes_check_without_a_warning() {
    let names = projects();
    assert_eq!(
        names,
        [
            "newsletter",
            "project",
            "route-by-complexity",
            "text-to-media"
        ]
    );
    for name in names {
        let dir = tempfile::tempdir().unwrap();
        copy_dir(&examples().join(&name), dir.path());
        let out = Command::new(env!("CARGO_BIN_EXE_decree"))
            .arg("check")
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            out.status.success() && out.stdout.is_empty() && out.stderr.is_empty(),
            "decree check in examples/{name} ({}):\nstdout:\n{}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// The 0.4 words on one line of `rel` (a path relative to the repository).
fn old_words(rel: &Path, line: &str) -> Vec<&'static str> {
    let in_examples = rel.starts_with("examples");
    VERSION_TERMS
        .iter()
        .chain(if in_examples { OLD_TERMS } else { &[] })
        .copied()
        .filter(|term| has_word(line, term))
        .collect()
}

#[test]
fn no_0_4_word_in_src_docs_readme_tests_or_examples() {
    let mut all = Vec::new();
    for root in ["src", "docs/reference", "tests", "examples"] {
        files(&repo().join(root), &mut all);
    }
    all.push(repo().join("README.md"));
    let mut hits = Vec::new();
    for path in all {
        let rel = path.strip_prefix(repo()).unwrap();
        if rel == Path::new(THIS_FILE) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue; // not text
        };
        for (n, line) in text.lines().enumerate() {
            let words = old_words(rel, line);
            if !words.is_empty() {
                hits.push(format!("{}:{}: {words:?}: {line}", rel.display(), n + 1));
            }
        }
    }
    assert!(hits.is_empty(), "0.4 words remain:\n{}", hits.join("\n"));
}

#[test]
fn the_0_4_scan_matches_versions_not_thresholds() {
    let doc = Path::new("docs/reference/x.md");
    assert_eq!(
        old_words(doc, "as decree 0.4.2 did"),
        ["0.4.2", "decree 0.4"]
    );
    assert_eq!(old_words(doc, "In v0.4, a Routine"), ["routine", "v0.4"]);
    assert!(old_words(
        Path::new("examples/text-to-media/.decree/lib/comfy/a.json"),
        "\"version\": 0.4"
    )
    .is_empty());
    assert_eq!(old_words(doc, "no .decree/config.yml"), ["config.yml"]);
    assert!(old_words(doc, "check: { confidence: c, at_least: 0.4 }").is_empty());
    assert!(old_words(doc, "\"ts\":\"2026-10-01T14:43:30.410Z\", 0.45").is_empty());
    assert!(old_words(doc, "a hooks list, the outbox").is_empty());
    let example = Path::new("examples/x/README.md");
    assert_eq!(
        old_words(example, "hooks and routines"),
        ["routines", "hooks"]
    );
}

#[test]
fn every_example_ships_its_graph_not_its_schema_and_starts_fresh_unless_recorded() {
    for name in projects() {
        let decree = examples().join(&name).join(".decree");
        assert!(decree.join("graph/system.md").is_file(), "{name}: no graph");
        // Editors read the hosted schemas; a local copy is generated and ignored.
        assert!(!decree.join("schema").exists(), "{name}: .decree/schema");
        let gitignore = fs::read_to_string(decree.join(".gitignore")).unwrap();
        assert!(
            gitignore.lines().any(|l| l == "schema/"),
            "{name}: .gitignore does not list schema/"
        );
        // Secrets stay out of git; only the template is committed.
        for line in [".env*", "!.env.example"] {
            assert!(
                gitignore.lines().any(|l| l == line),
                "{name}: .gitignore does not list {line}"
            );
        }
        for gone in ["routines", "outbox"] {
            assert!(!decree.join(gone).exists(), "{name}: .decree/{gone} exists");
        }
        let recorded = RECORDED.contains(&name.as_str());
        assert_eq!(
            decree.join("runs").is_dir(),
            recorded,
            "{name}: .decree/runs"
        );
        if !recorded {
            for gone in ["processed.md", "inbox"] {
                assert!(!decree.join(gone).exists(), "{name}: .decree/{gone} exists");
            }
        }
        let mut machines = Vec::new();
        files(&decree.join("machines"), &mut machines);
        for machine in machines {
            let text = fs::read_to_string(&machine).unwrap();
            let stem = machine.file_stem().unwrap().to_string_lossy();
            assert_eq!(
                text.lines().next(),
                Some(format!("# Graph: ../graph/{stem}.md").as_str()),
                "{}",
                machine.display()
            );
            assert!(!text.contains("$schema="), "{}", machine.display());
        }
    }
}

// --- README commands -------------------------------------------------------

/// Stand-ins for the tools the examples call. Each does just enough for the
/// next script to find what it expects.
const STUBS: &[(&str, &str)] = &[
    // `-o <file>` gets a body; ComfyUI's /upload/image and /history/<id> get a reply that
    // names one saved image; anything else gets a ComfyUI prompt id.
    (
        "curl",
        r#"out=""; url=""
while [ $# -gt 0 ]; do case "$1" in -o) out=$2; shift 2 ;; http*) url=$1; shift ;; *) shift ;; esac; done
if [ -n "$out" ]; then echo stub > "$out"
elif [[ "$url" == */upload/image ]]; then printf '{"name":"stub.png","subfolder":"","type":"input"}'
elif [[ "$url" == */history/* ]]; then printf '{"stub":{"status":{"status_str":"success"},"outputs":{"9":{"images":[{"filename":"stub.png","subfolder":"","type":"output"}]}}}}'
else printf '{"prompt_id":"stub"}'; fi"#,
    ),
];

/// Runs `decree daemon` until every queued message has a finished run, then
/// stops it with SIGTERM, which is a clean exit (0).
const RUN_DAEMON: &str = r#"run_daemon() {
  decree daemon --interval 1s & local pid=$!
  for _ in $(seq 100); do
    if [ -z "$(ls .decree/inbox)" ] && ! grep -L '^state: \(done\|failed\)' .decree/runs/*/message.md | grep -q .; then
      break
    fi
    sleep 0.1
  done
  kill -TERM "$pid"
  wait "$pid"
}
"#;

fn readme_bash_blocks(readme: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in readme.lines() {
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
    assert!(current.is_none(), "unclosed ```bash block");
    blocks
}

/// Copies `examples/<name>` into a temp directory (as `examples/<name>`, so the
/// README's `cd` works), runs `prepare` and then every line of the README's
/// ```bash blocks in one shell, with the stubs and this build of decree first
/// on PATH. `decree daemon` runs
/// until the queue is empty, then gets SIGTERM. Returns the project copy.
fn run_readme(name: &str, prepare: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("examples").join(name);
    copy_dir(&examples().join(name), &project);

    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    for (tool, body) in STUBS {
        write_script(&bin.join(tool), &format!("#!/usr/bin/env bash\n{body}\n"));
    }
    let decree_dir = Path::new(env!("CARGO_BIN_EXE_decree")).parent().unwrap();
    let path = std::env::join_paths(
        [bin, decree_dir.to_path_buf()]
            .into_iter()
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();

    let readme = fs::read_to_string(examples().join(name).join("README.md")).unwrap();
    let mut script = format!("set -euo pipefail\n{RUN_DAEMON}(cd examples/{name} && {prepare})\n");
    for block in readme_bash_blocks(&readme) {
        for line in block.lines() {
            if line == "decree daemon" {
                script.push_str("run_daemon\n");
            } else {
                script.push_str(line);
                script.push('\n');
            }
        }
    }

    let out = Command::new("bash")
        .arg("-c")
        .arg(&script)
        .current_dir(dir.path())
        .env("PATH", path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "examples/{name}/README.md commands failed ({}):\nscript:\n{script}\nstdout:\n{}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    dir
}

/// Every run in the project ended in `done`; returns how many there are.
fn all_runs_done(project: &Path) -> usize {
    let runs = project.join(".decree/runs");
    let mut count = 0;
    for entry in fs::read_dir(&runs).unwrap() {
        let message = entry.unwrap().path().join("message.md");
        let text = fs::read_to_string(&message).unwrap();
        assert!(
            text.lines().any(|l| l == "state: done"),
            "{}:\n{text}",
            message.display()
        );
        count += 1;
    }
    count
}

fn processed(project: &Path) -> Vec<String> {
    fs::read_to_string(project.join(".decree/processed.md"))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// jq is a real dependency of `text-to-media`'s scripts, not a stand-in.
fn has_jq() -> bool {
    let found = Command::new("jq").arg("--version").output().is_ok();
    if !found {
        eprintln!("jq is not installed: skipping this example's README commands");
    }
    found
}

#[test]
fn text_to_media_readme_commands_run() {
    if !has_jq() {
        return;
    }
    // The README's prerequisite: a picture of yours for the third render.
    let dir = run_readme(
        "text-to-media",
        "mkdir -p images && echo picture > images/reference.png",
    );
    let project = dir.path().join("examples/text-to-media");
    assert_eq!(processed(&project).len(), 4);
    assert_eq!(all_runs_done(&project), 5);
    for render in [
        "unicorn_landscape",
        "character_lily_fullbody",
        "style_transfer_demo",
        "lily_waving",
        "lighthouse",
    ] {
        assert!(
            project.join(format!("output/{render}.png")).is_file(),
            "{render}"
        );
    }
    let payload =
        fs::read_to_string(project.join(".decree/runs/04-animate-character/comfy-payload.json"))
            .unwrap();
    assert!(payload.contains("The character gently waves"), "{payload}");
}

/// The recorded examples' commands only read the project: nothing runs, and `decree graph`
/// changes nothing.
fn assert_readme_reads_only(name: &str) {
    let dir = run_readme(name, "true");
    let project = dir.path().join("examples").join(name);
    let mut copied = Vec::new();
    files(&project, &mut copied);
    let mut original = Vec::new();
    files(&examples().join(name), &mut original);
    assert_eq!(copied.len(), original.len(), "{name}");
    for path in original {
        let rel = path.strip_prefix(examples().join(name)).unwrap();
        assert_eq!(
            fs::read(project.join(rel)).unwrap(),
            fs::read(&path).unwrap(),
            "{name}/{}",
            rel.display()
        );
    }
}

#[test]
fn newsletter_readme_commands_run() {
    assert_readme_reads_only("newsletter");
}

#[test]
fn project_readme_commands_run() {
    assert_readme_reads_only("project");
}

#[test]
fn route_by_complexity_readme_commands_run() {
    assert_readme_reads_only("route-by-complexity");
}
