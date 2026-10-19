//! `decree emit [--machine <id>] [--param k=v]...` (docs/reference/cli.md): queue a new message
//! in `inbox/` from stdin: a body, or a whole message whose frontmatter the flags override. Called by scripts: the emitting run
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
use crate::message::{self, Message, MAX_DEPTH_VAR};
use crate::runtime::ROOT_STATE;
use crate::trace::{
    Incoming, TraceParent, TRACEPARENT_ENV, TRACEPARENT_KEY, TRACESTATE_ENV, TRACESTATE_KEY,
};

/// Run `decree emit`: read stdin as a message, check the emit against the emitting state's
/// `emits`, `max_depth` and the target machine's `data`, then queue it. Stdin that starts
/// with frontmatter is a whole message: its frontmatter, less the keys decree sets, is the new
/// message's, and `--machine` and `--param` override it. Otherwise stdin is the body. Prints
/// the new id.
pub fn run(
    project_root: &Path,
    machine: Option<&str>,
    params: &[String],
    format: Format,
) -> Result<(), DecreeError> {
    let project = Project::load(project_root)?;
    let max_depth = crate::commands::check::max_depth(&project)?;
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    let (mut message, from_stdin) = read_message(text)?;
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());

    // The machine: `--machine`, else the frontmatter's, checked as `decree check` does.
    if let Some(machine) = machine {
        target(&project, machine)?;
        message.set("machine", machine);
    } else if !from_stdin {
        return Err(DecreeError::Other(
            "no machine: pass --machine, or give a whole message on stdin whose frontmatter \
             names one"
                .into(),
        ));
    } else if !message.frontmatter.contains_key("machine") {
        return Err(DecreeError::Other(
            "stdin: line 1: no `machine` key: name one in the frontmatter or pass --machine".into(),
        ));
    }
    let mut named = message.clone();
    named.frontmatter.shift_remove("params");
    let machine = message::validate(&named, &project.machines, &project.machine_ids)
        .map_err(|errors| invalid(errors, from_stdin && machine.is_none()))?;
    let machine = machine.as_str();

    if let (Some(from), Some(state)) = (var("DECREE_MACHINE"), var("DECREE_STATE")) {
        check_emits(&project, &from, &state, machine)?;
    }
    if let Some(parent) = var("DECREE_MESSAGE_ID") {
        let depth = parent_depth(&project, &parent)? + 1;
        if depth > max_depth {
            return Err(DecreeError::Other(format!(
                "depth {depth} exceeds max_depth {max_depth} (set {MAX_DEPTH_VAR} to change it): \
                 run {parent} is at depth {}",
                depth - 1
            )));
        }
        message.set("parent", parent.as_str());
        message.set("depth", u64::from(depth));
    }
    message.set("trigger", "emit");
    let target = target(&project, machine)?;
    let flags = parse_params(target, params)?;
    if !flags.is_empty() {
        let mut params = match message.frontmatter.get("params") {
            Some(Value::Mapping(params)) => params.clone(),
            _ => Mapping::new(),
        };
        params.extend(flags);
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
        return Err(invalid(errors, from_stdin && params.is_empty()));
    }

    let id = message::queue(&project.decree_dir, &mut message)?;
    print_queued(&id, format)
}

/// Stdin as a message, and whether it starts with frontmatter. With frontmatter, the keys
/// decree sets are dropped, as for a file placed in `inbox/` (docs/reference/messages.md,
/// Lifecycle step 1); frontmatter that does not parse is refused with the error
/// `decree check` gives. Without, the whole text is the body, byte for byte.
fn read_message(text: String) -> Result<(Message, bool), DecreeError> {
    let message = Message::parse(&text)
        .map_err(|(line, e)| DecreeError::Other(format!("stdin: line {line}: {e}")))?;
    let unmarked = text.strip_prefix('\u{feff}').unwrap_or(&text);
    if message.body.len() == unmarked.len() {
        return Ok((Message::new(text), false));
    }
    let mut message = message;
    for key in message::DECREE_KEYS {
        message.frontmatter.shift_remove(key);
    }
    Ok((message, true))
}

/// `message::validate` errors as one error: each at its stdin line, as `decree check` gives
/// them, when `at_lines` (the keys checked came from stdin), else without lines.
fn invalid(errors: Vec<(usize, String)>, at_lines: bool) -> DecreeError {
    let errors: Vec<String> = errors
        .into_iter()
        .map(|(line, e)| {
            if at_lines {
                format!("stdin: line {line}: {e}")
            } else {
                e
            }
        })
        .collect();
    DecreeError::Other(errors.join("; "))
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
