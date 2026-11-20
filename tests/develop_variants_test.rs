//! The official `develop` variants (docs/reference/plugins.md, Official plugins):
//! `develop_rust`, `develop_node` and `develop_react` are `develop`'s machine under their own
//! names, with `develop`'s scripts but for the gate, the conventions the prompts read and the
//! tools they declare. Each gate runs with stub `cargo`, `npm`, `pnpm` and `yarn` on a `PATH`
//! of one folder, so no test needs Rust's or Node's real tools, or the network.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

mod common;
use common::write_script;

const VARIANTS: [&str; 3] = ["develop_rust", "develop_node", "develop_react"];

/// The files each variant has of its own: the gate, the conventions its prompts read and the
/// tools it declares.
const OWN: [&str; 3] = ["conventions.md", "gate.sh", "requirements"];

/// The prompts that read the variant's conventions: develop's, but for that one line and the
/// variant's own script paths.
const PROMPTS: [&str; 3] = ["fix.sh", "implement.sh", "verify.sh"];

/// Records `<name> <args>` in `calls` beside it, then exits 1 if that line starts with
/// `STUB_FAIL`, else 0.
const STUB: &str = r#"#!/usr/bin/env bash
line="${0##*/} $*"
echo "$line" >> "${0%/*}/calls"
echo "ran $line"
if [ -n "${STUB_FAIL:-}" ] && [[ $line == "$STUB_FAIL"* ]]; then
  exit 1
fi
"#;

/// `decree <args>` in `dir`: (exit code, stdout, stderr).
fn decree(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = common::decree_at(dir)
        .env("NO_COLOR", "1")
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let (code, stdout, stderr) = decree(dir, args);
    assert_eq!(code, 0, "decree {args:?}:\n{stdout}{stderr}");
    stdout
}

/// A fresh `decree init` with `plugins` added.
fn project(plugins: &[&str]) -> TempDir {
    let tmp = TempDir::new().unwrap();
    ok(tmp.path(), &["init", "--ai", "claude"]);
    let mut args = vec!["plugin", "add"];
    args.extend(plugins);
    ok(tmp.path(), &args);
    tmp
}

/// A plugin file's frontmatter, parsed.
fn frontmatter(plugin: &str) -> serde_norway::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("plugins/{plugin}.md"));
    let text = fs::read_to_string(path).unwrap();
    let yaml = text
        .strip_prefix("---\n")
        .and_then(|t| t.split_once("\n---\n"))
        .map(|(fm, _)| format!("{fm}\n"))
        .unwrap();
    serde_norway::from_str(&yaml).unwrap()
}

fn scripts(plugin: &str) -> BTreeMap<String, String> {
    frontmatter(plugin)["scripts"]
        .as_mapping()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.as_str().unwrap().into(), v.as_str().unwrap().into()))
        .collect()
}

/// A `PATH` of one folder in `dir`: links to the system tools the scripts use (no `cargo`,
/// `node`, `npm`, `pnpm` or `yarn`, whatever this system has), and a stub for each of `stubs`.
fn path_with(dir: &Path, stubs: &[&str]) -> PathBuf {
    let bin = dir.join("bin");
    fs::create_dir_all(&bin).unwrap();
    for tool in ["awk", "bash", "cat", "dirname", "git", "grep", "sed", "tee"] {
        let found = std::process::Command::new("bash")
            .args(["-c", &format!("command -v {tool}")])
            .output()
            .unwrap();
        let target = String::from_utf8(found.stdout).unwrap();
        assert!(!target.trim().is_empty(), "{tool} is not on this system");
        std::os::unix::fs::symlink(target.trim(), bin.join(tool)).unwrap();
    }
    for stub in stubs {
        write_script(&bin.join(stub), STUB);
    }
    bin
}

/// What a gate did: its exit code, its stdout and stderr, gate.log, the event it named and
/// each stub call, in order.
#[derive(Debug)]
struct Gate {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    log: String,
    event: String,
    calls: Vec<String>,
}

