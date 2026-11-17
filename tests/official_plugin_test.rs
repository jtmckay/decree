//! The official plugins built into decree (docs/reference/plugins.md, Official plugins):
//! `decree plugin list` and `decree plugin add` in a fresh `init`, with their JSON output
//! validated against `cli/plugin.schema.json`, and each official plugin, added alone with its
//! `needs` to an empty project, passing `decree check`.

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[path = "common/schema.rs"]
mod schema;

const OFFICIAL: [&str; 10] = [
    "comfy",
    "develop",
    "develop_by_size",
    "gliner_router",
    "infra",
    "lease",
    "llm_router",
    "newsletter",
    "qa",
    "router",
];

/// `decree <args>` in `dir`: (exit code, stdout, stderr).
fn decree(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = cargo_bin_cmd!("decree")
        .current_dir(dir)
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

/// A fresh `decree init`.
fn init() -> TempDir {
    let tmp = TempDir::new().unwrap();
    ok(tmp.path(), &["init", "--ai", "claude"]);
    tmp
}

fn json(dir: &Path, args: &[&str]) -> Value {
    let mut args = args.to_vec();
    args.extend(["--format", "json"]);
    let doc: Value = serde_json::from_str(&ok(dir, &args)).unwrap();
    let errors = schema::errors(&schema::validator(schema::CLI_PLUGIN_SCHEMA), &doc);
    assert!(errors.is_empty(), "{errors:?}\n{doc}");
    doc
}

/// The built-in copy of an official plugin.
fn built_in(name: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("plugins/{name}.md")))
        .unwrap()
}

