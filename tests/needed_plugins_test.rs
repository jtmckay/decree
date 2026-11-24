//! A15: an official plugin decree copies into `plugins/` because another extends or needs it
//! is recorded as added for a need (`.decree/cache/plugins/<name>.needed`), and
//! `decree install` with no argument removes it, file and installed copy, once no plugin in
//! `plugins/` needs it, as `apt autoremove` does. One a person wrote, or installed by name, or
//! with no record, stays. `decree list` shows `(needed by <names>)`, `--format json`
//! `needed_by`.

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

mod common;
#[path = "common/schema.rs"]
mod schema;

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

/// `decree <args> --format json`, checked against `cli/plugin.schema.json`.
fn json(dir: &Path, args: &[&str]) -> Value {
    let mut args = args.to_vec();
    args.extend(["--format", "json"]);
    let doc: Value = serde_json::from_str(&ok(dir, &args)).unwrap();
    let errors = schema::errors(&schema::validator(schema::CLI_PLUGIN_SCHEMA), &doc);
    assert!(errors.is_empty(), "{errors:?}\n{doc}");
    doc
}

/// An empty project: `.decree/` and nothing in it.
fn empty() -> TempDir {
    let tmp = TempDir::new().unwrap();
    fs::create_dir(tmp.path().join(".decree")).unwrap();
    tmp
}

fn plugin_path(p: &Path, name: &str) -> std::path::PathBuf {
    p.join(format!(".decree/plugins/{name}.md"))
}

fn has_plugin(p: &Path, name: &str) -> bool {
    plugin_path(p, name).is_file()
}

/// A plugin of the project's own with one state, needing `needs` (a YAML flow list).
fn own(name: &str, needs: &str) -> String {
    format!(
        "---\nname: {name}\nneeds: {needs}\nmachine:\n  description: {name}.\n  initial: a\n  states:\n    a: {{ invoke: x, transitions: {{ done: done }} }}\n    done: {{ final: true }}\n    failed: {{ final: true }}\nscripts:\n  x.sh: |\n    #!/usr/bin/env bash\n    true\n---\n# {name}\n"
    )
}

/// Install plugin text `text` from a file, as `decree install <file>`.
fn import(p: &Path, name: &str, text: &str) -> String {
    let file = p.join(format!("{name}.src.md"));
    fs::write(&file, text).unwrap();
    ok(p, &["install", file.to_str().unwrap()])
}

/// The line `decree list` prints for plugin `name`.
fn list_line(list: &str, name: &str) -> String {
    list.lines()
        .find(|l| l.trim_start().starts_with(&format!("{name} ")))
        .unwrap_or_else(|| panic!("{name}: {list}"))
        .to_string()
}

