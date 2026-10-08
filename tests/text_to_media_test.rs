//! `examples/text-to-media/`: the `comfy` machine run through the binary against a stub
//! ComfyUI. The example's `.decree/`, with its workflows in `lib/comfy/`, is copied to a temp project, with a
//! stub `curl` first on `PATH` that answers ComfyUI's `/upload/image`, `/prompt`,
//! `/history/<id>` and `/view` from files under `stub/`, and logs each request. No ComfyUI,
//! no network.

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

mod common;
use common::write_script;

/// ComfyUI, as far as the scripts use it. `/history/p1` answers `{}` until it has been
/// asked `stub/polls_before_done` times, then `stub/history.json`.
const STUB_CURL: &str = r#"#!/usr/bin/env bash
STUB="$DECREE_PROJECT_ROOT/stub"
out=""; data=""; url=""; query=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out=$2; shift 2 ;;
    -d) data=$2; shift 2 ;;
    -F) data="$data $2"; shift 2 ;;
    --data-urlencode) query="$query&$2"; shift 2 ;;
    --max-time|-H|-w) shift 2 ;;
    http*) url=$1; shift ;;
    *) shift ;;
  esac
done
path=${url#http://127.0.0.1:8188}
echo "$path$query" >> "$STUB/requests"
case "$path" in
  /upload/image)
    echo "$data" > "$STUB/uploaded"
    echo '{"name": "ref.png", "subfolder": "", "type": "input"}' ;;
  /prompt)
    cp "${data#@}" "$STUB/submitted.json"
    echo '{"prompt_id": "p1", "number": 1, "node_errors": {}}' ;;
  /history/p1)
    n=$(( $(cat "$STUB/polls" 2>/dev/null || echo 0) + 1 ))
    echo "$n" > "$STUB/polls"
    if [ "$n" -le "$(cat "$STUB/polls_before_done")" ]; then echo '{}'; else cat "$STUB/history.json"; fi ;;
  /view)
    cp "$STUB/rendered.png" "$out" ;;
  *) exit 22 ;;
esac
"#;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nstub image";

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/text-to-media")
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

/// The history entry of a finished prompt, with one saved image and one preview.
fn history(status: &str) -> String {
    serde_json::json!({
        "p1": {
            "status": { "status_str": status, "completed": status == "success", "messages": [] },
            "outputs": {
                "9": { "images": [{ "filename": "decree_fox_00001_.png", "subfolder": "", "type": "output" }] },
                "12": { "images": [{ "filename": "preview.png", "subfolder": "", "type": "temp" }] }
            }
        }
    })
    .to_string()
}

struct Project {
    tmp: TempDir,
}

