//! The official plugins built into decree (docs/reference/plugins.md, Official plugins):
//! `decree list` and `decree install` in a fresh `init`, with their JSON output
//! validated against `cli/plugin.schema.json`, and each official plugin, added alone with its
//! `needs` to an empty project, passing `decree check`.

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

mod common;
#[path = "common/schema.rs"]
mod schema;

const OFFICIAL: [&str; 18] = [
    "ai",
    "comfy",
    "develop",
    "develop_by_size",
    "develop_node",
    "develop_react",
    "develop_rust",
    "gliner_router",
    "infra",
    "lease",
    "llm_router",
    "newsletter",
    "order",
    "prune",
    "qa",
    "registry",
    "router",
    "skill",
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

/// A fresh `decree init`, without the default plugins.
fn init() -> TempDir {
    let tmp = TempDir::new().unwrap();
    ok(tmp.path(), &["init", "--ai", "claude"]);
    common::drop_default_plugins(tmp.path());
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

/// AC: after a fresh `init`, `decree install develop` puts `develop` and every plugin it needs in
/// `plugins/`, names those it added as needed, and `decree check` passes.
#[test]
fn add_develop_adds_what_it_needs_and_names_it() {
    let p = init();
    let out = ok(p.path(), &["install", "develop"]);
    assert!(out.contains("added .decree/plugins/develop.md\n"), "{out}");
    assert!(
        out.contains("added .decree/plugins/infra.md, needed by develop\n"),
        "{out}"
    );
    assert!(
        out.contains("added .decree/plugins/ai.md, needed by develop"),
        "{out}"
    );
    let mut files: Vec<String> = fs::read_dir(p.path().join(".decree/plugins"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(files, ["ai.md", "develop.md", "infra.md"]);
    for name in ["develop", "infra"] {
        assert_eq!(
            fs::read_to_string(p.path().join(format!(".decree/plugins/{name}.md"))).unwrap(),
            built_in(name)
        );
        assert!(p
            .path()
            .join(format!(".decree/machines/{name}/machine.yml"))
            .is_file());
    }
    let (code, stdout, stderr) = decree(p.path(), &["check"]);
    assert_eq!((code, stdout.as_str(), stderr.as_str()), (0, "", ""));

    // qa needs develop, which is there: only qa is added, and `needed_by` is empty.
    let doc = json(p.path(), &["install", "qa"]);
    assert_eq!(doc["plugins"].as_array().unwrap().len(), 1, "{doc}");
    assert_eq!(doc["plugins"][0]["status"], "added");
}

/// AC: in a project that added `develop`, edited its gate and exported it, `list`
/// marks `develop` added and differs; `decree install develop` without `--official` installs the
/// project's file; `--official` shows the gate edit dropped and makes the file the built-in copy.
#[test]
fn an_edited_plugin_is_installed_from_its_file_and_replaced_only_with_force() {
    let p = init();
    ok(p.path(), &["install", "develop"]);
    let gate = p.path().join(".decree/machines/develop/scripts/gate.sh");
    let text = fs::read_to_string(&gate).unwrap();
    let edited = text.replacen(
        "  echo \"gate: no checks configured;",
        "  cargo test --workspace\n  echo \"gate: no checks configured;",
        1,
    );
    assert_ne!(text, edited);
    fs::write(&gate, edited).unwrap();
    ok(p.path(), &["export", "develop"]);
    let plugin = p.path().join(".decree/plugins/develop.md");
    let exported = fs::read_to_string(&plugin).unwrap();
    assert!(exported.contains("cargo test --workspace"));

    let list = ok(p.path(), &["list"]);
    let line = list
        .lines()
        .find(|l| l.trim_start().starts_with("develop "))
        .unwrap();
    assert!(line.ends_with("[official, differs]"), "{list}");
    let infra = list
        .lines()
        .find(|l| l.trim_start().starts_with("infra "))
        .unwrap();
    assert!(infra.ends_with("[official]"), "{list}");
    let doc = json(p.path(), &["list"]);
    let names = |key: &str| -> Vec<String> {
        doc[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(names("plugins"), ["ai", "develop", "infra"]);
    let others: Vec<&str> = OFFICIAL
        .into_iter()
        .filter(|n| !["ai", "develop", "infra"].contains(n))
        .collect();
    assert_eq!(names("available"), others);
    let develop = &doc["plugins"][1];
    assert_eq!(
        (&develop["official"], &develop["differs"], &develop["state"]),
        (&true.into(), &true.into(), &"installed".into())
    );

    let before = fs::read_to_string(&gate).unwrap();
    let out = ok(p.path(), &["install", "develop"]);
    assert_eq!(out, "installed develop\n");
    assert_eq!(fs::read_to_string(&plugin).unwrap(), exported);
    assert_eq!(fs::read_to_string(&gate).unwrap(), before);

    let out = ok(p.path(), &["install", "--official", "develop"]);
    assert!(out.contains("replaced .decree/plugins/develop.md"), "{out}");
    assert!(
        out.contains("  changed .decree/machines/develop/scripts/gate.sh\n"),
        "{out}"
    );
    assert!(out.contains("    -   cargo test --workspace\n"), "{out}");
    assert_eq!(fs::read_to_string(&plugin).unwrap(), built_in("develop"));
    assert!(!fs::read_to_string(&gate)
        .unwrap()
        .contains("cargo test --workspace"));
    let list = ok(p.path(), &["list"]);
    assert!(
        list.contains("[official]") && !list.contains("differs"),
        "{list}"
    );
}

/// AC: `decree list` marks a plugin decree does not ship `own`, a file changed since it was
/// installed `not installed`, an installed copy edited in place `edited`, and names a machine
/// no plugin installed.
#[test]
fn list_marks_own_plugins_install_state_and_unplugged_machines() {
    let p = init();
    ok(p.path(), &["install", "develop"]);
    let plugins = p.path().join(".decree/plugins");
    let own = built_in("router").replace("name: router", "name: mine");
    fs::write(plugins.join("mine.md"), own).unwrap();
    let infra = plugins.join("infra.md");
    let text = fs::read_to_string(&infra).unwrap();
    fs::write(&infra, text.replacen("# infra", "# infra, changed", 1)).unwrap();
    let gate = p.path().join(".decree/machines/develop/scripts/gate.sh");
    fs::write(&gate, "#!/usr/bin/env bash\necho edited\n").unwrap();
    let extra = p.path().join(".decree/machines/extra");
    fs::create_dir_all(&extra).unwrap();
    fs::write(
        extra.join("machine.yml"),
        "initial: done\nstates:\n  done:\n    final: true\n",
    )
    .unwrap();

    let list = ok(p.path(), &["list"]);
    let line = |name: &str| {
        list.lines()
            .find(|l| l.trim_start().starts_with(&format!("{name} ")))
            .unwrap_or_else(|| panic!("{name}: {list}"))
            .to_string()
    };
    assert!(line("mine").ends_with("[own, not installed]"), "{list}");
    assert!(
        line("infra").ends_with("[official, differs, not installed]"),
        "{list}"
    );
    assert!(line("develop").ends_with("[official, edited]"), "{list}");
    assert!(
        list.contains(
            "Machines no plugin installed (`decree export <name>` writes one):\n  extra\n"
        ),
        "{list}"
    );
    let doc = json(p.path(), &["list"]);
    assert_eq!(doc["unplugged"], serde_json::json!(["extra"]));
}

/// AC: `decree install nope` exits 1, lists the official names and writes nothing.
#[test]
fn an_unknown_name_exits_1_and_lists_the_official_plugins() {
    let p = init();
    let (code, stdout, stderr) = decree(p.path(), &["install", "develop", "nope"]);
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(
        stderr.contains("no plugin `nope`: not in .decree/plugins/ and not an official plugin"),
        "{stderr}"
    );
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
        ok(tmp.path(), &["install", name]);
        let (code, stdout, stderr) = decree(tmp.path(), &["check"]);
        assert_eq!(
            (code, stdout.as_str(), stderr.as_str()),
            (0, "", ""),
            "{name}"
        );
    }
}

/// `decree export` with no name exports only the plugins whose installed copy
/// differs; `--dry-run` names them and writes nothing.
#[test]
fn export_with_no_name_takes_only_edited_plugins() {
    let p = init();
    ok(p.path(), &["install", "develop"]);
    let out = ok(p.path(), &["export"]);
    assert!(out.contains("nothing to export"), "{out}");
    let req = p.path().join(".decree/machines/infra/scripts/requirements");
    let mut text = fs::read_to_string(&req).unwrap();
    text.push_str("command   jq\n");
    fs::write(&req, text).unwrap();
    let doc = json(p.path(), &["export", "--dry-run"]);
    assert_eq!(doc["exported"], serde_json::json!(["infra"]));
    assert_eq!(doc["dry_run"], true);
    assert_eq!(
        fs::read_to_string(p.path().join(".decree/plugins/infra.md")).unwrap(),
        built_in("infra")
    );
    let out = ok(p.path(), &["export"]);
    assert_eq!(out, "wrote .decree/plugins/infra.md\n");
    let infra = fs::read_to_string(p.path().join(".decree/plugins/infra.md")).unwrap();
    assert!(infra.contains("\n    command   jq\n"), "{infra}");
    // develop keeps its `needs` (infra), which its machine does not show.
    ok(p.path(), &["export", "develop"]);
    let develop = fs::read_to_string(p.path().join(".decree/plugins/develop.md")).unwrap();
    assert_eq!(develop, built_in("develop"));
}

/// AC: a plugin file whose `needs` names an official plugin `plugins/` lacks seeds it, as
/// `decree install <name>` does, and says so; one needing a plugin that is neither fails,
/// writing nothing.
#[test]
fn a_plugin_file_seeds_the_official_plugins_it_needs() {
    let tmp = init();
    let file = tmp.path().join("mine.md");
    std::fs::write(
        &file,
        "---\nname: mine\nneeds: [lease]\nmachine:\n  description: d\n  initial: a\n  states:\n    a: { invoke: x, transitions: { done: done } }\n    done: { final: true }\n    failed: { final: true }\nscripts:\n  x.sh: |\n    #!/usr/bin/env bash\n    true\n---\n",
    )
    .unwrap();
    let out = ok(tmp.path(), &["install", file.to_str().unwrap()]);
    assert!(
        out.contains("added .decree/plugins/lease.md, needed by mine"),
        "{out}"
    );
    assert!(tmp
        .path()
        .join(".decree/machines/lease/machine.yml")
        .exists());
    let other = tmp.path().join("other.md");
    std::fs::write(
        &other,
        std::fs::read_to_string(&file)
            .unwrap()
            .replace("name: mine", "name: other")
            .replace("[lease]", "[nope]"),
    )
    .unwrap();
    let (code, _, err) = decree(tmp.path(), &["install", other.to_str().unwrap()]);
    assert_eq!(code, 1);
    assert!(
        err.contains("needs `nope`, not in .decree/plugins/ and not an official plugin"),
        "{err}"
    );
    assert!(!tmp.path().join(".decree/plugins/other.md").exists());
}
