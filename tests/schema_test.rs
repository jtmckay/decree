//! The JSON Schemas (docs/reference/machines.md, Schema): both are valid draft 2020-12 schemas;
//! every machine in `mock/`, `examples/`, `src/templates/`, this repository and a fresh
//! `decree init` for each `--ai` validates against `machine.schema.json`; every message in
//! `mock/` validates against `message.schema.json`; `decree schema` writes both, and
//! `decree check` warns when they are missing or stale. Whether the schemas reject what
//! `decree check` rejects is tested case by case in `validation_test.rs`.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[path = "common/schema.rs"]
mod schema;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The files in `dir` (not recursive) whose name ends with `suffix`, sorted; none if `dir`
/// does not exist.
fn files_in(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy();
            name.ends_with(suffix) && !name.starts_with('.')
        })
        .collect();
    out.sort();
    out
}

/// The machine files of every project in this repository: `mock/`, each of `examples/`, and
/// the repository's own `.decree/`.
fn project_machines() -> Vec<PathBuf> {
    let mut projects = vec![repo().join("mock"), repo()];
    for entry in fs::read_dir(repo().join("examples")).unwrap() {
        projects.push(entry.unwrap().path());
    }
    let machines: Vec<PathBuf> = projects
        .iter()
        .flat_map(|p| files_in(&p.join(".decree/machines"), ".yml"))
        .collect();
    assert!(machines.len() > 20, "found only {machines:?}");
    machines
}

/// One line per file in `files` that `validate` rejects or cannot read, naming the file.
fn rejected(files: &[(String, String)], validate: impl Fn(&str) -> Option<Vec<String>>) -> String {
    let mut out = Vec::new();
    for (name, text) in files {
        match validate(text) {
            None => out.push(format!("{name}: does not parse")),
            Some(errors) if !errors.is_empty() => {
                out.push(format!("{name}:\n  {}", errors.join("\n  ")))
            }
            Some(_) => {}
        }
    }
    out.join("\n")
}

fn read(paths: &[PathBuf]) -> Vec<(String, String)> {
    paths
        .iter()
        .map(|p| {
            let name = p.strip_prefix(repo()).unwrap_or(p).display().to_string();
            (name, fs::read_to_string(p).unwrap())
        })
        .collect()
}

fn machine_rejects(files: &[(String, String)]) -> String {
    let validator = schema::machine_validator();
    rejected(files, |text| schema::machine_errors(&validator, text))
}

#[test]
fn both_schemas_are_valid_draft_2020_12_schemas() {
    for (name, text) in [
        ("machine", schema::MACHINE_SCHEMA),
        ("message", schema::MESSAGE_SCHEMA),
    ] {
        let schema: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(
            schema["$schema"], "https://json-schema.org/draft/2020-12/schema",
            "{name}"
        );
        if let Err(e) = jsonschema::draft202012::meta::validate(&schema) {
            panic!("{name}.schema.json is not a valid draft 2020-12 schema: {e}");
        }
        assert!(
            schema["$id"].is_string() && schema["title"].is_string(),
            "{name}"
        );
    }
}

/// Subschemas that only narrow keys declared elsewhere: conditional branches, and the defs
/// they reference. Their properties need no `description` of their own.
const NARROWING: &[&str] = &["if", "then", "else", "oneOf", "allOf", "not"];
const NARROWING_DEFS: &[&str] = &[
    "finalState",
    "compoundState",
    "atomicState",
    "options",
    "option",
];

