//! The official plugins built into decree (docs/reference/plugins.md, Official plugins):
//! named `decree_*`, and only they; never edited in a project (P3), which extends one under
//! its own name instead (`extends`); `decree list`, `decree install` and `decree export` in a
//! fresh `init`, with their JSON output validated against `cli/plugin.schema.json`; and each
//! official plugin, added alone with what it needs to an empty project, passing `decree check`.

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

mod common;
#[path = "common/schema.rs"]
mod schema;

const OFFICIAL: [&str; 18] = [
    "decree_ai",
    "decree_comfy",
    "decree_develop",
    "decree_develop_by_size",
    "decree_develop_node",
    "decree_develop_react",
    "decree_develop_rust",
    "decree_gliner_router",
    "decree_infra",
    "decree_lease",
    "decree_llm_router",
    "decree_newsletter",
    "decree_order",
    "decree_prune",
    "decree_qa",
    "decree_registry",
    "decree_router",
    "decree_skill",
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

fn read(p: &Path, rel: &str) -> String {
    fs::read_to_string(p.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// The files of `.decree/plugins/`, sorted.
fn plugin_files(p: &Path) -> Vec<String> {
    let mut files: Vec<String> = fs::read_dir(p.join(".decree/plugins"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    files
}

/// The files under `.decree/archive/<name>/`.
fn archives(p: &Path, name: &str) -> Vec<String> {
    let mut out: Vec<String> = fs::read_dir(p.join(".decree/archive").join(name))
        .map(|d| {
            d.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// The line `decree list` prints for plugin `name`.
fn list_line(list: &str, name: &str) -> String {
    list.lines()
        .find(|l| l.trim_start().starts_with(&format!("{name} ")))
        .unwrap_or_else(|| panic!("{name}: {list}"))
        .to_string()
}

/// A plugin of the project's own that extends `decree_develop` with its own gate.
const APP_DEVELOP: &str = "---\nname: app_develop\nextends: decree_develop\nscripts:\n  gate.sh: |\n    #!/usr/bin/env bash\n    echo pass > \"${DECREE_EVENT_FILE}\"\n---\n# app_develop\n\nThe project's develop.\n";

/// AC: after `decree init`, `decree install decree_develop` leaves `plugins/` holding the
/// defaults and `decree_develop` (with what it needs), each this decree's copy byte for byte,
/// `decree list` shows each `official`, and `decree check` passes.
#[test]
fn init_then_install_decree_develop_holds_this_decrees_copies() {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path();
    ok(p, &["init", "--ai", "claude"]);
    let out = ok(p, &["install", "decree_develop"]);
    assert!(
        out.contains("added .decree/plugins/decree_develop.md\n"),
        "{out}"
    );
    assert!(
        out.contains("added .decree/plugins/decree_infra.md, needed by decree_develop\n"),
        "{out}"
    );
    let files = plugin_files(p);
    for name in [
        "decree_ai",
        "decree_order",
        "decree_prune",
        "decree_skill",
        "decree_develop",
        "decree_infra",
    ] {
        let file = format!("{name}.md");
        assert!(files.contains(&file), "{files:?}");
        assert_eq!(read(p, &format!(".decree/plugins/{file}")), built_in(name));
    }
    let list = ok(p, &["list"]);
    for file in &files {
        let name = file.trim_end_matches(".md");
        assert!(list_line(&list, name).ends_with("[official]"), "{list}");
    }
    let (code, stdout, stderr) = decree(p, &["check"]);
    assert_eq!((code, stdout.as_str()), (0, ""), "{stderr}");

    // decree_qa needs decree_develop, which is there: only decree_qa is added.
    let doc = json(p, &["install", "decree_qa"]);
    assert_eq!(doc["plugins"].as_array().unwrap().len(), 1, "{doc}");
    assert_eq!(doc["plugins"][0]["status"], "added");
}

/// AC: a plugin named `decree_mine`, which decree does not ship, and a plugin `decree_app`
/// that extends `decree_lease`, fail `decree check` and `decree install <file>` with P1,
/// saying the prefix is reserved and listing the official plugins; nothing is written.
#[test]
fn the_prefix_is_reserved_for_official_plugins() {
    let machine = "machine:\n  description: d\n  initial: a\n  states:\n    a: { final: true }\n";
    for (name, text) in [
        (
            "decree_mine",
            format!("---\nname: decree_mine\n{machine}---\n# decree_mine\n"),
        ),
        (
            "decree_app",
            "---\nname: decree_app\nextends: decree_lease\n---\n# decree_app\n".to_string(),
        ),
    ] {
        let p = init();
        let file = p.path().join(format!("{name}.md"));
        fs::write(&file, &text).unwrap();
        let before = plugin_files(p.path());
        let (code, _, err) = decree(p.path(), &["install", file.to_str().unwrap()]);
        assert_eq!(code, 1, "{err}");
        assert!(err.contains("which is reserved"), "{err}");
        assert!(err.contains(&OFFICIAL.join(", ")), "{err}");
        assert!(err.contains("(P1)"), "{err}");
        assert_eq!(plugin_files(p.path()), before);
        assert!(!p.path().join(format!(".decree/machines/{name}")).exists());

        fs::write(p.path().join(format!(".decree/plugins/{name}.md")), &text).unwrap();
        let (code, out, _) = decree(p.path(), &["check"]);
        assert_eq!(code, 1, "{out}");
        assert!(
            out.contains(&format!("plugins/{name}.md: line 2: `name` `{name}` starts with `decree_`, which is reserved")),
            "{out}"
        );
        assert!(out.contains(&OFFICIAL.join(", ")), "{out}");
        assert!(out.contains("(P1)"), "{out}");
    }
}

/// AC: a project's own `plugins/develop.md`, with no prefix and another machine, is `own`;
/// `decree install` leaves it and `machines/develop/` as they are and archives nothing; and
/// `decree install decree_develop` adds the official one beside it.
#[test]
fn a_projects_own_develop_is_its_own() {
    let p = init();
    let own = "---\nname: develop\nmachine:\n  description: Mine.\n  initial: a\n  states:\n    a: { invoke: x, transitions: { done: done } }\n    done: { final: true }\n    failed: { final: true }\nscripts:\n  x.sh: |\n    #!/usr/bin/env bash\n    true\n---\n# develop\n";
    fs::write(p.path().join(".decree/plugins/develop.md"), own).unwrap();
    ok(p.path(), &["install"]);
    let machine = read(p.path(), ".decree/machines/develop/machine.yml");
    let list = ok(p.path(), &["list"]);
    assert!(list_line(&list, "develop").ends_with("[own]"), "{list}");
    ok(p.path(), &["install"]);
    assert_eq!(read(p.path(), ".decree/plugins/develop.md"), own);
    assert_eq!(
        read(p.path(), ".decree/machines/develop/machine.yml"),
        machine
    );
    assert!(!p.path().join(".decree/archive").exists());
    ok(p.path(), &["install", "decree_develop"]);
    assert_eq!(read(p.path(), ".decree/plugins/develop.md"), own);
    assert_eq!(
        read(p.path(), ".decree/plugins/decree_develop.md"),
        built_in("decree_develop")
    );
    assert!(p
        .path()
        .join(".decree/machines/decree_develop/machine.yml")
        .is_file());
    assert_eq!(
        read(p.path(), ".decree/machines/develop/machine.yml"),
        machine
    );
}

/// AC: an official plugin edited in its file, or in its installed copy, fails `decree check`
/// with P3 naming the file and the plugin to write instead; `decree export` of it exits 1 with
/// the same advice and writes nothing; `decree install` archives the edit, prints what
/// changed, and restores this decree's copy.
#[test]
fn an_edited_official_plugin_is_p3_and_restored_by_install() {
    for edit_copy in [false, true] {
        let p = init();
        ok(p.path(), &["install", "decree_develop"]);
        let file = p.path().join(".decree/plugins/decree_develop.md");
        let implement = p
            .path()
            .join(".decree/machines/decree_develop/scripts/implement.sh");
        if edit_copy {
            let text = fs::read_to_string(&implement).unwrap();
            fs::write(&implement, format!("{text}echo edited\n")).unwrap();
        } else {
            let text = fs::read_to_string(&file).unwrap();
            fs::write(
                &file,
                text.replacen("# decree_develop\n", "# decree_develop\n\nMine.\n", 1),
            )
            .unwrap();
        }
        let (code, out, _) = decree(p.path(), &["check"]);
        assert_eq!(code, 1, "{out}");
        let named = if edit_copy {
            "machines/decree_develop/scripts/implement.sh: the installed copy of official plugin `decree_develop` is not this decree's copy (machines/decree_develop/scripts/implement.sh)"
        } else {
            "plugins/decree_develop.md: `decree_develop` is an official plugin, and this file is not this decree's copy"
        };
        assert!(out.contains(named), "{out}");
        assert!(out.contains("extends: decree_develop"), "{out}");
        assert!(out.contains("(P3)"), "{out}");
        // Nothing tells a person to export an official plugin: export refuses one.
        assert!(
            !out.contains("`decree export decree_develop` keeps the edit"),
            "{out}"
        );
        assert!(!out.contains("warning: plugins/decree_develop.md"), "{out}");
        let list = ok(p.path(), &["list"]);
        assert!(
            list_line(&list, "decree_develop").contains("differs"),
            "{list}"
        );

        let before = fs::read_to_string(&file).unwrap();
        let (code, _, err) = decree(p.path(), &["export", "decree_develop"]);
        assert_eq!(code, 1);
        assert!(err.contains("extends: decree_develop"), "{err}");
        assert!(err.contains("nothing was written"), "{err}");
        assert_eq!(fs::read_to_string(&file).unwrap(), before);
        let out = ok(p.path(), &["export"]);
        assert!(
            !edit_copy || out.contains("not exported: the installed copy of official plugin `decree_develop` was edited"),
            "{out}"
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), before);

        let out = ok(p.path(), &["install"]);
        if edit_copy {
            assert!(
                out.contains("restored the installed copy of .decree/plugins/decree_develop.md"),
                "{out}"
            );
            assert!(out.contains("    - echo edited\n"), "{out}");
        } else {
            assert!(
                out.contains("replaced .decree/plugins/decree_develop.md with this decree's copy:"),
                "{out}"
            );
            assert!(out.contains("    - Mine.\n"), "{out}");
        }
        // Only real changes are printed: the needs' order is not one.
        assert!(!out.contains("changed needs"), "{out}");
        assert!(
            out.contains("archived what installing `decree_develop` replaced"),
            "{out}"
        );
        assert_eq!(archives(p.path(), "decree_develop").len(), 1);
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            built_in("decree_develop")
        );
        assert!(!fs::read_to_string(&implement)
            .unwrap()
            .contains("echo edited"));
        let (code, out, _) = decree(p.path(), &["check"]);
        assert_eq!(code, 0, "{out}");
    }
}

/// AC: a project's `app_develop` that extends `decree_develop` with its own gate installs both:
/// `machines/decree_develop/` the official one, `machines/app_develop/` the official one with
/// that gate; `decree list` shows `app_develop` as `own`, `extends decree_develop`. Then, under
/// a decree whose `decree_develop` differs in `implement.sh` (the project was installed by an
/// older one), `decree install` replaces the old official file, archiving it and printing the
/// change, gives both installed copies the new `implement.sh`, keeps `app_develop`'s gate, and
/// leaves `plugins/app_develop.md` as it is. An edit of `app_develop`'s gate in place exports
/// as an overlay holding only that gate, and installing and exporting again gives the same bytes.
#[test]
fn an_overlay_installs_on_its_base_and_follows_a_new_base() {
    let p = init();
    let d = p.path().join(".decree");
    ok(p.path(), &["install", "decree_develop"]);
    let overlay = p.path().join("app_develop.md");
    fs::write(&overlay, APP_DEVELOP).unwrap();
    ok(p.path(), &["install", overlay.to_str().unwrap()]);
    let base = common::plugin_script("plugins/decree_develop.md", "implement.sh");
    let gate = read(&d, "machines/app_develop/scripts/gate.sh");
    assert!(gate.contains("echo pass"), "{gate}");
    assert_eq!(read(&d, "machines/app_develop/scripts/implement.sh"), base);
    assert_eq!(
        read(&d, "machines/decree_develop/scripts/gate.sh"),
        common::plugin_script("plugins/decree_develop.md", "gate.sh")
    );
    assert_eq!(
        read(&d, "machines/app_develop/machine.yml"),
        read(&d, "machines/decree_develop/machine.yml")
            .replace("name: decree_develop", "name: app_develop")
    );
    let list = ok(p.path(), &["list"]);
    assert!(
        list_line(&list, "app_develop").ends_with("[own, extends decree_develop]"),
        "{list}"
    );
    let doc = json(p.path(), &["list"]);
    let app = doc["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "app_develop")
        .unwrap();
    assert_eq!(app["extends"], "decree_develop");
    assert_eq!(app["official"], false);
    let (code, out, err) = decree(p.path(), &["check"]);
    assert_eq!((code, out.as_str()), (0, ""), "{err}");

    // The project as an older decree left it: its official file, both installed copies and
    // decree's records of them hold the old implement.sh.
    let old_line = "    echo \"=== AI prompt (implementation) ===\"\n";
    let old_text = "    echo \"=== AI prompt (old implementation) ===\"\n";
    for rel in [
        "plugins/decree_develop.md",
        "cache/plugins/decree_develop.md",
        "cache/plugins/app_develop.md",
    ] {
        let text = read(&d, rel);
        assert!(text.contains(old_line), "{rel}");
        fs::write(d.join(rel), text.replace(old_line, old_text)).unwrap();
    }
    for m in ["decree_develop", "app_develop"] {
        let rel = format!("machines/{m}/scripts/implement.sh");
        let text = read(&d, &rel);
        fs::write(
            d.join(&rel),
            text.replace(old_line.trim_start(), old_text.trim_start()),
        )
        .unwrap();
    }
    let app_file = read(&d, "plugins/app_develop.md");
    let out = ok(p.path(), &["install"]);
    assert!(
        out.contains("replaced .decree/plugins/decree_develop.md with this decree's copy:"),
        "{out}"
    );
    assert!(out.contains("(old implementation)"), "{out}");
    assert_eq!(archives(p.path(), "decree_develop").len(), 1, "{out}");
    assert!(archives(p.path(), "app_develop").is_empty(), "{out}");
    assert_eq!(
        read(&d, "plugins/decree_develop.md"),
        built_in("decree_develop")
    );
    assert_eq!(
        read(&d, "machines/decree_develop/scripts/implement.sh"),
        base
    );
    assert_eq!(read(&d, "machines/app_develop/scripts/implement.sh"), base);
    assert_eq!(read(&d, "machines/app_develop/scripts/gate.sh"), gate);
    assert_eq!(read(&d, "plugins/app_develop.md"), app_file);

    // An edit of the overlay's installed gate exports as the overlay with only that gate.
    let edited = "#!/usr/bin/env bash\necho fail > \"${DECREE_EVENT_FILE}\"\n";
    fs::write(d.join("machines/app_develop/scripts/gate.sh"), edited).unwrap();
    ok(p.path(), &["export", "app_develop"]);
    let exported = read(&d, "plugins/app_develop.md");
    assert!(
        exported.starts_with(
            "---\nname: app_develop\nextends: decree_develop\nscripts:\n  gate.sh: |\n"
        ),
        "{exported}"
    );
    assert!(exported.contains("echo fail"), "{exported}");
    assert!(
        !exported.contains("implement.sh") && !exported.contains("\nmachine:"),
        "{exported}"
    );
    ok(p.path(), &["install"]);
    ok(p.path(), &["export", "app_develop"]);
    assert_eq!(read(&d, "plugins/app_develop.md"), exported);
    let doc = json(p.path(), &["export"]);
    assert_eq!(doc["exported"], serde_json::json!([]));
}

/// AC: `decree check` fails with P1 for a plugin that extends itself, one that extends a
/// library, a chain of `extends` that loops, and a `source` that is not an https URL; and
/// with P2 for one that extends a plugin not in `plugins/`.
#[test]
fn bad_extends_and_source_fail_check() {
    let machine = "machine:\n  description: d\n  initial: a\n  states:\n    a: { final: true }\n";
    let cases: Vec<(&str, String, &str)> = vec![
        (
            "decree_lease",
            "---\nname: decree_lease\nextends: decree_lease\n---\n".to_string(),
            "`extends` names `decree_lease`, the plugin itself",
        ),
        (
            "on_ai",
            "---\nname: on_ai\nextends: decree_ai\n---\n".to_string(),
            "a library (a plugin with no machine), which cannot be extended",
        ),
        (
            "loop_a",
            "---\nname: loop_a\nextends: loop_b\n---\n".to_string(),
            "`extends` loops: loop_a extends loop_b extends loop_a (P1)",
        ),
        (
            "orphan",
            "---\nname: orphan\nextends: nowhere\n---\n".to_string(),
            "`extends` names `nowhere`, not in `plugins/`, so it is not installed",
        ),
        (
            "plain",
            format!("---\nname: plain\nsource: http://example.com/x.md\n{machine}---\n"),
            "`source` is `http://example.com/x.md`: it must be the https:// URL",
        ),
        (
            "named",
            format!("---\nname: named\nsource: official\n{machine}---\n"),
            "`source` is `official`: it must be the https:// URL",
        ),
    ];
    for (name, text, want) in cases {
        let p = init();
        let plugins = p.path().join(".decree/plugins");
        fs::write(plugins.join(format!("{name}.md")), text).unwrap();
        if name == "loop_a" {
            fs::write(
                plugins.join("loop_b.md"),
                "---\nname: loop_b\nextends: loop_a\n---\n",
            )
            .unwrap();
        }
        if name == "on_ai" {
            fs::write(plugins.join("decree_ai.md"), built_in("decree_ai")).unwrap();
        }
        let (code, out, _) = decree(p.path(), &["check"]);
        assert_eq!(code, 1, "{name}: {out}");
        assert!(out.contains(want), "{name}: {out}");
        let rule = if name == "orphan" { "(P2)" } else { "(P1)" };
        assert!(out.contains(rule), "{name}: {out}");
    }
}

/// AC: `decree_develop_rust` is `extends: decree_develop` with only its gate, conventions and
/// requirements; added alone to an empty project it adds `decree_develop` too, `decree check`
/// passes, and its installed machine and other scripts are `decree_develop`'s. `decree list`
/// shows it with `decree_develop`'s description.
#[test]
fn a_develop_variant_installs_on_decree_develop() {
    let text = built_in("decree_develop_rust");
    assert!(text.starts_with("---\nname: decree_develop_rust\nextends: decree_develop\nscripts:\n"));
    assert!(!text.contains("\nmachine:"));
    let tmp = TempDir::new().unwrap();
    fs::create_dir(tmp.path().join(".decree")).unwrap();
    let out = ok(tmp.path(), &["install", "decree_develop_rust"]);
    assert!(
        out.contains("added .decree/plugins/decree_develop.md, needed by decree_develop_rust"),
        "{out}"
    );
    let d = tmp.path().join(".decree");
    assert_eq!(
        read(&d, "machines/decree_develop_rust/machine.yml"),
        read(&d, "machines/decree_develop/machine.yml")
            .replace("name: decree_develop", "name: decree_develop_rust")
    );
    let own = ["conventions.md", "gate.sh", "requirements"];
    for entry in fs::read_dir(d.join("machines/decree_develop/scripts")).unwrap() {
        let file = entry.unwrap().file_name().to_string_lossy().into_owned();
        if !own.contains(&file.as_str()) {
            assert_eq!(
                read(&d, &format!("machines/decree_develop_rust/scripts/{file}")),
                read(&d, &format!("machines/decree_develop/scripts/{file}")),
                "{file}"
            );
        }
    }
    for file in own {
        assert_eq!(
            read(&d, &format!("machines/decree_develop_rust/scripts/{file}")),
            common::plugin_script("plugins/decree_develop_rust.md", file)
        );
    }
    let (code, out, err) = decree(tmp.path(), &["check"]);
    assert_eq!((code, out.as_str(), err.as_str()), (0, "", ""));
    let list = ok(tmp.path(), &["list"]);
    let rust = list_line(&list, "decree_develop_rust");
    assert!(
        rust.ends_with("[official, extends decree_develop]"),
        "{list}"
    );
    let doc = json(tmp.path(), &["list"]);
    let description = |name: &str| {
        doc["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == name)
            .unwrap()["description"]
            .clone()
    };
    assert_eq!(
        description("decree_develop_rust"),
        description("decree_develop")
    );
}

/// AC: `decree list` marks a plugin `own` unless it has the prefix, a file changed since it was
/// installed `not installed`, an installed copy edited in place `edited`, and names a machine
/// no plugin installed.
#[test]
fn list_marks_own_plugins_install_state_and_unplugged_machines() {
    let p = init();
    let plugins = p.path().join(".decree/plugins");
    let own = built_in("decree_router").replace("name: decree_router", "name: mine");
    fs::write(plugins.join("mine.md"), &own).unwrap();
    fs::write(
        plugins.join("other.md"),
        own.replace("name: mine", "name: other"),
    )
    .unwrap();
    ok(p.path(), &["install"]);
    fs::write(
        plugins.join("mine.md"),
        own.replacen("set -euo pipefail", "set -euo pipefail\n    # changed", 1),
    )
    .unwrap();
    let script = p.path().join(".decree/machines/other/scripts/ask.sh");
    fs::write(&script, "#!/usr/bin/env bash\necho edited\n").unwrap();
    let extra = p.path().join(".decree/machines/extra");
    fs::create_dir_all(&extra).unwrap();
    fs::write(
        extra.join("machine.yml"),
        "initial: done\nstates:\n  done:\n    final: true\n",
    )
    .unwrap();

    let list = ok(p.path(), &["list"]);
    assert!(
        list_line(&list, "mine").ends_with("[own, not installed]"),
        "{list}"
    );
    assert!(
        list_line(&list, "other").ends_with("[own, edited]"),
        "{list}"
    );
    assert!(
        list.contains(
            "Machines no plugin installed (`decree export <name>` writes one):\n  extra\n"
        ),
        "{list}"
    );
    let doc = json(p.path(), &["list"]);
    assert_eq!(doc["unplugged"], serde_json::json!(["extra"]));
}

/// AC: `decree install lease` (no `plugins/lease.md`, and no prefix) exits 1, lists the
/// official names and writes nothing; `--official` is gone.
#[test]
fn an_unknown_name_exits_1_and_lists_the_official_plugins() {
    let p = init();
    for name in ["nope", "lease"] {
        let (code, stdout, stderr) = decree(p.path(), &["install", "decree_develop", name]);
        assert_eq!(code, 1, "{stdout}{stderr}");
        assert!(
            stderr.contains(&format!(
                "no plugin `{name}`: not in .decree/plugins/ and not an official plugin"
            )),
            "{stderr}"
        );
        assert!(stderr.contains(&OFFICIAL.join(", ")), "{stderr}");
        assert_eq!(plugin_files(p.path()).len(), 0);
    }
    let (code, _, stderr) = decree(p.path(), &["install", "--official"]);
    assert_eq!(code, 2, "{stderr}");
}

/// AC: each official plugin, added alone with what it extends and needs to an empty project,
/// passes `decree check` without a warning.
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

/// `decree export` with no name exports only the project's own plugins whose installed copy
/// differs; `--dry-run` names them and writes nothing.
#[test]
fn export_with_no_name_takes_only_edited_plugins() {
    let p = init();
    ok(p.path(), &["install", "decree_infra"]);
    let app = p.path().join("app_infra.md");
    fs::write(
        &app,
        "---\nname: app_infra\nextends: decree_infra\n---\n# app_infra\n",
    )
    .unwrap();
    ok(p.path(), &["install", app.to_str().unwrap()]);
    let out = ok(p.path(), &["export"]);
    assert!(out.contains("nothing to export"), "{out}");
    let req = p
        .path()
        .join(".decree/machines/app_infra/scripts/requirements");
    let mut text = fs::read_to_string(&req).unwrap();
    text.push_str("command   jq\n");
    fs::write(&req, text).unwrap();
    let doc = json(p.path(), &["export", "--dry-run"]);
    assert_eq!(doc["exported"], serde_json::json!(["app_infra"]));
    assert_eq!(doc["dry_run"], true);
    let out = ok(p.path(), &["export"]);
    assert_eq!(out, "wrote .decree/plugins/app_infra.md\n");
    let infra = read(p.path(), ".decree/plugins/app_infra.md");
    assert!(infra.contains("extends: decree_infra\n"), "{infra}");
    assert!(infra.contains("\n    command   jq\n"), "{infra}");
    assert!(!infra.contains("\n  run.sh: |"), "{infra}");
}

/// AC: a plugin file that needs or extends an official plugin `plugins/` lacks seeds it, as
/// `decree install <name>` does, and says so; one needing a plugin that is neither fails,
/// writing nothing.
#[test]
fn a_plugin_file_seeds_the_official_plugins_it_needs() {
    let tmp = init();
    let file = tmp.path().join("mine.md");
    std::fs::write(
        &file,
        "---\nname: mine\nneeds: [decree_registry]\nmachine:\n  description: d\n  initial: a\n  states:\n    a: { invoke: x, transitions: { done: done } }\n    done: { final: true }\n    failed: { final: true }\nscripts:\n  x.sh: |\n    #!/usr/bin/env bash\n    true\n---\n",
    )
    .unwrap();
    let out = ok(tmp.path(), &["install", file.to_str().unwrap()]);
    assert!(
        out.contains("added .decree/plugins/decree_registry.md, needed by mine"),
        "{out}"
    );
    assert!(tmp
        .path()
        .join(".decree/machines/decree_registry/machine.yml")
        .exists());
    let other = tmp.path().join("other.md");
    std::fs::write(
        &other,
        std::fs::read_to_string(&file)
            .unwrap()
            .replace("name: mine", "name: other")
            .replace("[decree_registry]", "[nope]"),
    )
    .unwrap();
    let (code, _, err) = decree(tmp.path(), &["install", other.to_str().unwrap()]);
    assert_eq!(code, 1);
    assert!(
        err.contains("needs `nope`, which is not in .decree/plugins/ and not an official plugin"),
        "{err}"
    );
    assert!(!tmp.path().join(".decree/plugins/other.md").exists());
}
