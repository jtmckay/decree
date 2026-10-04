//! `examples/route-by-complexity/`: `develop_by_size` run through the binary down each of
//! its five paths. The example's `.decree/` is copied to a temp project whose
//! `gliner_router/ask_gliner.sh` is a stub that writes a chosen reply, with stub `opencode`,
//! `claude` and test commands first on `PATH`. No model, no network. Also: the request decree
//! writes carries a `reply_schema` over the options, the GLiNER quick starts are at most five
//! commands, the server code is in one file, and it byte-compiles.

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

mod common;
use common::write_script;

const RUN: &str = "01-raise-upload-limit";

/// Writes the reply the test chose (`stub/reply.json`), as the real script writes the
/// server's answer, and ends with a plain line.
const STUB_ASK_GLINER: &str = r#"#!/usr/bin/env bash
cp "$DECREE_PROJECT_ROOT/stub/reply.json" "$DECREE_REPLY"
echo "picked (stub)"
"#;

/// Records the call in `calls`, and exits `$<TOOL>_EXIT` (default 0).
fn stub_tool(tool: &str) -> String {
    let var = format!("{}_EXIT", tool.to_uppercase());
    format!(
        "#!/usr/bin/env bash\necho {tool} >> \"$(dirname \"$0\")/calls\"\nexit \"${{{var}:-0}}\"\n"
    )
}

/// The test command (`TEST_CMD`): its n-th call exits with the n-th line of
/// `stub/test_results`.
const STUB_TESTS: &str = r#"#!/usr/bin/env bash
dir="$DECREE_PROJECT_ROOT/stub"
n=$(( $(cat "$dir/test_count" 2>/dev/null || echo 0) + 1 ))
echo "$n" > "$dir/test_count"
echo tests >> "$(dirname "$0")/calls"
exit "$(sed -n "${n}p" "$dir/test_results")"
"#;

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/route-by-complexity")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap();
        }
    }
}

struct Project {
    tmp: TempDir,
}

