//! The official `registry` plugin (plugins/registry.md): a table of servers, their services
//! and addresses in `state/services.git.txt`; `check` probes each address and records which
//! answered; `registry.sh` looks addresses up, live ones first. Each test is a fresh project
//! with `registry` installed.

use assert_cmd::Command;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// decree's own `DECREE_*` variables, when the tests run inside a decree run, would point
/// the binary at that project; drop them all.
fn clean(cmd: &mut std::process::Command) {
    for (k, _) in std::env::vars() {
        if k.starts_with("DECREE") || k.ends_with("_URL") {
            cmd.env_remove(k);
        }
    }
    cmd.env("NO_COLOR", "1");
}

fn decree(root: &Path, args: &[&str]) -> Command {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_decree"));
    clean(&mut cmd);
    let mut cmd = Command::from_std(cmd);
    cmd.current_dir(root).args(args);
    cmd
}

/// `registry.sh <args>` in `root`: (exit code, stdout, stderr).
fn registry(root: &Path, args: &[&str]) -> (i32, String, String) {
    let mut cmd = std::process::Command::new("bash");
    clean(&mut cmd);
    let out = cmd
        .current_dir(root)
        .arg(".decree/machines/registry/scripts/registry.sh")
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

/// `decree init`, `decree install registry`, and `table` as the services.
fn project(table: &str) -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();
    decree(&root, &["init"]).assert().success();
    decree(&root, &["install", "registry"]).assert().success();
    fs::write(state(&root).join("services.git.txt"), table).unwrap();
    (tmp, root)
}

fn state(root: &Path) -> PathBuf {
    root.join(".decree/machines/registry/state")
}

/// An address that answers every request with HTTP 404 (an answer: up), for the test's life.
fn http_server() -> String {
    http_server_answering("404 Not Found")
}

/// An address that answers every request with `status`, for the test's life.
fn http_server_answering(status: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf);
            let _ = s.write_all(
                format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            );
        }
    });
    format!("http://127.0.0.1:{port}")
}

/// An address nothing listens on: down.
fn closed_address() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

fn has(tool: &str) -> bool {
    std::process::Command::new("sh")
        .args(["-c", &format!("command -v {tool}")])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// AC: installing it seeds a commented table (no services) that no install replaces, and
/// check passes; the table is committed and the health is not.
#[test]
fn installs_with_an_empty_table_that_installs_keep() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    decree(root, &["init"]).assert().success();
    decree(root, &["install", "registry"]).assert().success();
    decree(root, &["check"]).assert().success();
    let table = state(root).join("services.git.txt");
    assert!(fs::read_to_string(&table)
        .unwrap()
        .contains("# box1    render"));
    assert_eq!(registry(root, &["list"]).1, "");
    fs::write(&table, "a ollama http://127.0.0.1:9\n").unwrap();
    decree(root, &["install", "--official", "registry"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(&table).unwrap(),
        "a ollama http://127.0.0.1:9\n"
    );
}

/// AC: `resolve` gives each place a service is, with the lease to get there (`-` for
/// none), optionally on one server, and fails for an unknown one; `url` gives the
/// addresses alone; `env` gives <SERVICE>_URL lines for eval, leaving out a variable
/// already set.
#[test]
fn resolves_where_to_go_and_which_lease_to_get() {
    let (_tmp, root) = project(
        "# a comment\n\
         box1 render http://10.0.0.5:8188 box1   # leased here\n\
         box2 render http://10.0.0.9:8188 box2@http://10.0.0.9:8080\n\
         \n\
         laptop search-2 http://localhost:7700\n",
    );
    let (code, out, _) = registry(&root, &["resolve", "render"]);
    assert_eq!(
        (code, out.as_str()),
        (
            0,
            "box1 http://10.0.0.5:8188 box1\nbox2 http://10.0.0.9:8188 box2@http://10.0.0.9:8080\n"
        )
    );
    assert_eq!(
        registry(&root, &["resolve", "render", "box2"]).1,
        "box2 http://10.0.0.9:8188 box2@http://10.0.0.9:8080\n"
    );
    assert_eq!(
        registry(&root, &["resolve", "search-2"]).1,
        "laptop http://localhost:7700 -\n"
    );
    assert_eq!(
        registry(&root, &["url", "render"]).1,
        "http://10.0.0.5:8188\nhttp://10.0.0.9:8188\n"
    );
    let (code, _, err) = registry(&root, &["resolve", "tts"]);
    assert_eq!(code, 1);
    assert!(err.contains("no service `tts`"), "{err}");
    assert_eq!(
        registry(&root, &["env"]).1,
        "RENDER_URL=http://10.0.0.5:8188\nSEARCH_2_URL=http://localhost:7700\n"
    );
    let out = std::process::Command::new("bash")
        .current_dir(&root)
        .env("RENDER_URL", "http://mine")
        .args([".decree/machines/registry/scripts/registry.sh", "env"])
        .output()
        .unwrap();
    let env = String::from_utf8(out.stdout).unwrap();
    assert!(!env.contains("RENDER_URL"), "{env}");
}

/// AC: `check` (a run of the machine, at each daemon start) records the address that
/// answers as up and the one that does not as down; `url` then puts the live one first.
#[test]
fn check_records_health_and_lookups_put_live_addresses_first() {
    if !has("curl") {
        eprintln!("no curl: skipping");
        return;
    }
    let down = closed_address();
    let up = http_server();
    let (_tmp, root) = project(&format!("a comfy {down}\nb comfy {up}\nc ollama {down}\n"));
    // Before any check, the table's order.
    assert_eq!(
        registry(&root, &["url", "comfy"]).1,
        format!("{down}\n{up}\n")
    );
    decree(&root, &["daemon", "--once"]).assert().success();
    let health: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(state(&root).join("health.json")).unwrap())
            .unwrap();
    let words: Vec<(&str, &str)> = health
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["server"].as_str().unwrap(), e["state"].as_str().unwrap()))
        .collect();
    assert_eq!(
        words,
        [("a", "down"), ("b", "up"), ("c", "down")],
        "{health}"
    );
    // A 404 is an answer: up, with its code; no answer has none.
    assert_eq!(health[1]["http"], 404, "{health}");
    assert!(health[0].get("http").is_none(), "{health}");
    assert!(
        health[1]["checked"].as_str().unwrap().ends_with('Z'),
        "{health}"
    );
    assert_eq!(
        registry(&root, &["url", "comfy"]).1,
        format!("{up}\n{down}\n")
    );
    let list = registry(&root, &["list"]).1;
    assert!(
        list.lines()
            .any(|l| l.starts_with("b ") && l.ends_with(" up http=404")),
        "{list}"
    );
}