/// Runs `variant`'s gate, as installed in a project, in `work` (the project's code), with only
/// `stubs` on `PATH`, `STUB_FAIL` set to `fail`.
fn run_gate(variant: &str, work: &Path, stubs: &[&str], fail: &str) -> Gate {
    let p = project(&[variant]);
    let gate = p
        .path()
        .join(format!(".decree/machines/{variant}/scripts/gate.sh"));
    let run = work.join("run");
    fs::create_dir_all(&run).unwrap();
    let event = run.join("event");
    fs::write(&event, "").unwrap();
    let bin = path_with(&run, stubs);
    let out = std::process::Command::new(&gate)
        .current_dir(work)
        .env_clear()
        .env("PATH", &bin)
        .env("STUB_FAIL", fail)
        .env("DECREE_RUN_DIR", &run)
        .env("DECREE_ROOT_RUN_DIR", &run)
        .env("DECREE_EVENT_FILE", &event)
        .output()
        .unwrap();
    Gate {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        log: fs::read_to_string(run.join("gate.log")).unwrap_or_default(),
        event: fs::read_to_string(&event).unwrap(),
        calls: fs::read_to_string(bin.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect(),
    }
}

/// A Node project's folder: `package.json` and each of `files` (lockfiles, empty).
fn node_project(package_json: &str, files: &[&str]) -> TempDir {
    let work = TempDir::new().unwrap();
    fs::write(work.path().join("package.json"), package_json).unwrap();
    for f in files {
        fs::write(work.path().join(f), "").unwrap();
    }
    work
}

/// The block between ```mermaid and ``` in a graph file.
fn mermaid(graph: &str) -> String {
    graph
        .split_once("```mermaid\n")
        .and_then(|(_, rest)| rest.split_once("```"))
        .map(|(block, _)| block.to_string())
        .unwrap_or_else(|| panic!("no mermaid block:\n{graph}"))
}

/// AC: after a fresh `init` (which adds `order` and `prune`), `decree plugin add develop_rust
/// develop_react` adds both with `infra`, which they need; `decree check` passes, and `decree graph` draws both machines with
/// `develop`'s states and transitions.
#[test]
fn add_rust_and_react_adds_infra_passes_check_and_graphs_develops_states() {
    let p = project(&["develop_rust", "develop_react"]);
    let dot = p.path().join(".decree");
    let mut files: Vec<String> = fs::read_dir(dot.join("plugins"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(
        files,
        [
            "develop_react.md",
            "develop_rust.md",
            "infra.md",
            "order.md",
            "prune.md"
        ]
    );
    let (code, stdout, stderr) = decree(p.path(), &["check"]);
    assert_eq!((code, stdout.as_str(), stderr.as_str()), (0, "", ""));
    ok(p.path(), &["graph"]);

    let reference = project(&["develop"]);
    ok(reference.path(), &["graph"]);
    let develop =
        fs::read_to_string(reference.path().join(".decree/machines/develop/graph.md")).unwrap();
    for variant in ["develop_rust", "develop_react"] {
        let graph = fs::read_to_string(dot.join(format!("machines/{variant}/graph.md"))).unwrap();
        assert!(graph.starts_with(&format!("# {variant}\n")), "{graph}");
        assert_eq!(mermaid(&graph), mermaid(&develop), "{variant}");
        for state in [
            "precheck",
            "implement",
            "gate",
            "fix",
            "final_gate",
            "verify",
        ] {
            assert!(
                mermaid(&graph).contains(&format!("    {state} --> ")),
                "{variant}: {state}"
            );
        }
    }
}

/// Each variant is `develop`'s machine but for its description, which names its ecosystem,
/// needs `infra` as `develop` does, and `decree plugin list` shows it with that description.
#[test]
fn each_variant_is_develops_machine_and_list_names_its_ecosystem() {
    let develop = frontmatter("develop");
    let tmp = TempDir::new().unwrap();
    ok(tmp.path(), &["init", "--ai", "claude"]);
    let list = ok(tmp.path(), &["plugin", "list"]);
    for (variant, ecosystem) in VARIANTS.iter().zip(["Rust", "Node", "React"]) {
        let ours = frontmatter(variant);
        assert_eq!(ours["needs"], develop["needs"], "{variant}");
        let description = ours["machine"]["description"].as_str().unwrap();
        assert!(
            description.starts_with(&format!("develop for a {ecosystem} project. ")),
            "{variant}: {description}"
        );
        let mut machine = ours["machine"].clone();
        machine["description"] = develop["machine"]["description"].clone();
        assert_eq!(machine, develop["machine"], "{variant}");
        let line = list
            .lines()
            .find(|l| l.trim_start().starts_with(&format!("{variant} ")))
            .unwrap_or_else(|| panic!("{variant} not listed:\n{list}"));
        assert!(line.contains(description), "{line}");
    }
}

/// AC: the scripts the variants share with `develop` are the same in each. `ai.sh`,
/// `clean_tree.sh`, `commit.sh`, `keep.sh` and `precheck.sh` byte for byte; `implement.sh`,
/// `fix.sh` and `verify.sh` but for their own scripts' paths and the one line naming their
/// conventions. Each variant has no other file than those, its gate, conventions and
/// requirements; `develop_react`'s gate is `develop_node`'s but for its comment and `build`.
#[test]
fn shared_scripts_are_develops_in_every_variant() {
    let develop = scripts("develop");
    for variant in VARIANTS {
        let ours = scripts(variant);
        let mut expected: Vec<&str> = develop.keys().map(String::as_str).collect();
        expected.extend(OWN);
        expected.sort();
        expected.dedup();
        assert_eq!(
            ours.keys().map(String::as_str).collect::<Vec<_>>(),
            expected,
            "{variant}"
        );
        for (path, text) in &develop {
            if OWN.contains(&path.as_str()) {
                continue;
            }
            let theirs = &ours[path];
            if PROMPTS.contains(&path.as_str()) {
                let line = format!(
                    "Follow the conventions in .decree/machines/{variant}/scripts/conventions.md.\n"
                );
                assert_eq!(theirs.matches(&line).count(), 1, "{variant}: {path}");
                let back = theirs.replacen(&line, "", 1).replace(
                    &format!(".decree/machines/{variant}/scripts/"),
                    ".decree/machines/develop/scripts/",
                );
                assert_eq!(&back, text, "{variant}: {path} differs from develop's");
            } else {
                assert_eq!(theirs, text, "{variant}: {path} differs from develop's");
            }
        }
        assert!(
            ours["conventions.md"].contains(&format!("{variant}'s implement, fix and verify")),
            "{variant}"
        );
    }
    let body = |gate: &str| -> String {
        gate.lines()
            .filter(|l| !l.starts_with('#') && !l.starts_with("required="))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let (node, react) = (
        &scripts("develop_node")["gate.sh"],
        &scripts("develop_react")["gate.sh"],
    );
    assert_eq!(body(node), body(react));
    assert!(node.contains("\nrequired=()\n"));
    assert!(react.contains("\nrequired=(build)\n"));
}

/// AC: with a stub `cargo` whose `test` fails, `develop_rust`'s gate runs fmt, clippy and test
/// in that order, names `fail` and exits 0; with every check passing it names `pass`.
#[test]
fn rust_gate_runs_fmt_clippy_test_in_order_and_names_fail() {
    let work = TempDir::new().unwrap();
    let gate = run_gate("develop_rust", work.path(), &["cargo"], "cargo test");
    assert_eq!(gate.code, Some(0), "{gate:?}");
    assert_eq!(gate.event, "fail\n");
    assert_eq!(
        gate.calls,
        [
            "cargo fmt --check",
            "cargo clippy --all-targets -- -D warnings",
            "cargo test"
        ]
    );
    assert_eq!(gate.log, gate.stdout);

    let work = TempDir::new().unwrap();
    let gate = run_gate("develop_rust", work.path(), &["cargo"], "");
    assert_eq!((gate.code, gate.event.as_str()), (Some(0), "pass\n"));
    assert_eq!(gate.calls.len(), 3);

    // The first that fails ends the chain.
    let work = TempDir::new().unwrap();
    let gate = run_gate("develop_rust", work.path(), &["cargo"], "cargo fmt");
    assert_eq!((gate.code, gate.event.as_str()), (Some(0), "fail\n"));
    assert_eq!(gate.calls, ["cargo fmt --check"]);
}

/// AC: without `cargo` on `PATH`, `develop_rust`'s gate exits non-zero, names `cargo`, and
/// names no event.
#[test]
fn rust_gate_without_cargo_exits_non_zero_naming_it() {
    let work = TempDir::new().unwrap();
    let gate = run_gate("develop_rust", work.path(), &[], "");
    assert_eq!(gate.code, Some(1), "{gate:?}");
    assert_eq!(
        gate.stderr,
        "gate: cargo not found on PATH; the checks cannot run\n"
    );
    assert_eq!(gate.event, "");
}

/// AC: with `pnpm-lock.yaml` and a `package.json` defining only `test`, `develop_node`'s gate
/// calls only `pnpm`, to install for CI and then to run `test`, and says it skipped `lint` and
/// `typecheck`.
#[test]
fn node_gate_uses_pnpm_from_its_lockfile_and_skips_undefined_scripts() {
    let work = node_project(
        r#"{ "name": "app", "scripts": { "test": "vitest run" } }"#,
        &["pnpm-lock.yaml"],
    );
    let gate = run_gate("develop_node", work.path(), &["pnpm", "npm", "yarn"], "");
    assert_eq!(
        (gate.code, gate.event.as_str()),
        (Some(0), "pass\n"),
        "{gate:?}"
    );
    assert_eq!(
        gate.calls,
        ["pnpm install --frozen-lockfile", "pnpm run test"]
    );
    assert!(
        gate.stdout
            .contains("gate: skipped lint, typecheck: not in package.json's scripts\n"),
        "{}",
        gate.stdout
    );
    assert_eq!(gate.log, gate.stdout);
}

/// Each lockfile picks its manager and its CI install; npm without one. Scripts run in the
/// order lint, typecheck, test, and the first that fails names `fail`.
#[test]
fn node_gate_picks_the_manager_and_install_from_the_lockfile() {
    let all = r#"{
  "name": "app",
  "scripts": {
    "test": "jest",
    "lint": "eslint .",
    "typecheck": "tsc --noEmit"
  },
  "devDependencies": { "lint": "1.0.0" }
}"#;
    for (files, install, pm) in [
        (&["package-lock.json"][..], "npm ci", "npm"),
        (&[][..], "npm install", "npm"),
        (&["yarn.lock"][..], "yarn install --frozen-lockfile", "yarn"),
        (
            &["yarn.lock", ".yarnrc.yml"][..],
            "yarn install --immutable",
            "yarn",
        ),
        (
            &["pnpm-lock.yaml"][..],
            "pnpm install --frozen-lockfile",
            "pnpm",
        ),
    ] {
        let work = node_project(all, files);
        let gate = run_gate("develop_node", work.path(), &["pnpm", "npm", "yarn"], "");
        assert_eq!(gate.event, "pass\n", "{files:?}: {gate:?}");
        assert_eq!(
            gate.calls,
            [
                install.to_string(),
                format!("{pm} run lint"),
                format!("{pm} run typecheck"),
                format!("{pm} run test"),
            ],
            "{files:?}"
        );
        assert!(!gate.stdout.contains("skipped"), "{}", gate.stdout);
    }

    let work = node_project(all, &["package-lock.json"]);
    let gate = run_gate("develop_node", work.path(), &["npm"], "npm run typecheck");
    assert_eq!((gate.code, gate.event.as_str()), (Some(0), "fail\n"));
    assert_eq!(gate.calls, ["npm ci", "npm run lint", "npm run typecheck"]);

    // An install that fails is a failing check, as in `npm ci && npm test`.
    let work = node_project(all, &["package-lock.json"]);
    let gate = run_gate("develop_node", work.path(), &["npm"], "npm ci");
    assert_eq!((gate.code, gate.event.as_str()), (Some(0), "fail\n"));
    assert_eq!(gate.calls, ["npm ci"]);
}

/// AC: with a `package.json` that defines no scripts, `develop_node`'s gate names `pass`, says
/// it checked nothing, and calls nothing.
#[test]
fn node_gate_with_no_scripts_passes_and_says_it_checked_nothing() {
    for package_json in [
        r#"{ "name": "app" }"#,
        r#"{ "name": "app", "scripts": {} }"#,
    ] {
        let work = node_project(package_json, &["package-lock.json"]);
        let gate = run_gate("develop_node", work.path(), &["npm"], "");
        assert_eq!(
            (gate.code, gate.event.as_str()),
            (Some(0), "pass\n"),
            "{gate:?}"
        );
        assert_eq!(
            gate.stdout,
            "gate: package.json defines none of lint, typecheck, test: checked nothing\n"
        );
        assert_eq!(gate.log, gate.stdout);
        assert!(gate.calls.is_empty(), "{:?}", gate.calls);
    }
}

/// Without `package.json`, or without the lockfile's manager on `PATH`, the gate cannot run:
/// a non-zero exit naming what is missing, and no event.
#[test]
fn node_gate_without_package_json_or_its_manager_exits_non_zero() {
    let work = TempDir::new().unwrap();
    let gate = run_gate("develop_node", work.path(), &["npm"], "");
    assert_eq!((gate.code, gate.event.as_str()), (Some(1), ""), "{gate:?}");
    assert!(gate.stderr.contains("no package.json"), "{}", gate.stderr);

    let work = node_project(r#"{ "scripts": { "test": "x" } }"#, &["yarn.lock"]);
    let gate = run_gate("develop_node", work.path(), &["npm"], "");
    assert_eq!((gate.code, gate.event.as_str()), (Some(1), ""), "{gate:?}");
    assert_eq!(
        gate.stderr,
        "gate: yarn not found on PATH; the checks cannot run\n"
    );
    assert!(gate.calls.is_empty());
}

/// AC: `develop_react`'s gate runs `develop_node`'s checks then `build`, and names `fail` when
/// `build` fails, or when `package.json` defines no `build`.
#[test]
fn react_gate_builds_after_node_checks_and_names_fail_when_build_fails() {
    let app = r#"{ "scripts": { "test": "vitest run", "build": "vite build" } }"#;
    let work = node_project(app, &["package-lock.json"]);
    let gate = run_gate("develop_react", work.path(), &["npm"], "npm run build");
    assert_eq!(
        (gate.code, gate.event.as_str()),
        (Some(0), "fail\n"),
        "{gate:?}"
    );
    assert_eq!(gate.calls, ["npm ci", "npm run test", "npm run build"]);
    assert!(gate.stdout.contains("gate: skipped lint, typecheck"));

    let work = node_project(app, &["package-lock.json"]);
    let gate = run_gate("develop_react", work.path(), &["npm"], "");
    assert_eq!((gate.code, gate.event.as_str()), (Some(0), "pass\n"));

    let work = node_project(r#"{ "scripts": {} }"#, &["package-lock.json"]);
    let gate = run_gate("develop_react", work.path(), &["npm"], "");
    assert_eq!(
        (gate.code, gate.event.as_str()),
        (Some(0), "fail\n"),
        "{gate:?}"
    );
    assert!(
        gate.stdout
            .contains("gate: package.json defines no `build` script, which this gate needs\n"),
        "{}",
        gate.stdout
    );
    assert_eq!(gate.calls, ["npm ci"]);
}

/// AC: each variant declares its tools in its own `requirements` (`cargo`; `node` and `npm`),
/// and with `develop_node` added, its `precheck` without `node` on `PATH` fails, reporting
/// `node` missing from `machines/develop_node/scripts/requirements`.
#[test]
fn node_precheck_without_node_fails_naming_its_requirements() {
    for (variant, tools) in [
        ("develop_rust", &["cargo"][..]),
        ("develop_node", &["node", "npm"][..]),
        ("develop_react", &["node", "npm"][..]),
    ] {
        let declared: Vec<String> = scripts(variant)["requirements"]
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect();
        let expected: Vec<String> = tools.iter().map(|t| format!("command {t}")).collect();
        assert_eq!(declared, expected, "{variant}");
    }

    let p = project(&["develop_node"]);
    let run = p.path().join("run");
    let bin = path_with(&run, &["npm"]);
    let out = std::process::Command::new(
        p.path()
            .join(".decree/machines/develop_node/scripts/precheck.sh"),
    )
    .current_dir(p.path())
    .env_clear()
    .env("PATH", &bin)
    .output()
    .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_ne!(out.status.code(), Some(0), "{stdout}");
    assert_eq!(
        stdout,
        "ok       command git (machines/infra/scripts/requirements)\n\
         missing  command node (machines/develop_node/scripts/requirements)\n\
         ok       command npm (machines/develop_node/scripts/requirements)\n"
    );
}