impl Project {
    /// The example's project, the classifier's reply, and the exit codes of the test
    /// command's calls, in order.
    fn new(reply: &str, test_results: &[i32]) -> Project {
        let p = Project {
            tmp: TempDir::new().unwrap(),
        };
        copy_dir(&example().join(".decree"), &p.root().join(".decree"));
        fs::create_dir(p.root().join("src")).unwrap();
        let config: String = (1..=12).map(|i| format!("// line {i}\n")).collect();
        fs::write(p.root().join("src/config.rs"), config).unwrap();
        fs::create_dir(p.root().join("stub")).unwrap();
        fs::write(p.root().join("stub/reply.json"), reply).unwrap();
        let results: String = test_results.iter().map(|c| format!("{c}\n")).collect();
        fs::write(p.root().join("stub/test_results"), results).unwrap();
        write_script(
            &p.root().join(".decree/scripts/gliner_router/ask_gliner.sh"),
            STUB_ASK_GLINER,
        );
        fs::create_dir(p.bin()).unwrap();
        for tool in ["opencode", "claude"] {
            write_script(&p.bin().join(tool), &stub_tool(tool));
        }
        write_script(&p.bin().join("stub-tests"), STUB_TESTS);
        p
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    fn bin(&self) -> PathBuf {
        self.root().join("bin")
    }

    /// `decree process` with the stubs first on `PATH` and `TEST_CMD` the stub test command;
    /// it exits 0, or 1 when the migration ends in `failed`.
    fn process(&self, code: i32) {
        let path = std::env::join_paths(
            std::iter::once(self.bin())
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        cargo_bin_cmd!("decree")
            .current_dir(self.root())
            .env("PATH", path)
            .env("TEST_CMD", self.bin().join("stub-tests"))
            .env("NO_COLOR", "1")
            .arg("process")
            .assert()
            .code(code);
    }

    fn run_dir(&self, id: &str) -> PathBuf {
        self.root().join(".decree/runs").join(id)
    }

    fn events(&self, id: &str) -> Vec<Value> {
        fs::read_to_string(self.run_dir(id).join("events.jsonl"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// The run's path: `<from> <event> <to>` for each transition after the claim.
    fn path(&self) -> Vec<String> {
        self.events(RUN)
            .iter()
            .filter(|e| e["type"] == "transition" && e["source"] != "claim")
            .map(|e| format!("{} {} {}", e["from"], e["event"], e["to"]).replace('"', ""))
            .collect()
    }

    fn final_state(&self) -> String {
        let message = fs::read_to_string(self.run_dir(RUN).join("message.md")).unwrap();
        message
            .lines()
            .find_map(|l| l.strip_prefix("state: "))
            .unwrap()
            .to_string()
    }

    /// The stub tools called, in order.
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.bin().join("calls"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    /// The `request.json` of the router run `size_up` waited for.
    fn request(&self) -> Value {
        let child = self
            .events(RUN)
            .into_iter()
            .find(|e| e["type"] == "waiting")
            .unwrap()["child"]
            .as_str()
            .unwrap()
            .to_string();
        serde_json::from_str(
            &fs::read_to_string(self.run_dir(&child).join("request.json")).unwrap(),
        )
        .unwrap()
    }
}

const SMALL: &str = r#"{"event": "small", "confidence": 0.93}"#;
const LARGE: &str = r#"{"event": "large", "confidence": 0.88}"#;
const SMALL_BUT_UNSURE: &str = r#"{"event": "small", "confidence": 0.55}"#;

#[test]
fn small_is_implemented_locally_and_done() {
    let p = Project::new(SMALL, &[0]);
    p.process(0);
    assert_eq!(
        p.path(),
        [
            "describe done size_up",
            "size_up small implement_local",
            "implement_local done verify",
            "verify done done",
        ]
    );
    assert_eq!(p.final_state(), "done");
    assert_eq!(p.calls(), ["opencode", "tests"]);
}

#[test]
fn large_is_implemented_by_claude_and_done() {
    let p = Project::new(LARGE, &[0]);
    p.process(0);
    assert_eq!(
        p.path(),
        [
            "describe done size_up",
            "size_up large implement_claude",
            "implement_claude done verify",
            "verify done done",
        ]
    );
    assert_eq!(p.final_state(), "done");
    assert_eq!(p.calls(), ["claude", "tests"]);
}

#[test]
fn unsure_goes_to_claude() {
    let p = Project::new(SMALL_BUT_UNSURE, &[0]);
    p.process(0);
    assert_eq!(
        p.path()[..3],
        [
            "describe done size_up",
            "size_up unsure implement_claude",
            "implement_claude done verify",
        ]
    );
    assert_eq!(p.final_state(), "done");
    assert_eq!(p.calls(), ["claude", "tests"]);
}

#[test]
fn small_that_fails_verify_escalates_to_claude_once_and_is_done() {
    let p = Project::new(SMALL, &[1, 0]);
    p.process(0);
    assert_eq!(
        p.path(),
        [
            "describe done size_up",
            "size_up small implement_local",
            "implement_local done verify",
            "verify error tried_claude",
            "tried_claude true implement_claude",
            "implement_claude done verify",
            "verify done done",
        ]
    );
    assert_eq!(p.final_state(), "done");
    assert_eq!(p.calls(), ["opencode", "tests", "claude", "tests"]);
}

#[test]
fn small_that_fails_locally_and_with_claude_is_failed() {
    let p = Project::new(SMALL, &[1, 1]);
    p.process(1);
    assert_eq!(
        p.path(),
        [
            "describe done size_up",
            "size_up small implement_local",
            "implement_local done verify",
            "verify error tried_claude",
            "tried_claude true implement_claude",
            "implement_claude done verify",
            "verify error tried_claude",
            "tried_claude false failed",
        ]
    );
    assert_eq!(p.final_state(), "failed");
    assert_eq!(p.calls(), ["opencode", "tests", "claude", "tests"]);
}

/// The classifier reads the title, the acceptance criteria and the size of the files the
/// message names, and the request's `reply_schema` allows exactly the options.
#[test]
fn the_classifier_reads_a_size_and_the_request_has_a_reply_schema() {
    let p = Project::new(SMALL, &[0]);
    p.process(0);
    let request = p.request();
    let input = request["input"].as_str().unwrap();
    for want in [
        "# Raise the upload limit to 25 MB",
        "## Acceptance Criteria",
        "    12 src/config.rs",
        "Total: 12 lines in 1 files",
    ] {
        assert!(input.contains(want), "{want:?} not in:\n{input}");
    }
    let options: Vec<&Value> = request["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| &o["event"])
        .collect();
    let schema = &request["reply_schema"];
    assert_eq!(
        schema["properties"]["event"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .collect::<Vec<_>>(),
        options
    );
    let validator = jsonschema::draft202012::new(schema).unwrap();
    let reply: Value = serde_json::from_str(SMALL).unwrap();
    assert!(validator.is_valid(&reply));
    assert!(!validator.is_valid(&serde_json::json!({"event": "medium"})));
}

/// `gliner/decide_server.py` is valid Python. Skipped, with a note, without `python3`.
#[test]
fn the_gliner_server_byte_compiles() {
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("python3 is not on PATH: skipping the decide_server.py byte-compile");
        return;
    }
    let cache = TempDir::new().unwrap();
    let out = Command::new("python3")
        .args(["-m", "py_compile"])
        .arg(example().join("gliner/decide_server.py"))
        .env("PYTHONPYCACHEPREFIX", cache.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn repo() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The lines of the fenced `sh` block in `text` that starts the GLiNER server.
fn quick_start(text: &str) -> Vec<&str> {
    let mut blocks = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    for line in text.lines() {
        match current.as_mut() {
            None if line == "```sh" => current = Some(Vec::new()),
            None => {}
            Some(_) if line == "```" => blocks.push(current.take().unwrap()),
            Some(block) => block.push(line),
        }
    }
    blocks
        .into_iter()
        .find(|b| b.iter().any(|l| l.contains("decide_server.py")))
        .expect("no sh block starts decide_server.py")
}

/// The quick start in `docs/routers.md` and in the example's README is at most five
/// commands: install, start (which downloads the model), and one `curl`.
#[test]
fn the_gliner_quick_start_is_at_most_five_commands() {
    for doc in ["docs/routers.md", "examples/route-by-complexity/README.md"] {
        let text = fs::read_to_string(repo().join(doc)).unwrap();
        let commands: Vec<&str> = quick_start(&text)
            .into_iter()
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
            .collect();
        assert!(commands.len() <= 5, "{doc}: {commands:?}");
        for word in [
            "pip install 'gliner2[local]'",
            "python gliner/decide_server.py",
            "curl ",
        ] {
            assert!(
                commands.iter().any(|c| c.starts_with(word)),
                "{doc}: no `{word}` in {commands:?}"
            );
        }
        assert!(
            text.contains("docs/services.md") || text.contains("services.md#"),
            "{doc}"
        );
    }
}

/// Every file under `dir`, skipping build output, git and this repository's own `.decree/`.
fn repo_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let rel = path.strip_prefix(repo()).unwrap();
        if [".git", "target", ".decree"]
            .iter()
            .any(|s| rel == Path::new(s))
        {
            continue;
        }
        if path.is_dir() {
            repo_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// The GLiNER server's code is in one file: no doc or example inlines it.
#[test]
fn the_gliner_server_code_is_in_exactly_one_file() {
    let mut files = Vec::new();
    repo_files(repo(), &mut files);
    let server: Vec<PathBuf> = files
        .into_iter()
        .filter(|p| p != &repo().join(file!()))
        .filter(|p| fs::read_to_string(p).is_ok_and(|t| t.contains("BaseHTTPRequestHandler")))
        .map(|p| p.strip_prefix(repo()).unwrap().to_path_buf())
        .collect();
    assert_eq!(
        server,
        [PathBuf::from(
            "examples/route-by-complexity/gliner/decide_server.py"
        )]
    );
}