/// AC: a malformed line fails `check`, naming the file's line and the problem.
#[test]
fn a_malformed_table_fails_check_naming_the_line() {
    let (_tmp, root) = project(
        "a comfy http://x\nb comfy ftp://x\nc d http://x r extra\ne f http://x r@ftp://y\n",
    );
    let (code, _, err) = registry(&root, &["check"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("services.git.txt:2: `ftp://x` is not an http://"),
        "{err}"
    );
    assert!(
        err.contains("services.git.txt:3: want <server> <service> <url> [lease]"),
        "{err}"
    );
    assert!(
        err.contains("services.git.txt:4: lease `r@ftp://y` is not"),
        "{err}"
    );
    assert!(!err.contains(":1:"), "{err}");
}

/// AC: a 5xx answer is down, with its code; `list --json` gives the table with each line's
/// health as one array.
#[test]
fn a_server_error_is_down_and_list_json_says_so() {
    if !has("curl") || !has("jq") {
        eprintln!("no curl or jq: skipping");
        return;
    }
    let broken = http_server_answering("503 Service Unavailable");
    let up = http_server();
    let (_tmp, root) = project(&format!(
        "a comfy {broken}\nb comfy {up}\nc comfy http://127.0.0.1:9 box1\n"
    ));
    decree(&root, &["daemon", "--once"]).assert().success();
    let (code, out, err) = registry(&root, &["list", "--json"]);
    assert_eq!(code, 0, "{err}");
    let list: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(list[0]["state"], "down", "{list}");
    assert_eq!(list[0]["http"], 503, "{list}");
    assert_eq!(list[1]["state"], "up", "{list}");
    assert_eq!(list[2]["lease"], "box1", "{list}");
    assert_eq!(
        registry(&root, &["url", "comfy"]).1.lines().next(),
        Some(up.as_str())
    );
}

/// AC: a `#` inside an address is part of it (a comment starts at a line's start or after a
/// space); `env` gives a service whose name starts with a digit a variable with a leading `_`.
#[test]
fn an_address_may_hold_a_hash_and_env_names_are_valid() {
    if !has("jq") {
        eprintln!("no jq: skipping");
        return;
    }
    let (_tmp, root) = project(
        "box1 3d http://10.0.0.5:8188/#/frag   # a comment\n\
         # box2 3d http://10.0.0.9:8188\n",
    );
    assert_eq!(registry(&root, &["check"]).0, 0);
    assert_eq!(
        registry(&root, &["url", "3d"]).1,
        "http://10.0.0.5:8188/#/frag\n"
    );
    let (code, out, err) = registry(&root, &["env"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "_3D_URL=http://10.0.0.5:8188/#/frag\n");
}