/// `needed_by` of plugin `name` in `decree list --format json`.
fn needed_by(p: &Path, name: &str) -> Value {
    let doc = json(p, &["list"]);
    doc["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == name)
        .unwrap_or_else(|| panic!("{name}: {doc}"))["needed_by"]
        .clone()
}

const ADDED: [&str; 4] = [
    "decree_develop",
    "decree_infra",
    "decree_order",
    "decree_ai",
];

/// AC: `decree install decree_develop_rust` in an empty project adds four plugins for its
/// need; deleting its file, `decree install` removes the four files and their installed
/// copies, `state/` and `order/` staying, each printed `removed <file>, no longer needed`.
#[test]
fn deleting_the_plugin_that_needed_them_removes_the_four_added_plugins() {
    let tmp = empty();
    let p = tmp.path();
    let out = ok(p, &["install", "decree_develop_rust"]);
    for name in ADDED {
        assert!(has_plugin(p, name), "{name}: {out}");
        assert!(
            p.join(format!(".decree/cache/plugins/{name}.needed"))
                .is_file(),
            "{name}"
        );
    }
    assert!(!p
        .join(".decree/cache/plugins/decree_develop_rust.needed")
        .exists());
    assert_eq!(
        needed_by(p, "decree_develop"),
        serde_json::json!(["decree_develop_rust"])
    );
    assert_eq!(needed_by(p, "decree_develop_rust"), Value::Null);
    let list = ok(p, &["list"]);
    assert!(
        list_line(&list, "decree_infra").contains("(needed by decree_develop)"),
        "{list}"
    );
    // The project's own files in state/ and order/ stay.
    let machine = p.join(".decree/machines/decree_develop");
    fs::create_dir_all(machine.join("state")).unwrap();
    fs::write(machine.join("state/notes.txt"), "mine\n").unwrap();
    fs::create_dir_all(machine.join("order")).unwrap();
    fs::write(machine.join("order/0001-x.md"), "---\n---\nx\n").unwrap();

    fs::remove_file(plugin_path(p, "decree_develop_rust")).unwrap();
    let out = ok(p, &["install"]);
    for name in ADDED {
        assert!(
            out.contains(&format!(
                "removed .decree/plugins/{name}.md, no longer needed\n"
            )),
            "{name}: {out}"
        );
        assert!(!has_plugin(p, name), "{name}");
        let dir = p.join(format!(".decree/machines/{name}"));
        assert!(!dir.join("machine.yml").exists(), "{name}");
        assert!(!dir.join("scripts").exists(), "{name}");
        assert!(!dir.join("cron").exists(), "{name}");
        assert!(
            !p.join(format!(".decree/cache/plugins/{name}.needed"))
                .exists(),
            "{name}"
        );
    }
    assert!(out.contains("removed the installed copy of .decree/plugins/decree_develop_rust.md, which was deleted"), "{out}");
    assert_eq!(
        fs::read_to_string(machine.join("state/notes.txt")).unwrap(),
        "mine\n"
    );
    assert!(machine.join("order/0001-x.md").is_file());
    assert!(!p.join(".decree/archive").exists(), "nothing was edited");
    let (code, stdout, stderr) = decree(p, &["check"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    // A second install has nothing more to remove.
    let doc = json(p, &["install"]);
    assert_eq!(doc["no_longer_needed"], serde_json::json!([]), "{doc}");
}

/// AC: `decree install decree_order` of a plugin added for a need marks it asked for, so it
/// stays when the plugin that needed it goes; the other three go.
#[test]
fn a_plugin_installed_by_name_after_it_was_added_for_a_need_stays() {
    let tmp = empty();
    let p = tmp.path();
    ok(p, &["install", "decree_develop_rust"]);
    ok(p, &["install", "decree_order"]);
    assert_eq!(needed_by(p, "decree_order"), Value::Null);
    fs::remove_file(plugin_path(p, "decree_develop_rust")).unwrap();
    let doc = json(p, &["install"]);
    assert_eq!(
        doc["no_longer_needed"],
        serde_json::json!(["decree_ai", "decree_develop", "decree_infra"]),
        "{doc}"
    );
    assert!(has_plugin(p, "decree_order"));
    assert!(p
        .join(".decree/machines/decree_order/machine.yml")
        .is_file());
    for name in ["decree_develop", "decree_infra", "decree_ai"] {
        assert!(!has_plugin(p, name), "{name}");
    }
}

/// AC: with two plugins needing `decree_ai`, deleting one leaves it, needed by the other.
#[test]
fn a_plugin_still_needed_by_another_stays() {
    let tmp = empty();
    let p = tmp.path();
    import(p, "one", &own("one", "[decree_ai]"));
    import(p, "two", &own("two", "[decree_ai]"));
    assert_eq!(needed_by(p, "decree_ai"), serde_json::json!(["one", "two"]));
    fs::remove_file(plugin_path(p, "one")).unwrap();
    let out = ok(p, &["install"]);
    assert!(!out.contains("no longer needed"), "{out}");
    assert!(has_plugin(p, "decree_ai"));
    let list = ok(p, &["list"]);
    let line = list_line(&list, "decree_ai");
    assert!(line.contains("(needed by two)"), "{list}");
    assert!(line.ends_with("[official]"), "{list}");
    assert_eq!(needed_by(p, "decree_ai"), serde_json::json!(["two"]));
}

/// AC: a project's own plugin that starts needing a plugin added for another's need keeps it
/// when that other goes.
#[test]
fn a_plugin_that_later_needs_it_keeps_it() {
    let tmp = empty();
    let p = tmp.path();
    import(p, "first", &own("first", "[decree_ai]"));
    // Written by a person, not installed through decree, then installed with the rest.
    fs::write(plugin_path(p, "later"), own("later", "[decree_ai]")).unwrap();
    ok(p, &["install"]);
    fs::remove_file(plugin_path(p, "first")).unwrap();
    let out = ok(p, &["install"]);
    assert!(!out.contains("no longer needed"), "{out}");
    assert!(has_plugin(p, "decree_ai"));
    let list = ok(p, &["list"]);
    assert!(
        list_line(&list, "decree_ai").contains("(needed by later)"),
        "{list}"
    );
}

/// AC: a `needs` map that meets `decree_infra` with the project's own `infra` needs `infra`,
/// so a `decree_infra` added for another plugin's need goes when that plugin does.
#[test]
fn a_need_met_by_the_projects_plugin_does_not_keep_the_official_one() {
    let tmp = empty();
    let p = tmp.path();
    fs::write(plugin_path_dir(p).join("infra.md"), own("infra", "[]")).unwrap();
    import(p, "uses_infra", &own("uses_infra", "[decree_infra]"));
    assert!(has_plugin(p, "decree_infra"));
    fs::write(
        plugin_path(p, "mapped"),
        own("mapped", "[{decree_infra: infra}]"),
    )
    .unwrap();
    ok(p, &["install"]);
    assert_eq!(
        needed_by(p, "decree_infra"),
        serde_json::json!(["uses_infra"])
    );
    fs::remove_file(plugin_path(p, "uses_infra")).unwrap();
    let out = ok(p, &["install"]);
    assert!(
        out.contains("removed .decree/plugins/decree_infra.md, no longer needed"),
        "{out}"
    );
    assert!(!has_plugin(p, "decree_infra"));
    assert!(has_plugin(p, "infra"));
    assert!(has_plugin(p, "mapped"));
    assert!(p.join(".decree/machines/infra/machine.yml").is_file());
}

fn plugin_path_dir(p: &Path) -> std::path::PathBuf {
    let dir = p.join(".decree/plugins");
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// AC: a `plugins/decree_ai.md` with no record (rc.1-rc.3, a fresh clone, a lost cache)
/// stays though nothing needs it, and so does one a person copied there.
#[test]
fn a_plugin_without_a_record_stays() {
    let tmp = empty();
    let p = tmp.path();
    import(p, "one", &own("one", "[decree_ai]"));
    fs::remove_dir_all(p.join(".decree/cache")).unwrap();
    fs::remove_file(plugin_path(p, "one")).unwrap();
    let out = ok(p, &["install"]);
    assert!(!out.contains("no longer needed"), "{out}");
    assert!(has_plugin(p, "decree_ai"));
    assert_eq!(needed_by(p, "decree_ai"), Value::Null);

    // Installed by name: asked for, so it stays too.
    let tmp = empty();
    let p = tmp.path();
    ok(p, &["install", "decree_ai"]);
    ok(p, &["install"]);
    assert!(has_plugin(p, "decree_ai"));
}

/// AC: an installed copy of a plugin added for a need, edited in place, is archived in
/// `.decree/archive/<name>/` before it is removed.
#[test]
fn an_edited_copy_is_archived_before_it_is_removed() {
    let tmp = empty();
    let p = tmp.path();
    import(p, "one", &own("one", "[decree_ai]"));
    let script = p.join(".decree/machines/decree_ai/scripts/ai.sh");
    assert!(script.is_file());
    fs::write(&script, "#!/usr/bin/env bash\necho edited\n").unwrap();
    fs::remove_file(plugin_path(p, "one")).unwrap();
    let out = ok(p, &["install"]);
    assert!(
        out.contains("removed .decree/plugins/decree_ai.md, no longer needed"),
        "{out}"
    );
    assert!(!script.exists());
    let archive = p.join(".decree/archive/decree_ai");
    let files: Vec<String> = fs::read_dir(&archive)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    let kept = fs::read_to_string(archive.join(&files[0])).unwrap();
    assert!(kept.contains("echo edited"), "{kept}");
    assert!(out.contains(".decree/archive/decree_ai/"), "{out}");
}

/// A chain of plugins added for a need that only need each other goes together, and a
/// plugin file that fails its schema holds every removal, since what it needs is not known.
#[test]
fn an_invalid_plugin_file_holds_removal() {
    let tmp = empty();
    let p = tmp.path();
    import(p, "one", &own("one", "[decree_infra]"));
    fs::remove_file(plugin_path(p, "one")).unwrap();
    fs::write(
        plugin_path(p, "broken"),
        "---\nname: broken\nnope: 1\n---\n",
    )
    .unwrap();
    let (code, out, _) = decree(p, &["install"]);
    assert_eq!(code, 1);
    assert!(!out.contains("no longer needed"), "{out}");
    assert!(has_plugin(p, "decree_infra"));
    fs::remove_file(plugin_path(p, "broken")).unwrap();
    let out = ok(p, &["install"]);
    for name in ["decree_infra", "decree_ai", "decree_order"] {
        assert!(
            out.contains(&format!(
                "removed .decree/plugins/{name}.md, no longer needed"
            )),
            "{name}: {out}"
        );
    }
}

/// `decree check` and `decree daemon` say which plugins added for a need `decree install`
/// would remove, here once a `needs` change, not a deleted file, leaves one unneeded, and
/// remove nothing themselves.
#[test]
fn check_and_daemon_name_an_unneeded_plugin_and_remove_nothing() {
    let tmp = empty();
    let p = tmp.path();
    import(p, "uses_infra", &own("uses_infra", "[decree_infra]"));
    let (code, stdout, stderr) = decree(p, &["check"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(!format!("{stdout}{stderr}").contains("nothing needs it now"));

    fs::write(plugin_path(p, "uses_infra"), own("uses_infra", "[]")).unwrap();
    let warning = "plugins/decree_infra.md: added for a need, and nothing needs it now: run `decree install` to remove it";
    let (code, stdout, stderr) = decree(p, &["check"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        format!("{stdout}{stderr}").contains(warning),
        "{stdout}{stderr}"
    );
    let (_, stdout, stderr) = decree(p, &["daemon", "--once"]);
    assert!(
        format!("{stdout}{stderr}").contains(warning),
        "{stdout}{stderr}"
    );
    assert!(has_plugin(p, "decree_infra"));
    assert!(p
        .join(".decree/machines/decree_infra/machine.yml")
        .is_file());

    let out = ok(p, &["install"]);
    assert!(
        out.contains("removed .decree/plugins/decree_infra.md, no longer needed"),
        "{out}"
    );
    assert!(!has_plugin(p, "decree_infra"));
    let (_, stdout, stderr) = decree(p, &["check"]);
    assert!(!format!("{stdout}{stderr}").contains("nothing needs it now"));
}

/// The plugins `decree init` adds are asked for: a plugin installed later that needs them
/// does not mark them, so when it goes only what it added goes with it.
#[test]
fn the_plugins_init_adds_are_asked_for() {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path();
    ok(p, &["init", "--ai", "claude"]);
    let from_init: Vec<String> = fs::read_dir(p.join(".decree/plugins"))
        .unwrap()
        .map(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .trim_end_matches(".md")
                .to_string()
        })
        .collect();
    assert!(from_init.iter().any(|n| n == "decree_ai"), "{from_init:?}");
    assert!(
        from_init.iter().any(|n| n == "decree_order"),
        "{from_init:?}"
    );
    let out = ok(p, &["install", "decree_develop"]);
    assert!(
        out.contains("added .decree/plugins/decree_infra.md, needed by decree_develop"),
        "{out}"
    );
    for n in &from_init {
        assert_eq!(needed_by(p, n), Value::Null, "{n}");
    }

    fs::remove_file(plugin_path(p, "decree_develop")).unwrap();
    let out = ok(p, &["install"]);
    assert!(
        out.contains("removed .decree/plugins/decree_infra.md, no longer needed"),
        "{out}"
    );
    assert!(!has_plugin(p, "decree_infra"));
    for n in &from_init {
        assert!(has_plugin(p, n), "{n}: {out}");
    }
}
