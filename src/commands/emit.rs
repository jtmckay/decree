//! `decree emit --machine <id> [--param k=v]...` (spec section 8): queue a new message for
//! `<id>` in `inbox/`, with the body read from stdin. Called by scripts: the emitting run
//! is named by `DECREE_MESSAGE_ID`, its state by `DECREE_MACHINE` and `DECREE_STATE`.

use std::io::Read;
use std::path::Path;

use serde_norway::{Mapping, Value};

use crate::commands::check::Project;
use crate::config::RUNS_DIR;
use crate::error::DecreeError;
use crate::machine::{DataType, LoadedMachine};
use crate::message::{self, Message};
use crate::runtime::{MESSAGE_FILE, ROOT_STATE};

/// Run `decree emit`: check the emit against the emitting state's `emits`, `max_depth` and
/// the target machine's `data`, then read stdin and queue the message. Prints the new id.
pub fn run(project_root: &Path, machine: &str, params: &[String]) -> Result<(), DecreeError> {
    let project = Project::load(project_root)?;
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    if let (Some(from), Some(state)) = (var("DECREE_MACHINE"), var("DECREE_STATE")) {
        check_emits(&project, &from, &state, machine)?;
    }
    let mut message = Message::new(String::new());
    message.set("machine", machine);
    if let Some(parent) = var("DECREE_MESSAGE_ID") {
        let depth = parent_depth(&project, &parent)? + 1;
        let max_depth = project.config.max_depth;
        if depth > max_depth {
            return Err(fail(format!(
                "depth {depth} exceeds max_depth {max_depth}: run {parent} is at depth {}",
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
    if let Err(errors) = message::validate(&message, &project.machines, &project.machine_ids, None)
    {
        let errors: Vec<String> = errors.into_iter().map(|(_, e)| e).collect();
        return Err(fail(errors.join("; ")));
    }

    std::io::stdin().read_to_string(&mut message.body)?;
    let id = message::queue(&project.decree_dir, &mut message).map_err(fail)?;
    println!("{id}");
    Ok(())
}

/// `machine` must be in the `emits` of state `state` of machine `from` (section 2).
fn check_emits(project: &Project, from: &str, state: &str, machine: &str) -> Result<(), DecreeError> {
    let m = project
        .machines
        .get(from)
        .ok_or_else(|| fail(format!("DECREE_MACHINE names unknown machine `{from}`")))?;
    let node = if state == ROOT_STATE {
        Some(0)
    } else {
        m.find(state)
    };
    let node = node.map(|i| &m.nodes[i]).ok_or_else(|| {
        fail(format!(
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
    Err(fail(format!(
        "state `{state}` of machine `{from}` may not emit `{machine}`; its `emits`: {allowed}"
    )))
}

/// Frontmatter `depth` of run `parent` (absent means 0).
fn parent_depth(project: &Project, parent: &str) -> Result<u32, DecreeError> {
    if !message::is_valid_id(parent) {
        return Err(fail(format!("DECREE_MESSAGE_ID `{parent}` is not a run id")));
    }
    let path = project
        .decree_dir
        .join(RUNS_DIR)
        .join(parent)
        .join(MESSAGE_FILE);
    let parent_message = Message::read(&path).map_err(|e| {
        fail(format!(
            "cannot read the emitting run {parent} (DECREE_MESSAGE_ID): {e}"
        ))
    })?;
    match parent_message.frontmatter.get("depth") {
        None => Ok(0),
        Some(v) => v.as_u64().and_then(|d| u32::try_from(d).ok()).ok_or_else(|| {
            fail(format!(
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
            fail(format!(
                "machine `{machine}` does not load; run `decree check`"
            ))
        } else {
            fail(format!("unknown machine `{machine}`"))
        }
    })
}

/// `--param k=v` values as YAML values of the type `k` has in the target's `data`: an int
/// or a bool where it parses as one, else a string, so `message::validate` reports a wrong
/// type or an unknown name.
fn parse_params(m: &LoadedMachine, params: &[String]) -> Result<Mapping, DecreeError> {
    let mut mapping = Mapping::new();
    for param in params {
        let (key, text) = param
            .split_once('=')
            .ok_or_else(|| fail(format!("--param `{param}` is not `name=value`")))?;
        let value = match m.data.get(key).map(|d| d.kind) {
            Some(DataType::Int) => text
                .parse::<i64>()
                .map_or_else(|_| Value::from(text), Value::from),
            Some(DataType::Bool) => match text {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => Value::from(text),
            },
            Some(DataType::String) | None => Value::from(text),
        };
        if mapping.insert(Value::from(key), value).is_some() {
            return Err(fail(format!("--param `{key}` is given twice")));
        }
    }
    Ok(mapping)
}

fn fail(e: impl std::fmt::Display) -> DecreeError {
    DecreeError::Other(e.to_string())
}