impl Project {
    /// The example's `.decree/` (its workflows in `lib/comfy/`) with no messages, a stub ComfyUI that
    /// finishes the prompt after `polls_before_done` empty answers with `status`, and an
    /// image in `images/ref.png`.
    fn new(polls_before_done: u32, status: &str) -> Project {
        let p = Project {
            tmp: TempDir::new().unwrap(),
        };
        copy_dir(&example().join(".decree"), &p.root().join(".decree"));
        fs::remove_dir_all(p.root().join(".decree/migrations")).unwrap();
        fs::create_dir(p.root().join(".decree/migrations")).unwrap();
        fs::create_dir(p.root().join("images")).unwrap();
        fs::write(p.root().join("images/ref.png"), PNG).unwrap();
        fs::create_dir(p.stub()).unwrap();
        fs::write(
            p.stub().join("polls_before_done"),
            polls_before_done.to_string(),
        )
        .unwrap();
        fs::write(p.stub().join("history.json"), history(status)).unwrap();
        fs::write(p.stub().join("rendered.png"), PNG).unwrap();
        fs::create_dir(p.bin()).unwrap();
        write_script(&p.bin().join("curl"), STUB_CURL);
        p
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    fn stub(&self) -> PathBuf {
        self.root().join("stub")
    }

    fn bin(&self) -> PathBuf {
        self.root().join("bin")
    }

    /// Queues a `comfy` message with `params` and `body`, then runs `decree process`, which
    /// exits `code`: 0, or 1 when the run ends in `failed`.
    fn process(&self, params: &[&str], body: &str, code: i32) {
        let path = std::env::join_paths(
            std::iter::once(self.bin())
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        let mut emit = cargo_bin_cmd!("decree");
        emit.current_dir(self.root())
            .args(["emit", "--machine", "comfy"])
            .write_stdin(body);
        for param in params {
            emit.args(["--param", param]);
        }
        emit.assert().success();
        cargo_bin_cmd!("decree")
            .current_dir(self.root())
            .env("PATH", path)
            .env("COMFY_POLL_S", "0")
            .env("NO_COLOR", "1")
            .arg("process")
            .assert()
            .code(code);
    }

    fn run_dir(&self) -> PathBuf {
        let runs: Vec<PathBuf> = fs::read_dir(self.root().join(".decree/runs"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(runs.len(), 1, "{runs:?}");
        runs.into_iter().next().unwrap()
    }

    /// The run's path: `<from> <event> <to>` for each transition after the claim.
    fn path(&self) -> Vec<String> {
        fs::read_to_string(self.run_dir().join("events.jsonl"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .filter(|e| e["type"] == "transition" && e["source"] != "claim")
            .map(|e| format!("{} {} {}", e["from"], e["event"], e["to"]).replace('"', ""))
            .collect()
    }

    fn final_state(&self) -> String {
        let message = fs::read_to_string(self.run_dir().join("message.md")).unwrap();
        message
            .lines()
            .find_map(|l| l.strip_prefix("state: "))
            .unwrap()
            .to_string()
    }

    /// The requests the stub ComfyUI got, in order.
    fn requests(&self) -> Vec<String> {
        fs::read_to_string(self.stub().join("requests"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    /// The prompt graph `submit` posted.
    fn submitted(&self) -> Value {
        let text = fs::read_to_string(self.stub().join("submitted.json")).unwrap();
        serde_json::from_str::<Value>(&text).unwrap()["prompt"].clone()
    }

    /// The log of the script `name` in the run.
    fn log(&self, name: &str) -> String {
        fs::read_dir(self.run_dir())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.to_string_lossy().ends_with(&format!("-{name}.log")))
            .map(|p| fs::read_to_string(p).unwrap())
            .collect()
    }
}

/// build, submit, await (until the prompt is in the history), fetch: the saved image, not the
/// preview, is at `output` with its extension, and the run ends in `done`.
#[test]
fn a_render_is_submitted_awaited_and_saved_at_output() {
    let p = Project::new(2, "success");
    p.process(
        &[
            "method=image_flux2_text_landscape",
            "output=art/fox",
            "width=500",
            "seed=7",
        ],
        "A red fox in the snow",
        0,
    );
    assert_eq!(
        p.path(),
        [
            "build done submit",
            "submit done await",
            "await done fetch",
            "fetch done done",
        ]
    );
    assert_eq!(p.final_state(), "done");
    assert_eq!(fs::read(p.root().join("art/fox.png")).unwrap(), PNG);
    assert!(!p.root().join("art/fox.png.part").exists());
    assert_eq!(fs::read_dir(p.root().join("art")).unwrap().count(), 1);
    assert_eq!(
        p.requests(),
        [
            "/prompt",
            "/history/p1",
            "/history/p1",
            "/history/p1",
            "/view&filename=decree_fox_00001_.png&subfolder=&type=output",
        ]
    );
    let prompt = p.submitted();
    assert_eq!(prompt["6"]["inputs"]["text"], "A red fox in the snow");
    assert_eq!(prompt["47"]["inputs"]["width"], 496);
    assert_eq!(prompt["47"]["inputs"]["height"], 832);
    assert_eq!(prompt["25"]["inputs"]["noise_seed"], 7);
    let prefix = prompt["9"]["inputs"]["filename_prefix"].as_str().unwrap();
    assert!(prefix.starts_with("decree_"), "{prefix}");
}

/// An image-to-* method uploads `input_image` and points LoadImage at the uploaded name; the
/// negative prompt keeps its text.
#[test]
fn an_image_method_uploads_input_image() {
    let p = Project::new(0, "success");
    p.process(
        &[
            "method=video_i2v_wan2.2_14B_long",
            "output=art/fox",
            "input_image=images/ref.png",
        ],
        "The fox runs",
        0,
    );
    assert_eq!(p.final_state(), "done");
    assert_eq!(p.requests()[..2], ["/upload/image", "/prompt"]);
    let uploaded = fs::read_to_string(p.stub().join("uploaded")).unwrap();
    assert!(uploaded.contains("image=@") && uploaded.contains("/images/ref.png"));
    let prompt = p.submitted();
    assert_eq!(prompt["97"]["inputs"]["image"], "ref.png");
    assert_eq!(prompt["93"]["inputs"]["text"], "The fox runs");
    assert_ne!(prompt["89"]["inputs"]["text"], "The fox runs");
    assert_eq!(fs::read(p.root().join("art/fox.png")).unwrap(), PNG);
}

/// The invoke's `env: { METHOD: … }` names the method, as in the README's "When messages stop
/// naming a method": `build` reads it before the `method` param, from `$DECREE_LIB/comfy/`.
#[test]
fn an_invoke_env_method_is_built_from_lib() {
    let p = Project::new(0, "success");
    let machine = p.root().join(".decree/machines/comfy.yml");
    let text = fs::read_to_string(&machine).unwrap();
    let text = text.replacen(
        "    invoke: build\n",
        "    invoke: { script: { name: build, env: { METHOD: image_flux2_text_landscape } } }\n",
        1,
    );
    assert!(text.contains("METHOD: image_flux2_text_landscape"));
    fs::write(&machine, text).unwrap();
    p.process(&["output=art/fox"], "A red fox in the snow", 0);
    assert_eq!(p.final_state(), "done");
    assert!(p
        .log("build")
        .contains("=== image_flux2_text_landscape ==="));
    assert_eq!(
        p.submitted()["6"]["inputs"]["text"],
        "A red fox in the snow"
    );
}

/// An unknown method fails in `build`, before any request, and the log lists the methods.
#[test]
fn an_unknown_method_fails_in_build_listing_the_methods() {
    let p = Project::new(0, "success");
    p.process(&["method=oil_painting", "output=art/fox"], "A fox", 1);
    assert_eq!(p.path(), ["build error failed"]);
    assert_eq!(p.final_state(), "failed");
    assert!(p.requests().is_empty());
    let log = p.log("build");
    assert!(
        log.contains(
            "unknown method 'oil_painting'; the methods are: image_flux2_text_image, \
             image_flux2_text_landscape, video_i2v_wan2.2_14B_long"
        ),
        "{log}"
    );
}

#[test]
fn a_missing_required_param_fails_in_build() {
    for (params, want) in [
        (
            vec!["output=art/fox"],
            "method is required; the methods are: ",
        ),
        (
            vec!["method=image_flux2_text_landscape"],
            "output is required",
        ),
        (
            vec!["method=image_flux2_text_image", "output=art/fox"],
            "input_image is required by method image_flux2_text_image",
        ),
    ] {
        let p = Project::new(0, "success");
        p.process(&params, "A fox", 1);
        assert_eq!(p.path(), ["build error failed"], "{params:?}");
        assert!(
            p.log("build").contains(want),
            "{params:?}: {}",
            p.log("build")
        );
        assert!(p.requests().is_empty());
    }
}

/// A prompt that ComfyUI reports as failed fails `await`, which has no retry, and nothing is
/// fetched.
#[test]
fn a_failed_render_fails_in_await() {
    let p = Project::new(0, "error");
    p.process(
        &["method=image_flux2_text_landscape", "output=art/fox"],
        "A fox",
        1,
    );
    assert_eq!(
        p.path(),
        [
            "build done submit",
            "submit done await",
            "await error failed"
        ]
    );
    assert!(p.log("await").contains("prompt p1 failed"));
    assert!(!p.root().join("art").exists());
}

/// One machine, `comfy`, with the states build, submit, await, fetch, done and failed.
#[test]
fn comfy_is_the_only_machine_and_has_six_states() {
    let machines: Vec<String> = fs::read_dir(example().join(".decree/machines"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(machines, ["comfy.yml"]);
    let text = fs::read_to_string(example().join(".decree/machines/comfy.yml")).unwrap();
    let machine: serde_norway::Value = serde_norway::from_str(&text).unwrap();
    let states: Vec<&str> = machine["states"]
        .as_mapping()
        .unwrap()
        .keys()
        .map(|k| k.as_str().unwrap())
        .collect();
    assert_eq!(
        states,
        ["build", "submit", "await", "fetch", "done", "failed"]
    );
}