/// Every property either schema declares has a `description`, its own or its `$ref`'s, so
/// an editor shows it on hover and a model reading the schema learns what each key means.
#[test]
fn every_property_has_a_description() {
    fn walk(root: &serde_json::Value, at: &str, value: &serde_json::Value, out: &mut Vec<String>) {
        let Some(map) = value.as_object() else {
            return;
        };
        for (key, prop) in map
            .get("properties")
            .and_then(|p| p.as_object())
            .into_iter()
            .flatten()
        {
            let target = prop
                .get("$ref")
                .and_then(|r| r.as_str())
                .and_then(|r| r.strip_prefix('#'))
                .and_then(|pointer| root.pointer(pointer));
            let described = prop.is_boolean()
                || prop.get("description").is_some()
                || target.is_some_and(|t| t.get("description").is_some());
            if !described {
                out.push(format!("{at}/properties/{key}"));
            }
        }
        for (key, child) in map {
            if NARROWING.contains(&key.as_str()) {
                continue;
            }
            let children: Vec<(String, &serde_json::Value)> = match (key.as_str(), child) {
                ("$defs", serde_json::Value::Object(defs)) => defs
                    .iter()
                    .filter(|(name, _)| !NARROWING_DEFS.contains(&name.as_str()))
                    .map(|(name, def)| (format!("$defs/{name}"), def))
                    .collect(),
                (_, serde_json::Value::Array(items)) => items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| (format!("{key}/{i}"), item))
                    .collect(),
                _ => vec![(key.clone(), child)],
            };
            for (path, child) in children {
                walk(root, &format!("{at}/{path}"), child, out);
            }
        }
    }
    for (name, text) in [
        ("machine", schema::MACHINE_SCHEMA),
        ("message", schema::MESSAGE_SCHEMA),
    ] {
        let schema: serde_json::Value = serde_json::from_str(text).unwrap();
        let mut missing = Vec::new();
        walk(&schema, "#", &schema, &mut missing);
        assert!(missing.is_empty(), "{name}: no description at {missing:?}");
    }
}

#[test]
fn every_machine_in_the_repository_validates() {
    let found = machine_rejects(&read(&project_machines()));
    assert!(found.is_empty(), "{found}");
}

/// The machine templates, with their placeholders filled as `decree init --ai claude` fills
/// them.
#[test]
fn every_machine_template_validates() {
    let templates = repo().join("src/templates");
    let mut paths = files_in(&templates.join("machines"), ".yml");
    paths.push(templates.join("router/router.yml"));
    let files: Vec<(String, String)> = read(&paths)
        .into_iter()
        .map(|(name, text)| {
            let filled = text
                .replace("{ai_title}", "Claude")
                .replace("{ai_cli}", "claude -p")
                .replace("{ai}", "claude");
            assert!(!filled.contains("{ai"), "{name}: unfilled placeholder");
            (name, filled)
        })
        .collect();
    assert_eq!(files.len(), 3);
    let found = machine_rejects(&files);
    assert!(found.is_empty(), "{found}");
}

#[test]
fn every_machine_of_a_fresh_init_validates_for_each_ai() {
    for ai in ["claude", "copilot", "opencode"] {
        let tmp = TempDir::new().unwrap();
        cargo_bin_cmd!("decree")
            .current_dir(tmp.path())
            .env("NO_COLOR", "1")
            .args(["init", "--ai", ai])
            .assert()
            .success();
        let paths = files_in(&tmp.path().join(".decree/machines"), ".yml");
        assert_eq!(paths.len(), 3, "{ai}");
        for path in &paths {
            let text = fs::read_to_string(path).unwrap();
            assert!(
                text.starts_with(
                    "# yaml-language-server: $schema=../schema/machine.schema.json\n# Graph: "
                ),
                "{ai}: {}",
                path.display()
            );
        }
        let found = machine_rejects(&read(&paths));
        assert!(found.is_empty(), "{ai}: {found}");
    }
}