/// AC: after a fresh `init`, `plugin add develop` puts `develop` and every plugin it needs in
/// `plugins/`, names those it added as needed, and `decree check` passes.
#[test]
fn add_develop_adds_what_it_needs_and_names_it() {
    let p = init();
    let out = ok(p.path(), &["plugin", "add", "develop"]);
    assert!(out.contains("added .decree/plugins/develop.md\n"), "{out}");
    assert!(
        out.contains("added .decree/plugins/infra.md, needed by develop\n"),
        "{out}"
    );
    let mut files: Vec<String> = fs::read_dir(p.path().join(".decree/plugins"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(files, ["develop.md", "infra.md"]);
    for name in ["develop", "infra"] {
        assert_eq!(
            fs::read_to_string(p.path().join(format!(".decree/plugins/{name}.md"))).unwrap(),
            built_in(name)
        );
        assert!(p
            .path()
            .join(format!(".decree/machines/{name}.yml"))
            .is_file());
    }
    let (code, stdout, stderr) = decree(p.path(), &["check"]);
    assert_eq!((code, stdout.as_str(), stderr.as_str()), (0, "", ""));

    // qa needs develop, which is there: only qa is added, and `needed_by` is empty.
    let doc = json(p.path(), &["plugin", "add", "qa"]);
    assert_eq!(doc["plugins"].as_array().unwrap().len(), 1, "{doc}");
    assert_eq!(doc["plugins"][0]["status"], "added");
}

/// AC: in a project that added `develop`, edited its gate and exported it, `plugin list`
/// marks `develop` added and differs; `plugin add develop` without `--force` changes nothing
/// and says so; `--force` shows the gate edit dropped and makes the file the built-in copy.
#[test]
fn an_edited_plugin_is_marked_kept_and_replaced_only_with_force() {
    let p = init();
    ok(p.path(), &["plugin", "add", "develop"]);
    let gate = p.path().join(".decree/scripts/develop/gate.sh");
    let text = fs::read_to_string(&gate).unwrap();
    let edited = text.replacen(
        "  echo \"gate: no checks configured;",
        "  cargo test --workspace\n  echo \"gate: no checks configured;",
        1,
    );
    assert_ne!(text, edited);
    fs::write(&gate, edited).unwrap();
    ok(p.path(), &["plugin", "export", "develop"]);
    let plugin = p.path().join(".decree/plugins/develop.md");
    let exported = fs::read_to_string(&plugin).unwrap();
    assert!(exported.contains("cargo test --workspace"));

    let list = ok(p.path(), &["plugin", "list"]);
    let line = list
        .lines()
        .find(|l| l.trim_start().starts_with("develop "))
        .unwrap();
    assert!(line.ends_with("[added, differs]"), "{list}");
    let infra = list
        .lines()
        .find(|l| l.trim_start().starts_with("infra "))
        .unwrap();
    assert!(infra.ends_with("[added]"), "{list}");
    let doc = json(p.path(), &["plugin", "list"]);
    let names: Vec<&str> = doc["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, OFFICIAL);
    let develop = &doc["plugins"][1];
    assert_eq!(
        (develop["added"].clone(), develop["differs"].clone()),
        (true.into(), true.into())
    );
    assert_eq!(doc["plugins"][0]["added"], false);

    let before = fs::read_to_string(&gate).unwrap();
    let out = ok(p.path(), &["plugin", "add", "develop"]);
    assert!(
        out.contains(
            "kept .decree/plugins/develop.md: already in .decree/plugins/, so nothing changed"
        ),
        "{out}"
    );
    assert_eq!(fs::read_to_string(&plugin).unwrap(), exported);
    assert_eq!(fs::read_to_string(&gate).unwrap(), before);

    let out = ok(p.path(), &["plugin", "add", "--force", "develop"]);
    assert!(out.contains("replaced .decree/plugins/develop.md"), "{out}");
    assert!(
        out.contains("  changed .decree/scripts/develop/gate.sh\n"),
        "{out}"
    );
    assert!(out.contains("    -   cargo test --workspace\n"), "{out}");
    assert_eq!(fs::read_to_string(&plugin).unwrap(), built_in("develop"));
    assert!(!fs::read_to_string(&gate)
        .unwrap()
        .contains("cargo test --workspace"));
    let list = ok(p.path(), &["plugin", "list"]);
    assert!(
        list.contains("[added]") && !list.contains("differs]"),
        "{list}"
    );
}

/// AC: `plugin add nope` exits 1, lists the official names and writes nothing.
#[test]
fn an_unknown_name_exits_1_and_lists_the_official_plugins() {
    let p = init();
    let (code, stdout, stderr) = decree(p.path(), &["plugin", "add", "develop", "nope"]);
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(stderr.contains("no official plugin `nope`"), "{stderr}");
    assert!(stderr.contains(&OFFICIAL.join(", ")), "{stderr}");
    assert_eq!(
        fs::read_dir(p.path().join(".decree/plugins"))
            .unwrap()
            .count(),
        0
    );
}

/// AC: each official plugin, added alone with its `needs` to an empty project, passes
/// `decree check` without a warning.
#[test]
fn each_official_plugin_alone_passes_check() {
    for name in OFFICIAL {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".decree")).unwrap();
        ok(tmp.path(), &["plugin", "add", name]);
        let (code, stdout, stderr) = decree(tmp.path(), &["check"]);
        assert_eq!(
            (code, stdout.as_str(), stderr.as_str()),
            (0, "", ""),
            "{name}"
        );
    }
}

/// `decree plugin export` with no name exports only the plugins whose installed copy
/// differs; `--dry-run` names them and writes nothing.
#[test]
fn export_with_no_name_takes_only_edited_plugins() {
    let p = init();
    ok(p.path(), &["plugin", "add", "develop"]);
    let out = ok(p.path(), &["plugin", "export"]);
    assert!(out.contains("nothing to export"), "{out}");
    let req = p.path().join(".decree/scripts/infra/requirements");
    let mut text = fs::read_to_string(&req).unwrap();
    text.push_str("command   jq\n");
    fs::write(&req, text).unwrap();
    let doc = json(p.path(), &["plugin", "export", "--dry-run"]);
    assert_eq!(doc["exported"], serde_json::json!(["infra"]));
    assert_eq!(doc["dry_run"], true);
    assert_eq!(
        fs::read_to_string(p.path().join(".decree/plugins/infra.md")).unwrap(),
        built_in("infra")
    );
    let out = ok(p.path(), &["plugin", "export"]);
    assert_eq!(out, "wrote .decree/plugins/infra.md\n");
    let infra = fs::read_to_string(p.path().join(".decree/plugins/infra.md")).unwrap();
    assert!(infra.contains("\n    command   jq\n"), "{infra}");
    // develop keeps its `needs` (infra), which its machine does not show.
    ok(p.path(), &["plugin", "export", "develop"]);
    let develop = fs::read_to_string(p.path().join(".decree/plugins/develop.md")).unwrap();
    assert_eq!(develop, built_in("develop"));
}
