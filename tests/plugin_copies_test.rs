//! Copies of official plugin files stay the official ones (docs/reference/plugins.md, Official
//! plugins): a file that several plugins carry (`clean_tree.sh`, `commit.sh`, `keep.sh`) is
//! the same in every plugin file of this repository, the escalation fixture's
//! `gliner_router` is the official one, and this repository's own `.decree/plugins/` are the
//! official ones but for the edits it names. Reads this repository's files only.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

mod common;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A plugin file's frontmatter, parsed.
fn frontmatter(path: &Path) -> serde_norway::Value {
    let text = fs::read_to_string(path).unwrap();
    let yaml = text
        .strip_prefix("---\n")
        .and_then(|t| t.split_once("\n---\n"))
        .map(|(fm, _)| format!("{fm}\n"))
        .unwrap_or_else(|| panic!("{}: no frontmatter", path.display()));
    serde_norway::from_str(&yaml).unwrap()
}

/// Every plugin file of this repository, relative to it: the official `plugins/`, each
/// example's and this repository's own `.decree/plugins/`.
fn plugin_files() -> Vec<String> {
    let mut dirs = vec![repo().join("plugins"), repo().join(".decree/plugins")];
    for entry in fs::read_dir(repo().join("examples")).unwrap() {
        dirs.push(entry.unwrap().path().join(".decree/plugins"));
    }
    let mut out = Vec::new();
    for dir in dirs.iter().filter(|d| d.is_dir()) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "md") {
                out.push(path.strip_prefix(repo()).unwrap().display().to_string());
            }
        }
    }
    out.sort();
    out
}

/// Every copy of `clean_tree.sh`, `commit.sh` and `keep.sh` in a plugin file is the official
/// `develop`'s, byte for byte; `ai.sh` is only the `ai` library's, which every plugin whose
/// scripts source it lists in `needs`.
#[test]
fn shared_files_are_the_official_ones_in_every_plugin() {
    let files = plugin_files();
    for must in [
        "plugins/develop.md",
        "plugins/develop_by_size.md",
        ".decree/plugins/develop.md",
        "examples/project/.decree/plugins/develop.md",
    ] {
        assert!(files.iter().any(|f| f == must), "{must} missing: {files:?}");
    }
    let mut found: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for shared in ["clean_tree.sh", "commit.sh", "keep.sh"] {
        let official = common::plugin_script("plugins/develop.md", shared);
        for file in &files {
            let scripts = &frontmatter(&repo().join(file))["scripts"];
            if let Some(text) = scripts.get(shared).and_then(|v| v.as_str()) {
                assert!(
                    text == official,
                    "{file}: {shared} differs from plugins/develop.md's"
                );
                found.entry(shared).or_default().push(file.clone());
            }
        }
    }
    for file in &files {
        let fm = frontmatter(&repo().join(file));
        let name = fm["name"].as_str().unwrap();
        if name != "ai" {
            assert!(fm["scripts"].get("ai.sh").is_none(), "{file} carries ai.sh");
        }
        let sources = fs::read_to_string(repo().join(file))
            .unwrap()
            .contains(". .decree/machines/ai/scripts/ai.sh");
        let needs_ai = fm["needs"]
            .as_sequence()
            .is_some_and(|n| n.iter().any(|v| v.as_str() == Some("ai")));
        if name != "ai" {
            assert_eq!(sources, needs_ai, "{file}: sources ai.sh, needs ai");
        }
    }
    assert!(files
        .iter()
        .any(|f| f == "examples/project/.decree/plugins/ai.md"));
    assert!(found["commit.sh"].contains(&".decree/plugins/develop.md".to_string()));
}

/// The escalation fixture's `gliner_router`, whose recorded runs `replay_test.rs` replays, is
/// the official plugin's: the same machine and the same script.
#[test]
fn the_escalation_fixtures_gliner_router_is_the_official_one() {
    let fixture = repo().join("tests/fixtures/escalation/.decree");
    let machine: serde_norway::Value = serde_norway::from_str(
        &fs::read_to_string(fixture.join("machines/gliner_router/machine.yml")).unwrap(),
    )
    .unwrap();
    let official: serde_norway::Value =
        serde_norway::from_str(&common::plugin_machine("plugins/gliner_router.md")).unwrap();
    assert_eq!(machine, official);
    assert_eq!(
        fs::read_to_string(fixture.join("machines/gliner_router/scripts/ask_gliner.sh")).unwrap(),
        common::plugin_script("plugins/gliner_router.md", "ask_gliner.sh")
    );
}

/// AC (A10): this repository's `.decree/plugins/` keeps its machine names, each started from
/// the official plugin: the same files but for this repository's own edits, which are the
/// cargo gate, the prompts that read `conventions.md`, `conventions.md` itself, done's
/// `restart` with `restart.sh` and `run-queue.sh`, infra's requirements and qa's `QA_RUN`.
#[test]
fn this_repositorys_plugins_are_the_official_ones_plus_its_edits() {
    let edits: &[(&str, &[&str])] = &[
        (
            "develop",
            &[
                "machine",
                "conventions.md",
                "fix.sh",
                "gate.sh",
                "implement.sh",
                "run-queue.sh",
                "verify.sh",
            ],
        ),
        ("infra", &["requirements"]),
        ("qa", &["qa.sh"]),
        ("router", &[]),
        ("order", &[]),
        ("ai", &[]),
        ("skill", &[]),
    ];
    let mut names: Vec<String> = fs::read_dir(repo().join(".decree/plugins"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "ai.md",
            "develop.md",
            "infra.md",
            "order.md",
            "qa.md",
            "router.md",
            "skill.md"
        ]
    );
    for (name, edited) in edits {
        let ours = frontmatter(&repo().join(format!(".decree/plugins/{name}.md")));
        let official = frontmatter(&repo().join(format!("plugins/{name}.md")));
        assert_eq!(ours["needs"], official["needs"], "{name}: needs");
        if !edited.contains(&"machine") {
            assert_eq!(ours["machine"], official["machine"], "{name}: machine");
        }
        let scripts = |v: &serde_norway::Value| -> BTreeMap<String, String> {
            v["scripts"]
                .as_mapping()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.as_str().unwrap().into(), v.as_str().unwrap().into()))
                .collect()
        };
        let (ours, official) = (scripts(&ours), scripts(&official));
        for (path, text) in &official {
            if !edited.contains(&path.as_str()) {
                assert_eq!(ours.get(path), Some(text), "{name}: {path}");
            }
        }
        for path in ours.keys().filter(|p| !official.contains_key(*p)) {
            assert!(
                edited.contains(&path.as_str()),
                "{name}: {path} is not an official file or a named edit"
            );
        }
    }
}