/// Migrations, inbox messages, cron files, and the messages and replies in `mock/`'s runs.
#[test]
fn every_message_in_mock_validates() {
    let decree = repo().join("mock/.decree");
    let mut paths = Vec::new();
    for dir in ["migrations", "inbox", "cron"] {
        paths.extend(files_in(&decree.join(dir), ".md"));
    }
    for run in fs::read_dir(decree.join("runs")).unwrap() {
        let run = run.unwrap().path();
        paths.extend(files_in(&run, "message.md"));
        paths.extend(files_in(&run.join("received"), ".md"));
    }
    let files = read(&paths);
    assert!(
        files.iter().any(|(n, _)| n.contains("/received/")),
        "no reply in mock/"
    );
    assert!(
        files.iter().any(|(n, _)| n.contains("/cron/")),
        "no cron file in mock/"
    );
    let validator = schema::message_validator();
    let found = rejected(&files, |text| schema::message_errors(&validator, text));
    assert!(found.is_empty(), "{found}");
}

/// A machine with every `invoke` kind, long and short.
const EVERY_INVOKE: &str = "\
name: m
description: Every invoke kind.
data:
  max_rounds: { type: int, default: 2 }
  file: { type: string, default: a.pdf }
initial: short
states:
  short:
    invoke: work
    transitions: { done: long }
  long:
    invoke:
      script: { name: work, max_attempts: 2, timeout_s: 60 }
    transitions: { done: named }
  named:
    invoke:
      script: work
    transitions: { done: rounds }
  rounds:
    invoke:
      check: { visits: long, less_than: { data: max_rounds } }
    transitions: { true: pdf, false: done }
  pdf:
    invoke:
      check: { data: file, matches: '\\.pdf$' }
    transitions: { true: text, false: done }
  text:
    invoke:
      check: { output: long, matches: '(?i)invoice' }
    transitions: { true: pick, false: done }
  pick:
    invoke:
      model:
        question: Which way?
        router: router
        min_confidence: 0.8
        output: long
    transitions:
      left:   { target: sure, description: Go left. }
      right:  { target: ask, description: Go right. }
      unsure: ask
  sure:
    invoke:
      check: { confidence: pick, at_least: 0.4 }
    transitions: { true: ask, false: child }
  ask:
    invoke:
      person:
        question: Ship it?
        ask: ask_person
        timeout_s: 86400
    transitions:
      approve: { target: child, description: Ship it. }
      reject:  { target: done, description: Do not ship. }
  child:
    invoke: { machine: hello }
    transitions: { done: params }
  params:
    invoke:
      machine: { name: hello, params: { label: release, rounds: 3, strict: true } }
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

#[test]
fn every_invoke_kind_and_short_form_validates() {
    let found = machine_rejects(&[("every invoke".into(), EVERY_INVOKE.into())]);
    assert!(found.is_empty(), "{found}");
}

/// `true:` and `false:` unquoted are YAML booleans; as JSON keys they are "true" and
/// "false", the events of a `check`, as decree reads them.
#[test]
fn boolean_transition_keys_are_the_events_true_and_false() {
    let json = schema::yaml_to_json("transitions: { true: a, false: b }").unwrap();
    assert_eq!(
        json,
        serde_json::json!({ "transitions": { "true": "a", "false": "b" } })
    );
    let unquoted = EVERY_INVOKE.replace("true: pdf, false: done", "'true': pdf, \"false\": done");
    let found = machine_rejects(&[("quoted".into(), unquoted)]);
    assert!(found.is_empty(), "{found}");
}

