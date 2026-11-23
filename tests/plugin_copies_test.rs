//! Copies of official plugin files stay the official ones (docs/reference/plugins.md, Official
//! plugins): a file that several plugins carry (`clean_tree.sh`, `commit.sh`, `keep.sh`) is
//! the same in every plugin file of this repository, the escalation fixture's
//! `gliner_router` is the official one, and this repository's own `.decree/plugins/` are the
//! official ones byte for byte, plus its own plugins that extend them with only its edits.
//! Reads this repository's files only.

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
/// `decree_develop`'s, byte for byte; `ai.sh` is only the `decree_ai` library's, which every
/// plugin whose scripts source it lists in `needs`.
#[test]
fn shared_files_are_the_official_ones_in_every_plugin() {
    let files = plugin_files();
    for must in [
        "plugins/decree_develop.md",
        "plugins/decree_develop_by_size.md",
        ".decree/plugins/develop.md",
        "examples/project/.decree/plugins/develop.md",
    ] {
        assert!(files.iter().any(|f| f == must), "{must} missing: {files:?}");
    }
    let mut found: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for shared in ["clean_tree.sh", "commit.sh", "keep.sh"] {
        let official = common::plugin_script("plugins/decree_develop.md", shared);
        for file in &files {
            let scripts = &frontmatter(&repo().join(file))["scripts"];
            if let Some(text) = scripts.get(shared).and_then(|v| v.as_str()) {
                assert!(
                    text == official,
                    "{file}: {shared} differs from plugins/decree_develop.md's"
                );
                found.entry(shared).or_default().push(file.clone());
            }
        }
    }
    for file in &files {
        let fm = frontmatter(&repo().join(file));
        let name = fm["name"].as_str().unwrap();
        if name != "decree_ai" {
            assert!(fm["scripts"].get("ai.sh").is_none(), "{file} carries ai.sh");
        }
        let sources = fs::read_to_string(repo().join(file))
            .unwrap()
            .contains(". \".decree/machines/${DECREE_NEED_DECREE_AI:-decree_ai}/scripts/ai.sh\"");
        let needs_ai = fm["needs"]
            .as_sequence()
            .is_some_and(|n| n.iter().any(|v| v.as_str() == Some("decree_ai")));
        if name != "decree_ai" {
            assert_eq!(sources, needs_ai, "{file}: sources ai.sh, needs ai");
        }
    }
    assert!(files
        .iter()
        .any(|f| f == "examples/project/.decree/plugins/decree_ai.md"));
    assert!(found["commit.sh"].contains(&".decree/plugins/decree_develop.md".to_string()));
}

/// The escalation fixture's `decree_gliner_router`, whose recorded runs `replay_test.rs` replays, is
/// the official plugin's: the same machine and the same script.
#[test]
fn the_escalation_fixtures_gliner_router_is_the_official_one() {
    let fixture = repo().join("tests/fixtures/escalation/.decree");
    let machine: serde_norway::Value = serde_norway::from_str(
        &fs::read_to_string(fixture.join("machines/decree_gliner_router/machine.yml")).unwrap(),
    )
    .unwrap();
    let official: serde_norway::Value =
        serde_norway::from_str(&common::plugin_machine("plugins/decree_gliner_router.md")).unwrap();
    assert_eq!(machine, official);
    assert_eq!(
        fs::read_to_string(fixture.join("machines/decree_gliner_router/scripts/ask_gliner.sh"))
            .unwrap(),
        common::plugin_script("plugins/decree_gliner_router.md", "ask_gliner.sh")
    );
}

/// AC (A14): this repository's `.decree/plugins/` holds each official plugin it runs as this
/// decree's copy, byte for byte, and keeps its machine names `develop`, `infra` and `qa` as
/// plugins of its own that extend the official ones and hold only this repository's edits:
/// the cargo gate, `conventions.md` and `run-queue.sh`; infra's requirements; qa's `QA_RUN`.
#[test]
fn this_repositorys_plugins_are_the_official_ones_plus_its_overlays() {
    let mut names: Vec<String> = fs::read_dir(repo().join(".decree/plugins"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "decree_ai.md",
            "decree_develop.md",
            "decree_infra.md",
            "decree_order.md",
            "decree_qa.md",
            "decree_router.md",
            "decree_skill.md",
            "develop.md",
            "infra.md",
            "qa.md",
        ]
    );
    for name in names.iter().filter(|n| n.starts_with("decree_")) {
        assert_eq!(
            fs::read_to_string(repo().join(".decree/plugins").join(name)).unwrap(),
            fs::read_to_string(repo().join("plugins").join(name)).unwrap(),
            "{name} is not this decree's copy"
        );
    }
    let overlays: &[(&str, &[&str], &str)] = &[
        (
            "develop",
            &["conventions.md", "gate.sh", "run-queue.sh"],
            "- decree_infra: infra\n",
        ),
        ("infra", &["requirements"], ""),
        ("qa", &["qa.sh"], "- decree_develop: develop\n"),
    ];
    for (name, files, needs) in overlays {
        let fm = frontmatter(&repo().join(format!(".decree/plugins/{name}.md")));
        assert_eq!(
            fm["extends"].as_str(),
            Some(format!("decree_{name}").as_str())
        );
        assert!(fm.get("machine").is_none(), "{name}: machine");
        let scripts: Vec<&str> = fm["scripts"]
            .as_mapping()
            .unwrap()
            .keys()
            .map(|k| k.as_str().unwrap())
            .collect();
        assert_eq!(&scripts, files, "{name}");
        let needs = if needs.is_empty() {
            serde_norway::Value::Null
        } else {
            serde_norway::from_str(needs).unwrap()
        };
        assert_eq!(fm["needs"], needs, "{name}: needs");
    }
}
