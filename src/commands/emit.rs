//! `decree emit --machine <id> [--param k=v]...` (docs/reference/cli.md): queue a new message for
//! `<id>` in `inbox/`, with the body read from stdin. Called by scripts: the emitting run
//! is named by `DECREE_MESSAGE_ID`, its state by `DECREE_MACHINE` and `DECREE_STATE`, and its
//! span by `TRACEPARENT`.

use std::io::Read;
use std::path::Path;

use serde_norway::{Mapping, Value};

use crate::cli::Format;
use crate::commands::check::Project;
use crate::commands::print_json;
use crate::error::DecreeError;
use crate::layout::{DECREE_DIR, INBOX_DIR, MESSAGE_FILE, RUNS_DIR};
use crate::machine::{DataType, LoadedMachine};
use crate::message::{self, Message, MAX_DEPTH};
use crate::runtime::ROOT_STATE;
use crate::trace::{
    Incoming, TraceParent, TRACEPARENT_ENV, TRACEPARENT_KEY, TRACESTATE_ENV, TRACESTATE_KEY,
};

/// Run `decree emit`: check the emit against the emitting state's `emits`, `max_depth` and
/// the target machine's `data`, then read stdin and queue the message. Prints the new id.
pub fn run(
    project_root: &Path,
    machine: &str,
    params: &[String],
    format: Format,
) -> Result<(), DecreeError> {
    let project = Project::load(project_root)?;
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    if let (Some(from), Some(state)) = (var("DECREE_MACHINE"), var("DECREE_STATE")) {
        check_emits(&project, &from, &state, machine)?;
    }
    let mut message = Message::new(String::new());
    message.set("machine", machine);
    if let Some(parent) = var("DECREE_MESSAGE_ID") {
        let depth = parent_depth(&project, &parent)? + 1;
        if depth > MAX_DEPTH {
            return Err(DecreeError::Other(format!(
                "depth {depth} exceeds max_depth {MAX_DEPTH}: run {parent} is at depth {}",
                depth - 1
            )));
        }
        message.set("parent", parent.as_str());
        message.set("depth", u64::from(depth));
    }
    message.set("trigger", "emit");
    let target = target(&project, machine)?;
    let params = parse_params(target, params)?;
    if !params.is_empty() {
        message.set("params", Value::Mapping(params));
    }
    // A script's `TRACEPARENT` names its span: the new message's run joins the trace under
    // it (docs/reference/observability.md, Traces). An invalid one is left out.
    let incoming = Incoming::new(
        var(TRACEPARENT_ENV).as_deref(),
        var(TRACESTATE_ENV).as_deref(),
    );
    if let Some(parent) = &incoming.parent {
        message.set(
            TRACEPARENT_KEY,
            TraceParent::format(&parent.trace_id, &parent.parent_id),
        );
    }
    if let Some(tracestate) = incoming.tracestate {
        message.set(TRACESTATE_KEY, tracestate);
    }
    if let Err(errors) = message::validate(&message, &project.machines, &project.machine_ids) {
        let errors: Vec<String> = errors.into_iter().map(|(_, e)| e).collect();
        return Err(DecreeError::Other(errors.join("; ")));
    }

    std::io::stdin().read_to_string(&mut message.body)?;
    let id = message::queue(&project.decree_dir, &mut message)?;
    print_queued(&id, format)
}

/// Print the id of a message just queued in `inbox/`: alone as text, or as
/// `{ "id", "path" }` (`emit` and `event`).
pub(crate) fn print_queued(id: &str, format: Format) -> Result<(), DecreeError> {
    match format {
        Format::Text => {
            println!("{id}");
            Ok(())
        }
        Format::Json => print_json(&serde_json::json!({
            "id": id,
            "path": format!("{DECREE_DIR}/{INBOX_DIR}/{id}.md"),
        })),
    }
}

/// `machine` must be in the `emits` of state `state` of machine `from` (docs/reference/README.md, Architecture).
fn check_emits(
    project: &Project,
    from: &str,
    state: &str,
    machine: &str,
) -> Result<(), DecreeError> {
    let m = project.machines.get(from).ok_or_else(|| {
        DecreeError::Other(format!("DECREE_MACHINE names unknown machine `{from}`"))
    })?;
    let node = if state == ROOT_STATE {
        Some(0)
    } else {
        m.find(state)
    };
    let node = node.map(|i| &m.nodes[i]).ok_or_else(|| {
        DecreeError::Other(format!(
            "DECREE_STATE names unknown state `{state}` of machine `{from}`"
        ))
    })?;
    if node.emits.iter().any(|e| e == machine) {
        return Ok(());
    }
    let allowed = if node.emits.is_empty() {
        "none".to_string()
    } else {
        node.emits.join(", ")
    };
    Err(DecreeError::Other(format!(
        "state `{state}` of machine `{from}` may not emit `{machine}`; its `emits`: {allowed}"
    )))
}

/// Frontmatter `depth` of run `parent` (absent means 0).
fn parent_depth(project: &Project, parent: &str) -> Result<u32, DecreeError> {
    if !message::is_valid_id(parent) {
        return Err(DecreeError::Other(format!(
            "DECREE_MESSAGE_ID `{parent}` is not a run id"
        )));
    }
    let path = project
        .decree_dir
        .join(RUNS_DIR)
        .join(parent)
        .join(MESSAGE_FILE);
    let parent_message = Message::read(&path).map_err(|e| {
        DecreeError::Other(format!(
            "cannot read the emitting run {parent} (DECREE_MESSAGE_ID): {e}"
        ))
    })?;
    match parent_message.frontmatter.get("depth") {
        None => Ok(0),
        Some(v) => v
            .as_u64()
            .and_then(|d| u32::try_from(d).ok())
            .ok_or_else(|| {
                DecreeError::Other(format!(
                    "{}: `depth` is not a non-negative integer",
                    path.display()
                ))
            }),
    }
}

/// The target machine, which must exist and load.
fn target<'p>(project: &'p Project, machine: &str) -> Result<&'p LoadedMachine, DecreeError> {
    project.machines.get(machine).ok_or_else(|| {
        if project.machine_ids.contains(machine) {
            DecreeError::Other(format!(
                "machine `{machine}` does not load; run `decree check`"
            ))
        } else {
            DecreeError::Other(format!("unknown machine `{machine}`"))
        }
    })
}

/// `--param k=v` values as YAML values of the type `k` has in the target's `data`: an int,
/// a number or a bool where it parses as one, else a string, so `message::validate` reports a wrong
/// type or an unknown name.
fn parse_params(m: &LoadedMachine, params: &[String]) -> Result<Mapping, DecreeError> {
    let mut mapping = Mapping::new();
    for param in params {
        let (key, text) = param
            .split_once('=')
            .ok_or_else(|| DecreeError::Other(format!("--param `{param}` is not `name=value`")))?;
        let value = match m.data.get(key).map(|d| d.kind) {
            Some(DataType::Int) => text
                .parse::<i64>()
                .map_or_else(|_| Value::from(text), Value::from),
            Some(DataType::Number) => match (text.parse::<i64>(), text.parse::<f64>()) {
                (Ok(n), _) => Value::from(n),
                (_, Ok(x)) if x.is_finite() => Value::from(x),
                _ => Value::from(text),
            },
            Some(DataType::Bool) => match text {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => Value::from(text),
            },
            Some(DataType::String) | None => Value::from(text),
        };
        if mapping.insert(Value::from(key), value).is_some() {
            return Err(DecreeError::Other(format!(
                "--param `{key}` is given twice"
            )));
        }
    }
    Ok(mapping)
}
