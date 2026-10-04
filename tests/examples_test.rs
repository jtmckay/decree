//! Every project in `examples/` is a 0.5 project.
//! `decree check` passes in each without a warning, no 0.4 term remains outside
//! the history told in `examples/decree/README.md`, and each README's commands
//! run as written, with stubs on `PATH` for the AI tools and services.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod common;
use common::write_script;

/// 0.4 words that must not appear in `examples/`, matched as whole words.
const OLD_TERMS: &[&str] = &["routine", "routines", "outbox", "hooks", "ai_router"];

/// The one file allowed to use them: it tells how decree was built under 0.4.
const HISTORY: &str = "decree/README.md";

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples")
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

/// Whole-word matches of `term` in `line`, as `rg -w` finds them.
fn has_word(line: &str, term: &str) -> bool {
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
            "business-eval",
            "docker",
            "text-to-media",
            "text-to-speech",
            "whisper-transcribe"
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

#[test]
fn no_0_4_term_in_examples_outside_the_decree_history() {
    let mut all = Vec::new();
    files(&examples(), &mut all);
    let mut hits = Vec::new();
    for path in all {
        let rel = path.strip_prefix(examples()).unwrap();
        if rel == Path::new(HISTORY) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue; // not text
        };
        for (n, line) in text.lines().enumerate() {
            if OLD_TERMS.iter().any(|t| has_word(line, t)) {
                hits.push(format!("examples/{}:{}: {line}", rel.display(), n + 1));
            }
        }
    }
    assert!(hits.is_empty(), "0.4 terms remain:\n{}", hits.join("\n"));
}

#[test]
fn every_example_ships_its_graph_and_starts_fresh() {
    for name in projects() {
        let decree = examples().join(&name).join(".decree");
        assert!(decree.join("graph/system.md").is_file(), "{name}: no graph");
        for gone in ["processed.md", "runs", "inbox", "routines", "outbox"] {
            assert!(!decree.join(gone).exists(), "{name}: .decree/{gone} exists");
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
        }
    }
}

// --- README commands -------------------------------------------------------

/// Stand-ins for the tools the examples call. Each does just enough for the
/// next script to find what it expects.
const STUBS: &[(&str, &str)] = &[
    // Writes the file the prompt asks for ("... markdown to <path>").
    (
        "claude",
        r#"prompt=""
while [ $# -gt 0 ]; do case "$1" in -p) prompt=$2; shift 2 ;; *) shift ;; esac; done
out=$(printf '%s\n' "$prompt" | grep -o 'markdown to [^ ]*' | tail -n 1 | cut -d' ' -f3-)
[ -z "$out" ] || printf '# stub report\n' > "$out"
echo "stub claude""#,
    ),
    ("opencode", r#"echo "stub opencode $1""#),
    // `-o <file>` gets a body; otherwise a ComfyUI reply plus the `-w` status code.
    (
        "curl",
        r#"out=""; w=""
while [ $# -gt 0 ]; do case "$1" in -o) out=$2; shift 2 ;; -w) w=$2; shift 2 ;; *) shift ;; esac; done
if [ -n "$out" ]; then echo stub > "$out"; else printf '{"prompt_id":"stub"}'; [ -z "$w" ] || printf '\n200'; fi"#,
    ),
    // Copies the `-i` input to the last argument.
    (
        "ffmpeg",
        r#"for a in "$@"; do last=$a; done
while [ $# -gt 0 ]; do case "$1" in -i) in=$2; shift 2 ;; *) shift ;; esac; done
cp "$in" "$last""#,
    ),
    // Writes <output_dir>/<input basename>.txt.
    (
        "whisper",
        r#"input=$1; dir=.
while [ $# -gt 0 ]; do case "$1" in --output_dir) dir=$2; shift 2 ;; *) shift ;; esac; done
base=$(basename "$input"); echo "stub transcript" > "$dir/${base%.*}.txt""#,
    ),
];

/// Runs `decree daemon` until every queued message has a finished run, then
/// stops it with SIGTERM, which is a clean exit (0).
const RUN_DAEMON: &str = r#"run_daemon() {
  decree daemon --interval 1 & local pid=$!
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
/// on PATH. `docker` lines are skipped: they need Docker and the network, and
/// the image is built from this repository's Dockerfile. `decree daemon` runs
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
            if line.starts_with("docker ") {
                continue;
            }
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

/// jq is a real dependency of the media examples' scripts, not a stand-in.
fn has_jq() -> bool {
    let found = Command::new("jq").arg("--version").output().is_ok();
    if !found {
        eprintln!("jq is not installed: skipping this example's README commands");
    }
    found
}

#[test]
fn business_eval_readme_commands_run() {
    let dir = run_readme("business-eval", "true");
    let project = dir.path().join("examples/business-eval");
    assert_eq!(
        processed(&project),
        [
            "01-petpulse.spec.md",
            "02-greenroute.spec.md",
            "03-studystream.spec.md"
        ]
    );
    // Three specs plus the extra idea, each a chain of four runs.
    assert_eq!(all_runs_done(&project), 16);
    let summaries = fs::read_dir(project.join(".decree/runs"))
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .path()
                .join("04-executive-summary.md")
                .is_file()
        })
        .count();
    assert_eq!(summaries, 4);
}

#[test]
fn docker_readme_commands_run() {
    let dir = run_readme("docker", "true");
    let project = dir.path().join("examples/docker");
    assert!(!project.join(".decree/runs").exists());
}

#[test]
fn text_to_media_readme_commands_run() {
    if !has_jq() {
        return;
    }
    let dir = run_readme("text-to-media", "true");
    let project = dir.path().join("examples/text-to-media");
    assert_eq!(processed(&project).len(), 4);
    assert_eq!(all_runs_done(&project), 5);
    let payload =
        fs::read_to_string(project.join(".decree/runs/04-animate-character/comfy-payload.json"))
            .unwrap();
    assert!(payload.contains("\"lily_waving\""), "{payload}");
    assert!(project
        .join(".decree/runs/04-animate-character/comfy-response.json")
        .is_file());
}

#[test]
fn text_to_speech_readme_commands_run() {
    if !has_jq() {
        return;
    }
    let dir = run_readme("text-to-speech", "true");
    let project = dir.path().join("examples/text-to-speech");
    assert_eq!(processed(&project), ["01-greeting.md", "02-narration.md"]);
    assert_eq!(all_runs_done(&project), 3);
    for file in ["greeting", "narration", "sentence"] {
        assert!(
            project.join(format!("output/{file}.mp3")).is_file(),
            "{file}"
        );
    }
}

#[test]
fn whisper_transcribe_readme_commands_run() {
    // The README's prerequisite: a recording at the path the migration names.
    let dir = run_readme(
        "whisper-transcribe",
        "mkdir -p audio && echo recording > audio/meeting-notes.mp3",
    );
    let project = dir.path().join("examples/whisper-transcribe");
    assert_eq!(processed(&project), ["01-transcribe-sample.md"]);
    assert_eq!(all_runs_done(&project), 2);
    assert!(project.join("audio/meeting-notes.txt").is_file());
}
