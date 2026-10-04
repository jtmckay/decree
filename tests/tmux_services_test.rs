//! `examples/tmux-services/`: `illustrated_post` run through the binary with stub `tmux` and
//! `curl` first on `PATH` and a stub `ask_gliner.sh`. No tmux, no service, no network.
//!
//! The stub `tmux` keeps sessions as files in `stub/sessions/` and logs each call to
//! `stub/tmux.log`. The stub `curl` answers a service's URLs while its stub session exists
//! (unless `stub/never/<service>` exists), or always when `stub/outside/<service>` exists: a
//! service running outside tmux. It logs each request it answers to `stub/requests.log` as
//! `<method> /<path> <body>`. It keeps "loaded" models as files: ComfyUI's `/prompt` writes
//! `stub/loaded/comfyui` and `/free` removes it; Ollama's plain `/api/generate` writes
//! `stub/loaded/ollama` and one with `keep_alive: 0` removes it, unless `stub/stuck/<service>`
//! exists. `/api/ps` and `/system_stats` report from those files. It answers ComfyUI's `GET
//! /queue` (one job running the first time, then empty) and `/history/<id>`
//! (`stub/history.json` if the test wrote one, else a finished image). The example sits at
//! `examples/tmux-services/` in a temp directory next to the two files it reuses by path, so
//! its defaults resolve as in the repository.

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
method=GET; [ -n "$data" ] && method=POST
echo "$method /${url#http://*/} $data" >> "$STUB/requests.log"
mkdir -p "$STUB/loaded"
case "$method $url" in
  "POST "*/prompt)
    cp "${data#@}" "$STUB/comfy-payload.json"
    touch "$STUB/loaded/comfyui"
    echo '{"prompt_id": "p1", "number": 0, "node_errors": {}}' ;;
  "GET "*/queue)
    if [ -f "$STUB/queue-asked" ]; then
      echo '{"queue_running": [], "queue_pending": []}'
    else
      touch "$STUB/queue-asked"
      echo '{"queue_running": [[0, "p1", {}, {}, []]], "queue_pending": []}'
    fi ;;
  "POST "*/free)
    [ -f "$STUB/stuck/comfyui" ] || rm -f "$STUB/loaded/comfyui" ;;
  *"/system_stats")
    reserved=0; [ -f "$STUB/loaded/comfyui" ] && reserved=12884901888
    echo "{\"devices\": [{\"name\": \"cuda:0 Stub GPU : cudaMallocAsync\", \"type\": \"cuda\", \"index\": 0, \"torch_vram_total\": $reserved}]}" ;;
  */history/p1)
    if [ -f "$STUB/history.json" ]; then
      cat "$STUB/history.json"
    else
      echo '{"p1": {"outputs": {"9": {"images": [{"filename": "decree_00001_.png", "subfolder": "", "type": "output"}]}}, "status": {"status_str": "success", "completed": true, "messages": []}}}'
    fi ;;
  *"/api/ps")
    if [ -f "$STUB/loaded/ollama" ]; then
      echo '{"models": [{"name": "gemma4:e4b", "model": "gemma4:e4b", "size_vram": 9600000000}]}'
    else
      echo '{"models": []}'
    fi ;;
  "POST "*/api/generate)
    if jq -e '.keep_alive == 0' <<<"$data" >/dev/null; then
      [ -f "$STUB/stuck/ollama" ] || rm -f "$STUB/loaded/ollama"
      echo '{"model": "gemma4:e4b", "response": "", "done": true, "done_reason": "unload"}'
    else
      printf '%s\n' "$data" > "$STUB/generate.json"
      touch "$STUB/loaded/ollama"
      echo '{"model": "stub", "response": "A stub post.", "done": true}'
    fi ;;
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

    /// A marker file `stub/<kind>/<service>` (`never`, `outside`, `loaded` or `stuck`).
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

    /// The stub tmux calls other than `has-session`, in order.
    fn switches(&self) -> Vec<String> {
        self.tmux_log()
            .into_iter()
            .filter(|l| !l.starts_with("has-session"))
            .collect()
    }

    /// The requests the stub curl answered, as `<method> /<path> <body>`, in order.
    fn requests(&self) -> Vec<String> {
        fs::read_to_string(self.stub().join("requests.log"))
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim_end().to_string())
            .collect()
    }

    /// Where the first request starting with `prefix` is in `requests()`.
    fn request_at(&self, prefix: &str) -> usize {
        let requests = self.requests();
        requests
            .iter()
            .position(|r| r.starts_with(prefix))
            .unwrap_or_else(|| panic!("no {prefix:?} in {requests:#?}"))
    }

    fn loaded(&self, service: &str) -> bool {
        self.stub().join("loaded").join(service).exists()
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
const FREE: &str = r#"POST /free {"unload_models": true, "free_memory": true}"#;
const OLLAMA_UNLOAD: &str = r#"POST /api/generate {"model":"gemma4:e4b","keep_alive":0}"#;

#[test]
fn with_picture_starts_each_service_once_and_unloads_comfyui_before_ollama() {
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
    // Each session started once, none killed, and all still exist.
    assert_eq!(
        p.switches(),
        [
            "new-session gliner",
            "new-session comfyui",
            "new-session ollama"
        ]
    );
    for session in ["gliner", "comfyui", "ollama"] {
        assert!(
            p.stub().join("sessions").join(session).is_file(),
            "{session}"
        );
    }
    // GLiNER runs the one copy of the server, by its path next to this example.
    let gliner = fs::read_to_string(p.stub().join("sessions/gliner")).unwrap();
    assert!(
        gliner.starts_with("python3 ")
            && gliner
                .trim_end()
                .ends_with("/../route-by-complexity/gliner/decide_server.py"),
        "{gliner}"
    );
    assert_eq!(
        fs::read_to_string(p.stub().join("sessions/ollama")).unwrap(),
        "ollama serve\n"
    );
    // Ollama was not running at render, so there was nothing to unload.
    assert!(log(&run, "-render-without_ollama.log").contains("nothing to unload"));

    // ComfyUI's model was unloaded through /free, with exactly that body, after its queue
    // drained and before Ollama's request; Ollama's model is the one still loaded.
    assert!(p.request_at("GET /queue") < p.request_at(FREE));
    assert!(p.request_at(FREE) < p.request_at(r#"POST /api/generate {"#));
    assert!(!p.loaded("comfyui"));
    assert!(p.loaded("ollama"));

    // The workflow carries the message as its prompt; render only queued it.
    let payload: Value =
        serde_json::from_str(&fs::read_to_string(p.stub().join("comfy-payload.json")).unwrap())
            .unwrap();
    assert_eq!(
        payload["prompt"]["6"]["inputs"]["text"],
        PICTURE_POST.trim_end()
    );
    assert_eq!(
        fs::read_to_string(run.join("comfy-prompts.txt")).unwrap(),
        "p1\n"
    );
    assert_eq!(
        fs::read_to_string(run.join("images.txt")).unwrap(),
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

/// A second picture run: Ollama still holds the model from the first.
#[test]
fn without_ollama_unloads_a_loaded_model_with_keep_alive_0_before_the_prompt() {
    if !has_jq() {
        return;
    }
    let p = Project::new(WITH_PICTURE);
    p.session("ollama");
    p.mark("loaded", "ollama");
    let run = p.run(PICTURE_POST, &[]);
    assert_eq!(final_state(&run), "done");
    let unload = p.request_at(OLLAMA_UNLOAD);
    // It polled /api/ps after the unload, until it listed none, all before ComfyUI's prompt.
    let polled = p.requests()[unload..]
        .iter()
        .position(|r| r.starts_with("GET /api/ps"))
        .map(|i| i + unload)
        .expect("no /api/ps after the unload");
    assert!(polled < p.request_at("POST /prompt"));
    assert!(log(&run, "-render-without_ollama.log").contains("ollama has no model loaded"));
    // Ollama's session was never ended, so use_ollama found it answering.
    assert_eq!(p.switches(), ["new-session gliner", "new-session comfyui"]);
    assert!(log(&run, "-write-use_ollama.log").contains("ollama already answers"));
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
    // ComfyUI does not answer, so without_comfy_wait has nothing to wait for or unload.
    assert!(!p.stub().join("comfy-payload.json").exists());
    assert!(log(&run, "-write-without_comfy_wait.log").contains("comfyui is not running"));
    assert_eq!(
        fs::read_to_string(run.join("post.md")).unwrap(),
        "A stub post.\n"
    );
}

/// GLiNER in a session started by hand, Ollama outside tmux (its own systemd service, say):
/// both answer, so both are used as they are.
#[test]
fn a_service_that_already_answers_is_used_and_no_session_is_started() {
    if !has_jq() {
        return;
    }
    let p = Project::new(TEXT_ONLY);
    p.session("gliner");
    p.mark("outside", "ollama");
    let run = p.run("A post about our new release.\n", &[]);
    assert_eq!(final_state(&run), "done");
    assert_eq!(p.tmux_log(), Vec::<String>::new());
    assert!(!p.stub().join("sessions/ollama").exists());
    assert_eq!(
        fs::read_to_string(p.stub().join("sessions/gliner")).unwrap(),
        "started by hand\n"
    );
    assert!(log(&run, "-_root-use_gliner.log")
        .contains("gliner already answers at http://127.0.0.1:8090/health"));
    assert!(log(&run, "-write-use_ollama.log")
        .contains("ollama already answers at http://127.0.0.1:11434/api/version"));
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
    let log = log(&run, "-render-use_comfy.log");
    for want in [
        "[stderr] http://127.0.0.1:8188/system_stats did not answer within 2 s",
        "[stderr] comfyui did not start; see why with: tmux attach -t comfyui",
    ] {
        assert!(log.contains(want), "{want:?} not in:\n{log}");
    }
    assert!(!p.stub().join("comfy-payload.json").exists());
}

/// An unload that never takes effect fails the state, naming the service and what it still
/// holds; the next service is never started.
#[test]
fn an_unload_that_never_takes_effect_fails_the_state() {
    if !has_jq() {
        return;
    }
    // Ollama keeps its model: render fails before use_comfy.
    let p = Project::new(WITH_PICTURE);
    p.session("ollama");
    p.mark("loaded", "ollama");
    p.mark("stuck", "ollama");
    let run = p.run(PICTURE_POST, &[("OLLAMA_UNLOAD_TIMEOUT_S", "2")]);
    assert_eq!(final_state(&run), "failed");
    assert_eq!(path(&run).last().unwrap(), "render error failed");
    let text = log(&run, "-render-without_ollama.log");
    let want = "[stderr] ollama: gemma4:e4b still loaded 2 s after keep_alive: 0";
    assert!(text.contains(want), "{want:?} not in:\n{text}");
    assert_eq!(p.switches(), ["new-session gliner"]);

    // ComfyUI keeps its model: write fails before use_ollama.
    let p = Project::new(WITH_PICTURE);
    p.mark("stuck", "comfyui");
    let run = p.run(PICTURE_POST, &[("COMFY_UNLOAD_TIMEOUT_S", "2")]);
    assert_eq!(final_state(&run), "failed");
    assert_eq!(path(&run).last().unwrap(), "write error failed");
    let text = log(&run, "-write-without_comfy_wait.log");
    let want = "[stderr] comfyui: still cuda:0 Stub GPU : cudaMallocAsync: 12288 MB reserved \
                (more than 1024 MB) 2 s after /free";
    assert!(text.contains(want), "{want:?} not in:\n{text}");
    assert_eq!(p.switches(), ["new-session gliner", "new-session comfyui"]);
}

/// `without_comfy_wait` fails `write`'s onentry, before anything is unloaded, when this run's
/// prompt failed or ComfyUI lost it.
#[test]
fn a_failed_or_lost_prompt_fails_without_comfy_wait_before_anything_is_unloaded() {
    if !has_jq() {
        return;
    }
    for (history, expected) in [
        (
            r#"{"p1": {"outputs": {}, "status": {"status_str": "error", "completed": false, "messages": []}}}"#,
            "comfyui: prompt p1 failed",
        ),
        (
            "{}",
            "comfyui: prompt p1 is neither queued nor in its history: it was lost",
        ),
    ] {
        let p = Project::new(WITH_PICTURE);
        fs::write(p.stub().join("history.json"), history).unwrap();
        let run = p.run(PICTURE_POST, &[]);
        assert_eq!(final_state(&run), "failed");
        assert_eq!(path(&run).last().unwrap(), "write error failed");
        assert!(
            log(&run, "-write-without_comfy_wait.log").contains(expected),
            "{history}"
        );
        // Nothing was cleared, interrupted or unloaded, and use_ollama never ran.
        let requests = p.requests();
        assert!(
            !requests.iter().any(|r| r.starts_with("POST /queue")
                || r.starts_with("POST /interrupt")
                || r.starts_with("POST /free")),
            "{history}: {requests:#?}"
        );
        assert!(p.loaded("comfyui"), "{history}");
        assert_eq!(
            p.switches(),
            ["new-session gliner", "new-session comfyui"],
            "{history}"
        );
    }
}

/// `without_comfy_no_wait` clears the queue and interrupts before `/free`, never waits for the
/// queue, and leaves the server running; with ComfyUI down it has nothing to do.
#[test]
fn without_comfy_no_wait_clears_and_interrupts_before_free() {
    if !has_jq() {
        return;
    }
    let p = Project::new(TEXT_ONLY);
    let script = p.root().join(".decree/scripts/without_comfy_no_wait.sh");
    let run = || {
        let out = Command::new(&script)
            .env(
                "PATH",
                format!("{}:{}", p.bin().display(), std::env::var("PATH").unwrap()),
            )
            .env("STUB", p.stub())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    assert!(run().contains("comfyui does not answer at http://127.0.0.1:8188: nothing to unload"));
    assert!(p.requests().is_empty());

    p.session("comfyui");
    p.mark("loaded", "comfyui");
    let stdout = run();
    let posts: Vec<String> = p
        .requests()
        .into_iter()
        .filter(|r| r.starts_with("POST "))
        .collect();
    assert_eq!(
        posts,
        [r#"POST /queue {"clear": true}"#, "POST /interrupt {}", FREE]
    );
    assert!(!p.requests().iter().any(|r| r.starts_with("GET /queue")));
    assert!(!p.loaded("comfyui"));
    assert!(p.stub().join("sessions/comfyui").is_file());
    assert_eq!(
        stdout.lines().last().unwrap(),
        "comfyui has unloaded its models; its server keeps running"
    );
    assert_eq!(p.switches(), Vec::<String>::new());
}
