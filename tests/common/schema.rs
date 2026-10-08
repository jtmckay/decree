//! The JSON Schemas of `schema/v1/` (docs/reference/README.md, Schemas),
//! compiled with the `jsonschema` crate (draft 2020-12); the lines of `events.jsonl` and
//! JSON files checked against them; and YAML read as the YAML language server sees it:
//! converted to JSON, with the boolean keys `true:` and `false:` as the strings "true" and
//! "false". Used by `schema_test.rs`, `validation_test.rs` and `interpreter_props.rs`
//! through `#[path]`, so the other test files do not compile it.

// Each including test file uses only some of these.
#![allow(dead_code)]

use jsonschema::Validator;
use serde_json::Value as Json;

/// The machine schema, as `decree schema` writes it.
pub const MACHINE_SCHEMA: &str = include_str!("../../schema/v1/machine.schema.json");

/// The message frontmatter schema, as `decree schema` writes it.
pub const MESSAGE_SCHEMA: &str = include_str!("../../schema/v1/message.schema.json");

/// The schema of one line of `events.jsonl`.
pub const EVENTS_SCHEMA: &str = include_str!("../../schema/v1/events.schema.json");

/// The schema of a router's `request.json`.
pub const REQUEST_SCHEMA: &str = include_str!("../../schema/v1/request.schema.json");

/// The schema of a router's `reply.json`.
pub const REPLY_SCHEMA: &str = include_str!("../../schema/v1/reply.schema.json");

/// The schema of `decree check --format json`.
pub const CLI_CHECK_SCHEMA: &str = include_str!("../../schema/v1/cli/check.schema.json");

/// The schema of `decree emit --format json`.
pub const CLI_EMIT_SCHEMA: &str = include_str!("../../schema/v1/cli/emit.schema.json");

/// The schema of `decree event --format json`.
pub const CLI_EVENT_SCHEMA: &str = include_str!("../../schema/v1/cli/event.schema.json");

/// The schema of `decree graph --format json`.
pub const CLI_GRAPH_SCHEMA: &str = include_str!("../../schema/v1/cli/graph.schema.json");

/// The schema of `decree process --format json`.
pub const CLI_PROCESS_SCHEMA: &str = include_str!("../../schema/v1/cli/process.schema.json");

/// The schema of `decree prune --format json`.
pub const CLI_PRUNE_SCHEMA: &str = include_str!("../../schema/v1/cli/prune.schema.json");

/// The schema of `decree schema --format json`.
pub const CLI_SCHEMA_SCHEMA: &str = include_str!("../../schema/v1/cli/schema.schema.json");

/// The schema of `decree skill --format json`.
pub const CLI_SKILL_SCHEMA: &str = include_str!("../../schema/v1/cli/skill.schema.json");

/// The schema of `decree status --format json`.
pub const CLI_STATUS_SCHEMA: &str = include_str!("../../schema/v1/cli/status.schema.json");

/// Every schema `decree schema` writes, by its path under `.decree/schema/v1/` without
/// `.schema.json`.
pub const ALL: [(&str, &str); 14] = [
    ("cli/check", CLI_CHECK_SCHEMA),
    ("cli/emit", CLI_EMIT_SCHEMA),
    ("cli/event", CLI_EVENT_SCHEMA),
    ("cli/graph", CLI_GRAPH_SCHEMA),
    ("cli/process", CLI_PROCESS_SCHEMA),
    ("cli/prune", CLI_PRUNE_SCHEMA),
    ("cli/schema", CLI_SCHEMA_SCHEMA),
    ("cli/skill", CLI_SKILL_SCHEMA),
    ("cli/status", CLI_STATUS_SCHEMA),
    ("events", EVENTS_SCHEMA),
    ("machine", MACHINE_SCHEMA),
    ("message", MESSAGE_SCHEMA),
    ("reply", REPLY_SCHEMA),
    ("request", REQUEST_SCHEMA),
];

