//! `examples/tmux-services/`: `illustrated_post` run through the binary with stub `tmux` and
//! `curl` first on `PATH` and a stub `ask_gliner.sh`. No tmux, no service, no network.
//!
//! The stub `tmux` keeps sessions as files in `stub/sessions/` and logs each call to
//! `stub/tmux.log`. The stub `curl` answers a service's URLs while its stub session exists
//! (unless `stub/never/<service>` exists), or always when `stub/outside/<service>` exists: a
//! service running outside tmux. It answers ComfyUI's `/prompt` and `/history/<id>` (`{}`
//! the first time, as ComfyUI does until the prompt has finished) and Ollama's
//! `/api/generate`. The example sits at `examples/tmux-services/` in a temp directory next
//! to the two files it reuses by path, so its defaults resolve as in the repository.

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

mod common;
use common::write_script;

const STUB_TMUX: &str = r#"#!/usr/bin/env bash
cmd=$1; shift; target=""; name=""; command=""
while [ $# -gt 0 ]; do
  case "$1" in -t) target=${2#=}; shift 2 ;; -s) name=$2; shift 2 ;; -*) shift ;; *) command=$1; shift ;; esac
done
echo "$cmd ${target:-$name}" >> "$STUB/tmux.log"
case "$cmd" in
  has-session) [ -f "$STUB/sessions/$target" ] ;;
  new-session) printf '%s\n' "$command" > "$STUB/sessions/$name" ;;
  kill-session) [ -f "$STUB/sessions/$target" ] && rm "$STUB/sessions/$target" ;;
  *) exit 1 ;;
esac
"#;

const STUB_CURL: &str = r#"#!/usr/bin/env bash
url=""; data=""
while [ $# -gt 0 ]; do
  case "$1" in -d) data=$2; shift 2 ;; -H|--max-time) shift 2 ;; http*) url=$1; shift ;; *) shift ;; esac
done
case "$url" in
  *:8090/*) service=gliner ;; *:11434/*) service=ollama ;; *:8188/*) service=comfyui ;; *) exit 6 ;;
esac
if [ ! -f "$STUB/outside/$service" ] && { [ ! -f "$STUB/sessions/$service" ] || [ -f "$STUB/never/$service" ]; }; then
  echo "curl: (7) Failed to connect to $url" >&2
  exit 7
fi
case "$url" in
  */prompt)
    cp "${data#@}" "$STUB/comfy-payload.json"
    echo '{"prompt_id": "p1", "number": 0, "node_errors": {}}' ;;
  */history/p1)
    if [ -f "$STUB/history-asked" ]; then
      echo '{"p1": {"outputs": {"9": {"images": [{"filename": "decree_00001_.png", "subfolder": "", "type": "output"}]}}, "status": {"status_str": "success", "completed": true, "messages": []}}}'
    else
      touch "$STUB/history-asked"
      echo '{}'
    fi ;;
  */api/generate)
    printf '%s\n' "$data" > "$STUB/generate.json"
    echo '{"model": "stub", "response": "A stub post.", "done": true}' ;;
  *) echo '{"ok": true}' ;;
esac
"#;

/// Writes the reply the test chose, as the real script writes the server's answer.
const STUB_ASK_GLINER: &str = r#"#!/usr/bin/env bash
cp "$STUB/reply.json" "$DECREE_REPLY"
echo "picked (stub)"
"#;

const WITH_PICTURE: &str = r#"{"event": "with_picture", "confidence": 0.92}"#;
const TEXT_ONLY: &str = r#"{"event": "text_only", "confidence": 0.9}"#;

fn repo() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
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

/// jq is a real dependency of the example's scripts, not a stand-in.
fn has_jq() -> bool {
    let found = Command::new("jq").arg("--version").output().is_ok();
    if !found {
        eprintln!("jq is not installed: skipping the tmux-services runs");
    }
    found
}

struct Project {
    tmp: TempDir,
}

impl Project {
    /// The example, the files it reuses by path, the stubs, and the classifier's reply.
    fn new(reply: &str) -> Project {
        let p = Project {
            tmp: TempDir::new().unwrap(),
        };
        let examples = p.tmp.path().join("examples");
        copy_dir(
            &repo().join("examples/tmux-services/.decree"),
            &p.root().join(".decree"),
        );
        for file in [
            "route-by-complexity/gliner/decide_server.py",
            "text-to-media/workflows/image_flux2_text_landscape.json",
        ] {
            let to = examples.join(file);
            fs::create_dir_all(to.parent().unwrap()).unwrap();
            fs::copy(repo().join("examples").join(file), to).unwrap();
        }
        fs::create_dir_all(p.stub().join("sessions")).unwrap();
        fs::write(p.stub().join("reply.json"), reply).unwrap();
        write_script(
            &p.root().join(".decree/scripts/gliner_router/ask_gliner.sh"),
            STUB_ASK_GLINER,
        );
        fs::create_dir(p.bin()).unwrap();
        write_script(&p.bin().join("tmux"), STUB_TMUX);
        write_script(&p.bin().join("curl"), STUB_CURL);
        p
    }

