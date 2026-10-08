//! `decree check --format sarif`: the errors and warnings as a SARIF 2.1.0 log, the OASIS
//! standard that code scanning tools read (GitHub code scanning, GitLab, Azure DevOps).
//! Spec: <https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/sarif-v2.1.0-errata01-os-complete.html>.

use serde_json::{json, Value};

use super::{CheckError, CheckWarning};
use crate::layout::DECREE_DIR;

/// The schema URI the SARIF 2.1.0 standard (errata 01) gives for a log's `$schema`.
const SCHEMA: &str =
    "https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json";

/// The reference docs, the driver's `informationUri`.
const INFORMATION_URI: &str =
    "https://github.com/jtmckay/decree/blob/main/docs/reference/README.md";

/// The Validation table, each rule's `helpUri`.
const HELP_URI: &str =
    "https://github.com/jtmckay/decree/blob/main/docs/reference/machines.md#validation";

/// Every rule of docs/reference/machines.md (Validation) with its check, as the table
/// states it (a test holds the two equal).
pub(crate) const RULES: [(&str, &str); 25] = [
    (
        "V1",
        "`name` equals the file stem and matches `^[a-z][a-z0-9_]*$`.",
    ),
    (
        "V2",
        "State ids match the same pattern and are unique across the machine.",
    ),
    (
        "V3",
        "Root and every compound `initial` names a direct child.",
    ),
    (
        "V4",
        "Every transition target exists.",
    ),
    (
        "V5",
        "A root-level final state named `failed` exists.",
    ),
    (
        "V6",
        "Compound states have `initial` and `states`, and no `invoke`.",
    ),
    (
        "V7",
        "Final states have only `final`, `description`, `onentry` and `emits`.",
    ),
    (
        "V8",
        "Decision and sub-machine states cover their events: a `check` state handles `true` and `false` (one still written with `yes` and `no` is told to rename them); a `model` state with `min_confidence` handles `unsure`; a `model` or `person` state has a `question` and at least two options, each with a `description`; a `machine` state handles every root final state of the child except `failed`.",
    ),
    (
        "V9",
        "Every `output`, in a condition or a `model`, names a state with a script invoke.",
    ),
    (
        "V10",
        "Every condition has exactly one subject (`output`, `data`, `visits` or `confidence`) and exactly one operator that subject takes; `visits` names an atomic state; `confidence` names a `model` state and compares to a number from 0 to 1; `data` names existing data, compared to a value of its type, or with `matches` to a regex when its type is `string`; every `matches` compiles as a regular expression.",
    ),
    (
        "V11",
        "Every state is reachable from root `initial`, and every non-final state can reach a root-level final state.",
    ),
    (
        "V12",
        "Every script name (`script`, `ask`, `onentry`, `onexit`, root `onentry`, `onexit`) resolves to exactly one executable file ([Resolution](scripts.md#resolution)); a `person` has an `ask` script.",
    ),
    (
        "V13",
        "Every `emits` entry is an existing machine name.",
    ),
    (
        "V14",
        "Every `data` default matches its `type`. `enum` appears only on `string` data, is a non-empty list of distinct strings, and holds the `default`. Every `store` name matches `^[A-Za-z0-9][A-Za-z0-9._-]*$`, a file or folder directly in the store folder, and has a non-empty description.",
    ),
    (
        "V15",
        "Every compound state with a final child handles `done.state.<id>`, itself or through an ancestor, so the run cannot stall.",
    ),
    (
        "V16",
        "Every `router` and `machine` names an existing machine, and a machine named `router` exists if any `model` names no router; `params` are valid for the child's `data`; `min_confidence` is between 0 and 1; every `timeout` is a [duration](#durations); `attempts` is a positive integer or a non-empty list of values that match `^[A-Za-z0-9][A-Za-z0-9._:/@-]{0,127}$`; every `env` key matches `^[A-Za-z_][A-Za-z0-9_]*$` and is not `DECREE_*`, `TRACEPARENT` or `TRACESTATE`; `attempts`, `timeout` and `env` appear only inside a `script` invoke (and `timeout` inside a `person`), which the parser enforces with V19.",
    ),
    (
        "V17",
        "`type: internal` appears only on a compound state's transition whose target is one of its descendants.",
    ),
    (
        "V18",
        "Event names follow the Rules (a boolean key in `transitions` is the event `true` or `false`); no reserved name is a script-named event or a `model` or `person` option.",
    ),
    (
        "V19",
        "Nothing outside the SCXML subset: unknown keys fail with the name of the SCXML feature, when there is one, and the decree alternative. Each shape this format replaced fails with the one to write instead: `choose` (`model:` or `person:`), `input` (`output`), a bare `matches` (`{ output: <state>, matches: … }`), `max_attempts` anywhere (`attempts: <n>` or `attempts: [<value>, …]`, inside `invoke: { script: … }`), `timeout_s` on a state (inside `invoke: { script: … }`), `timeout_s` in an invoke (`timeout: <n>s|m|h|d`) and `{ machine: x, params: … }` (`{ machine: { name: x, params: … } }`).",
    ),
    (
        "V20",
        "No cycle of `machine` and `router` invokes: a machine never invokes itself, directly or through others.",
    ),
    (
        "V21",
        "Within one state, no transition's event equals another's followed by `.` and more (`done` and `done.state.work`), so at most one of a state's transitions matches any event.",
    ),
    (
        "M1",
        "Every pending migration (not in `processed.md`) parses, names a known machine in `machine:`, and has valid `params` for that machine's `data`.",
    ),
    (
        "M2",
        "Every `inbox/*.md` passes the same checks.",
    ),
    (
        "M3",
        "Every `cron/*.md` passes the same checks, and its `cron:` expression parses.",
    ),
    (
        "E1",
        "`.decree/env`, if it exists, is a dotenv file: each line is blank, a `#` comment or `KEY=value` (optionally after `export `, the value optionally in matching single or double quotes); each key matches `^[A-Za-z_][A-Za-z0-9_]*$` and is not `DECREE_*`, `TRACEPARENT` or `TRACESTATE`.",
    ),
];