/// Mistakes an editor underlines: each is rejected at the key or value that is wrong.
#[test]
fn mistakes_are_rejected_where_they_are() {
    let validator = schema::machine_validator();
    for (from, to, at) in [
        ("    invoke: work\n", "    invok: work\n", "/states/short"),
        (
            "{ name: work, max_attempts: 2,",
            "{ name: work, max_atempts: 2,",
            "/states/long/invoke",
        ),
        (
            "min_confidence: 0.8",
            "min_confidence: 80",
            "/states/pick/invoke",
        ),
        ("at_least: 0.4", "at_least: high", "/states/sure/invoke"),
        (
            "less_than: { data: max_rounds }",
            "less_than: 1.5",
            "/states/rounds/invoke",
        ),
        (
            "{ true: pdf, false: done }",
            "{ yes: pdf, no: done }",
            "/states/rounds/transitions",
        ),
        (
            "Go left. }",
            "Go left. }\n      done:   { target: sure, description: Done. }",
            "/states/pick/transitions/done",
        ),
        (
            "{ target: child, description: Ship it. }",
            "child",
            "/states/ask/transitions/approve",
        ),
        (
            "{ done: long }",
            "{ Done: long }",
            "/states/short/transitions",
        ),
        (
            "  done: { final: true }\n",
            "  done: { final: true, onexit: [notify] }\n",
            "/states/done",
        ),
        ("  failed: { final: true }\n", "", "/states"),
        (
            "{ type: int, default: 2 }",
            "{ type: int, default: two }",
            "/data/max_rounds/default",
        ),
    ] {
        assert!(EVERY_INVOKE.contains(from), "{from}");
        let text = EVERY_INVOKE.replacen(from, to, 1);
        let errors = schema::machine_errors(&validator, &text).unwrap();
        assert!(
            errors
                .iter()
                .any(|e| e.ends_with(&format!("(at {at})")) || e.contains(&format!("(at {at}/"))),
            "{to}: expected an error at {at}, got {errors:?}"
        );
    }
}

/// A project with a machine and no `.decree/schema/`.
fn project() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let decree = tmp.path().join(".decree");
    fs::create_dir_all(decree.join("machines")).unwrap();
    fs::write(
        decree.join("machines/hello.yml"),
        fs::read_to_string(repo().join("mock/.decree/machines/hello.yml")).unwrap(),
    )
    .unwrap();
    common_script(&decree.join("scripts/greet"));
    tmp
}

fn common_script(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "#!/usr/bin/env bash\nexit 0\n").unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// `decree <cmd>` in `dir`: (exit code, stdout, stderr).
fn decree(dir: &Path, cmd: &str) -> (i32, String, String) {
    let out = cargo_bin_cmd!("decree")
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .arg(cmd)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn decree_schema_writes_both_schemas() {
    let tmp = project();
    let (code, stdout, _) = decree(tmp.path(), "schema");
    assert_eq!(
        (code, stdout.as_str()),
        (
            0,
            ".decree/schema/machine.schema.json\n.decree/schema/message.schema.json\n"
        )
    );
    let dir = tmp.path().join(".decree/schema");
    assert_eq!(
        fs::read_to_string(dir.join("machine.schema.json")).unwrap(),
        schema::MACHINE_SCHEMA
    );
    assert_eq!(
        fs::read_to_string(dir.join("message.schema.json")).unwrap(),
        schema::MESSAGE_SCHEMA
    );
    let names: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 2, "a temp file was left: {names:?}");
}

#[test]
fn check_warns_when_the_schema_is_missing_or_stale_until_decree_schema_runs() {
    let tmp = project();
    decree(tmp.path(), "graph");
    let (code, stdout, stderr) = decree(tmp.path(), "check");
    assert_eq!((code, stdout.as_str()), (0, ""));
    assert_eq!(
        stderr,
        "warning: schema/machine.schema.json: missing; run `decree schema`\n\
         warning: schema/message.schema.json: missing; run `decree schema`\n"
    );

    decree(tmp.path(), "schema");
    let dir = tmp.path().join(".decree/schema");
    fs::write(dir.join("machine.schema.json"), "{}\n").unwrap();
    let (code, stdout, stderr) = decree(tmp.path(), "check");
    assert_eq!((code, stdout.as_str()), (0, ""));
    assert_eq!(
        stderr,
        "warning: schema/machine.schema.json: out of date; run `decree schema`\n"
    );

    decree(tmp.path(), "schema");
    assert_eq!(
        decree(tmp.path(), "check"),
        (0, String::new(), String::new())
    );
}
