//! The JSON Schemas of `src/templates/schema/` (docs/reference/machines.md, Schema), compiled with
//! the `jsonschema` crate (draft 2020-12), and YAML read as the YAML language server sees it:
//! converted to JSON, with the boolean keys `true:` and `false:` as the strings "true" and
//! "false". Used by `schema_test.rs` and `validation_test.rs` through `#[path]`, so the other
//! test files do not compile it.

// Each including test file uses only some of these.
#![allow(dead_code)]

use jsonschema::Validator;
use serde_json::Value as Json;

/// The machine schema, as `decree schema` writes it.
pub const MACHINE_SCHEMA: &str = include_str!("../../src/templates/schema/machine.schema.json");

/// The message frontmatter schema, as `decree schema` writes it.
pub const MESSAGE_SCHEMA: &str = include_str!("../../src/templates/schema/message.schema.json");

/// A draft 2020-12 validator for `schema`.
pub fn validator(schema: &str) -> Validator {
    let schema: Json = serde_json::from_str(schema).unwrap();
    jsonschema::draft202012::new(&schema).unwrap()
}

pub fn machine_validator() -> Validator {
    validator(MACHINE_SCHEMA)
}

pub fn message_validator() -> Validator {
    validator(MESSAGE_SCHEMA)
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