/// The SARIF log: one run of the `decree` driver, a `result` per error (`level: error`,
/// with its rule when it has one) and per warning (`level: warning`, no rule). Each result
/// points at its file relative to the project root, and at its line when the error names one.
pub(crate) fn log(errors: &[CheckError], warnings: &[CheckWarning]) -> Value {
    let rules: Vec<Value> = RULES
        .iter()
        .map(|(id, check)| {
            json!({
                "id": id,
                "shortDescription": { "text": check },
                "helpUri": HELP_URI,
            })
        })
        .collect();
    let mut results: Vec<Value> = errors.iter().map(error_result).collect();
    results.extend(warnings.iter().map(|w| {
        json!({
            "level": "warning",
            "message": { "text": w.message },
            "locations": [location(&w.file, None)],
        })
    }));
    json!({
        "$schema": SCHEMA,
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "decree",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": INFORMATION_URI,
                    "rules": rules,
                }
            },
            "results": results,
        }],
    })
}

fn error_result(e: &CheckError) -> Value {
    // Without a line, the state path says where in the file.
    let text = match &e.state {
        Some(state) => format!("{state}: {}", e.message),
        None => e.message.clone(),
    };
    let mut result = json!({
        "level": "error",
        "message": { "text": text },
        "locations": [location(&e.file, e.line)],
    });
    if let Some(rule) = &e.rule {
        result["ruleId"] = rule.as_str().into();
    }
    result
}

/// A `location` for `file`, relative to `.decree/`, as a URI relative to the project root.
fn location(file: &str, line: Option<usize>) -> Value {
    let mut physical = json!({ "artifactLocation": { "uri": format!("{DECREE_DIR}/{file}") } });
    if let Some(line) = line {
        physical["region"] = json!({ "startLine": line });
    }
    json!({ "physicalLocation": physical })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `RULES` is the Validation table of docs/reference/machines.md, row for row.
    #[test]
    fn rules_are_the_validation_table() {
        let doc = include_str!("../../../docs/reference/machines.md");
        let table: Vec<(String, String)> = doc
            .lines()
            .filter_map(|line| {
                let row = line.strip_prefix("| ")?.strip_suffix(" |")?;
                let (id, check) = row.split_once(" | ")?;
                let rule = id.starts_with('V') || id.starts_with('M') || id.starts_with('E');
                (rule && id[1..].parse::<u32>().is_ok())
                    .then(|| (id.to_string(), check.replace("\\|", "|")))
            })
            .collect();
        let rules: Vec<(String, String)> = RULES
            .iter()
            .map(|(id, check)| (id.to_string(), check.to_string()))
            .collect();
        assert_eq!(rules, table);
    }
}