/// A draft 2020-12 validator for `schema`. A `$ref` to another of `ALL` (as the `status`
/// document's `events` refer to `events.schema.json`) resolves to it, offline.
pub fn validator(schema: &str) -> Validator {
    let schema: Json = serde_json::from_str(schema).unwrap();
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .with_retriever(Bundled)
        .build(&schema)
        .unwrap()
}

/// Resolves the `$id` of each schema in `ALL` to its text, and nothing else: tests never
/// fetch.
struct Bundled;

impl jsonschema::Retrieve for Bundled {
    fn retrieve(
        &self,
        uri: &jsonschema::Uri<String>,
    ) -> Result<Json, Box<dyn std::error::Error + Send + Sync>> {
        let prefix = "https://raw.githubusercontent.com/jtmckay/decree/main/schema/v1/";
        let name = uri
            .as_str()
            .strip_prefix(prefix)
            .and_then(|path| path.strip_suffix(".schema.json"));
        match ALL.iter().find(|(n, _)| Some(*n) == name) {
            Some((_, text)) => Ok(serde_json::from_str(text)?),
            None => Err(format!("not a decree schema: {}", uri.as_str()).into()),
        }
    }
}

pub fn machine_validator() -> Validator {
    validator(MACHINE_SCHEMA)
}

pub fn message_validator() -> Validator {
    validator(MESSAGE_SCHEMA)
}

pub fn events_validator() -> Validator {
    validator(EVENTS_SCHEMA)
}

/// The schema errors of each line of `events.jsonl` text, one line each naming the line
/// number; a line that is not JSON is an error too.
pub fn events_errors(validator: &Validator, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (n, line) in text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
    {
        match serde_json::from_str::<Json>(line) {
            Ok(event) => out.extend(
                errors(validator, &event)
                    .into_iter()
                    .map(|e| format!("line {}: {e}", n + 1)),
            ),
            Err(e) => out.push(format!("line {}: not JSON: {e}", n + 1)),
        }
    }
    out
}

/// The schema errors of a JSON file's text, or `None` if it is not JSON.
pub fn json_errors(validator: &Validator, text: &str) -> Option<Vec<String>> {
    serde_json::from_str::<Json>(text)
        .ok()
        .map(|json| errors(validator, &json))
}

/// YAML as JSON: map keys become strings (`true` becomes "true"), as JSON requires. `None`
/// if the YAML does not parse.
pub fn yaml_to_json(yaml: &str) -> Option<Json> {
    let value: serde_norway::Value = serde_norway::from_str(yaml).ok()?;
    Some(serde_json::to_value(value).unwrap())
}

/// A message's frontmatter as JSON, following docs/reference/messages.md (Parsing and
/// writing): a leading BOM is ignored, no opening `---` means an empty map. `None` if it
/// does not parse (no closing `---`, a duplicate key, bad YAML).
pub fn frontmatter_to_json(text: &str) -> Option<Json> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.split_inclusive('\n');
    if lines.next().map(str::trim_end) != Some("---") {
        return Some(Json::Object(Default::default()));
    }
    let mut yaml = String::new();
    for line in lines {
        if line.trim_end() == "---" {
            return match yaml.trim() {
                "" => Some(Json::Object(Default::default())),
                _ => yaml_to_json(&yaml),
            };
        }
        yaml.push_str(line);
    }
    None
}

/// Every schema error in `instance`, one line each: the message and where it is.
pub fn errors(validator: &Validator, instance: &Json) -> Vec<String> {
    validator
        .iter_errors(instance)
        .map(|e| format!("{e} (at {})", e.instance_path()))
        .collect()
}

/// The schema errors of the machine file `yaml`, or `None` if it is not YAML.
pub fn machine_errors(validator: &Validator, yaml: &str) -> Option<Vec<String>> {
    yaml_to_json(yaml).map(|json| errors(validator, &json))
}

/// The schema errors of a message file's frontmatter, or `None` if it does not parse.
pub fn message_errors(validator: &Validator, text: &str) -> Option<Vec<String>> {
    frontmatter_to_json(text).map(|json| errors(validator, &json))
}