    fn root(&self) -> PathBuf {
        self.tmp.path().join("examples/tmux-services")
    }

    fn stub(&self) -> PathBuf {
        self.tmp.path().join("stub")
    }

    fn bin(&self) -> PathBuf {
        self.tmp.path().join("bin")
    }

    /// A stub session that is already running.
    fn session(&self, name: &str) {
        fs::write(self.stub().join("sessions").join(name), "started by hand\n").unwrap();
    }

    /// A marker file `stub/<kind>/<service>` (`never` or `outside`).
    fn mark(&self, kind: &str, service: &str) {
        fs::create_dir_all(self.stub().join(kind)).unwrap();
        fs::write(self.stub().join(kind).join(service), "").unwrap();
    }

    /// `decree emit --machine illustrated_post` with `body`, then `decree process` with the
    /// stubs first on `PATH` and `env` set; returns the run's directory.
    fn run(&self, body: &str, env: &[(&str, &str)]) -> PathBuf {
        cargo_bin_cmd!("decree")
            .current_dir(self.root())
            .args(["emit", "--machine", "illustrated_post"])
            .write_stdin(body)
            .assert()
            .success();
        let path = std::env::join_paths(
            std::iter::once(self.bin())
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        let mut cmd = cargo_bin_cmd!("decree");
        cmd.current_dir(self.root())
            .env("PATH", path)
            .env("STUB", self.stub())
            .env("COMFYUI_DIR", "/opt/ComfyUI")
            .env("NO_COLOR", "1")
            .arg("process");
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().unwrap();
        fs::read_dir(self.root().join(".decree/runs"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|d| {
                fs::read_to_string(d.join("message.md"))
                    .unwrap()
                    .lines()
                    .any(|l| l == "machine: illustrated_post")
            })
            .expect("no illustrated_post run")
    }

    /// The stub tmux calls that start or end a session, in order.
    fn switches(&self) -> Vec<String> {
        self.tmux_log()
            .into_iter()
            .filter(|l| !l.starts_with("has-session"))
            .collect()
    }

    fn tmux_log(&self) -> Vec<String> {
        fs::read_to_string(self.stub().join("tmux.log"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }
}

/// The run's path: `<from> <event> <to>` for each transition after the claim.
fn path(run: &Path) -> Vec<String> {
    fs::read_to_string(run.join("events.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|e| e["type"] == "transition" && e["source"] != "claim")
        .map(|e| format!("{} {} {}", e["from"], e["event"], e["to"]).replace('"', ""))
        .collect()
}

fn final_state(run: &Path) -> String {
    fs::read_to_string(run.join("message.md"))
        .unwrap()
        .lines()
        .find_map(|l| l.strip_prefix("state: "))
        .unwrap()
        .to_string()
}

/// The log of the run's script whose file name ends in `suffix`.
fn log(run: &Path, suffix: &str) -> String {
    let file = fs::read_dir(run)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().ends_with(suffix))
        .unwrap_or_else(|| panic!("no *{suffix} in {}", run.display()));
    fs::read_to_string(file).unwrap()
}

const PICTURE_POST: &str = "A post about lighthouses at dawn, with a picture of one.\n";

#[test]
fn with_picture_starts_gliner_then_switches_the_gpu_from_comfyui_to_ollama() {
    if !has_jq() {
        return;
    }
    let p = Project::new(WITH_PICTURE);
    let run = p.run(PICTURE_POST, &[]);
    assert_eq!(
        path(&run),
        [
            "needs_picture with_picture render",
            "render done write",
            "write done done"
        ]
    );
    assert_eq!(final_state(&run), "done");
    // No sessions at the start: ollama is absent when comfyui starts, so nothing ends.
    assert_eq!(
        p.switches(),
        [
            "new-session gliner",
            "new-session comfyui",
            "kill-session comfyui",
            "new-session ollama"
        ]
    );
    // GLiNER runs the one copy of the server, by its path next to this example.
    let gliner = fs::read_to_string(p.stub().join("sessions/gliner")).unwrap();
    assert!(
        gliner.starts_with("python3 ")
            && gliner
                .trim_end()
                .ends_with("/../route-by-complexity/gliner/decide_server.py"),
        "{gliner}"
    );
    let comfyui = fs::read_to_string(p.stub().join("sessions/comfyui"));
    assert!(comfyui.is_err(), "comfyui still running");
    let ollama = fs::read_to_string(p.stub().join("sessions/ollama")).unwrap();
    assert_eq!(ollama, "ollama serve\n");

    // The workflow carries the message as its prompt, and render waited for /history.
    let payload: Value =
        serde_json::from_str(&fs::read_to_string(p.stub().join("comfy-payload.json")).unwrap())
            .unwrap();
    assert_eq!(
        payload["prompt"]["6"]["inputs"]["text"],
        PICTURE_POST.trim_end()
    );
    assert!(p.stub().join("history-asked").is_file());
    assert_eq!(
        fs::read_to_string(run.join("image.txt")).unwrap(),
        "/opt/ComfyUI/output/decree_00001_.png\n"
    );
    let generate: Value =
        serde_json::from_str(&fs::read_to_string(p.stub().join("generate.json")).unwrap()).unwrap();
    assert_eq!(generate["model"], "gemma4:e4b");
    assert_eq!(generate["stream"], false);
    assert_eq!(
        fs::read_to_string(run.join("post.md")).unwrap(),
        "A stub post.\n\n![picture](/opt/ComfyUI/output/decree_00001_.png)\n"
    );
}

#[test]
fn starting_comfyui_ends_a_running_ollama_first() {
    if !has_jq() {
        return;
    }
    let p = Project::new(WITH_PICTURE);
    p.session("ollama");
    let run = p.run(PICTURE_POST, &[]);
    assert_eq!(final_state(&run), "done");
    assert_eq!(
        p.switches(),
        [
            "new-session gliner",
            "kill-session ollama",
            "new-session comfyui",
            "kill-session comfyui",
            "new-session ollama"
        ]
    );
}

#[test]
fn text_only_never_touches_comfyui() {
    if !has_jq() {
        return;
    }
    let p = Project::new(TEXT_ONLY);
    let run = p.run("A post about our new release.\n", &[]);
    assert_eq!(
        path(&run),
        ["needs_picture text_only write", "write done done"]
    );
    assert_eq!(final_state(&run), "done");
    assert_eq!(p.switches(), ["new-session gliner", "new-session ollama"]);
    // use_ollama only asks whether a comfyui session runs, and ComfyUI's health URL.
    assert!(!p.stub().join("comfy-payload.json").exists());
    assert_eq!(
        fs::read_to_string(run.join("post.md")).unwrap(),
        "A stub post.\n"
    );
}

#[test]
fn a_running_gliner_session_is_reused() {
    if !has_jq() {
        return;
    }
    let p = Project::new(TEXT_ONLY);
    p.session("gliner");
    let run = p.run("A post about our new release.\n", &[]);
    assert_eq!(final_state(&run), "done");
    assert_eq!(p.tmux_log()[0], "has-session gliner");
    assert_eq!(p.switches(), ["new-session ollama"]);
    assert_eq!(
        fs::read_to_string(p.stub().join("sessions/gliner")).unwrap(),
        "started by hand\n"
    );
    assert!(log(&run, "-_root-use_gliner.log").contains("using the running tmux session gliner"));
}

#[test]
fn a_service_that_never_answers_fails_its_onentry_and_the_run() {
    if !has_jq() {
        return;
    }
    let p = Project::new(WITH_PICTURE);
    p.mark("never", "comfyui");
    let run = p.run(PICTURE_POST, &[("COMFYUI_START_TIMEOUT_S", "2")]);
    assert_eq!(
        path(&run),
        ["needs_picture with_picture render", "render error failed"]
    );
    assert_eq!(final_state(&run), "failed");
    let log = log(&run, "-render-use_comfyui.log");
    for want in [
        "[stderr] http://127.0.0.1:8188/system_stats did not answer within 2 s",
        "[stderr] comfyui did not start; see why with: tmux attach -t comfyui",
    ] {
        assert!(log.contains(want), "{want:?} not in:\n{log}");
    }
    assert!(!p.stub().join("comfy-payload.json").exists());
}

#[test]
fn a_service_still_answering_after_its_session_ended_runs_outside_tmux() {
    if !has_jq() {
        return;
    }
    let p = Project::new(WITH_PICTURE);
    p.mark("outside", "ollama");
    let run = p.run(PICTURE_POST, &[("TMUX_END_TIMEOUT_S", "2")]);
    assert_eq!(final_state(&run), "failed");
    assert_eq!(p.switches(), ["new-session gliner"]);
    let log = log(&run, "-render-use_comfyui.log");
    for want in [
        "ollama still answers at http://127.0.0.1:11434/api/version 2 s after its tmux session ended",
        "so it is running outside tmux",
        "sudo systemctl stop ollama",
    ] {
        assert!(log.contains(want), "{want:?} not in:\n{log}");
    }
}
