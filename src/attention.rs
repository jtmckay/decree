//! `.decree/attention/` (docs/reference/cli.md, Attention): one markdown file per run that
//! needs a person, as `processed.md` records what is done. decree writes `<id>.md` when run
//! `<id>` becomes `failed`, `interrupted` or `waiting` in a `person` state, and for a
//! migration that waits for an interrupted child run; it deletes the file when the run
//! becomes `active` or `pending`, finishes in another state, or is pruned. It does so as the
//! status changes, never by rescanning, so a file a person deleted stays deleted.
//!
//! The files are read-only for people: answers go through `decree event` or a reply, and a
//! run continues with `decree process --retry`. A file with `holds: true` (its run has a
//! `hold` file), or any `.md` file decree cannot read as one of its own, holds the queue:
//! `process` and `daemon` start and continue no runs until it is gone. decree never rewrites
//! or deletes a file it did not write.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::DateTime;
use serde_json::{json, Map, Value};

use crate::events::{current_state, first_text, is_type, strings, text, waiting_child, Event};
use crate::interpreter::recover::RunStatus;
use crate::interpreter::{io_err, Context, InterpreterError};
use crate::layout::{ATTENTION_DIR, DECREE_DIR, HOLD_FILE, MESSAGE_FILE, STOP_FILE};
use crate::machine::{Invoke, FAILED};
use crate::message::{lock_state, run_ids, write_replace, LockState, Message};

/// The `kind` of a file decree writes for a run.
pub const KIND_RUN: &str = "run";

/// The frontmatter keys of a run's attention file, in the order decree writes them
/// (`schema/v1/attention.schema.json`).
const KEYS: [&str; 8] = [
    "kind", "run", "machine", "message", "status", "state", "since", "holds",
];

/// The `status` values of a run's attention file.
const STATUSES: [&str; 3] = ["failed", "interrupted", "waiting"];

/// `.decree/attention/`.
pub fn dir(project_root: &Path) -> PathBuf {
    project_root.join(DECREE_DIR).join(ATTENTION_DIR)
}

/// One `.md` file in `.decree/attention/`.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// The file name, `<id>.md`.
    pub name: String,
    /// The frontmatter, valid against `attention.schema.json`, or why it is not.
    pub frontmatter: Result<Map<String, Value>, Unreadable>,
}

/// A file decree cannot read as an attention file: it holds the queue.
#[derive(Debug, Clone, PartialEq)]
pub struct Unreadable {
    /// `no frontmatter`, `invalid frontmatter: <error>` or `cannot be read: <error>`.
    pub why: String,
    /// The file's first line that has anything but whitespace, if it could be read.
    pub first_line: String,
}

impl Entry {
    /// Read `.decree/attention/<name>`.
    fn read(path: &Path, name: String) -> Entry {
        let frontmatter = match fs::read(path) {
            Err(e) => Err(Unreadable {
                why: format!("cannot be read: {e}"),
                first_line: String::new(),
            }),
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                let first_line = text
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .unwrap_or_default()
                    .to_string();
                parse(&text).map_err(|why| Unreadable { why, first_line })
            }
        };
        Entry { name, frontmatter }
    }

    /// The path from the project root, `.decree/attention/<name>`.
    pub fn path(&self) -> String {
        format!("{DECREE_DIR}/{ATTENTION_DIR}/{}", self.name)
    }

    /// Whether the file holds the queue: `holds: true`, or a file decree cannot read.
    pub fn holds(&self) -> bool {
        match &self.frontmatter {
            Ok(fm) => fm.get("holds") == Some(&Value::Bool(true)),
            Err(_) => true,
        }
    }

    /// `decree status --format json`: the frontmatter plus `path`; for a file decree cannot
    /// read, `path`, `holds: true` and `error`, why.
    pub fn json(&self) -> Value {
        let mut out = match &self.frontmatter {
            Ok(fm) => fm.clone(),
            Err(u) => {
                let mut out = Map::new();
                out.insert("holds".into(), json!(true));
                out.insert("error".into(), json!(u.why));
                out
            }
        };
        out.insert("path".into(), json!(self.path()));
        out.into()
    }

    /// One line for `decree status` and `process`: the file, then what it says.
    pub fn summary(&self) -> String {
        let holds = if self.holds() {
            "holds the queue: "
        } else {
            ""
        };
        match &self.frontmatter {
            Ok(fm) => {
                let get = |k: &str| fm.get(k).and_then(Value::as_str).unwrap_or_default();
                format!(
                    "{}: {holds}run {} ({}) {} in `{}` since {}",
                    self.path(),
                    get("run"),
                    get("machine"),
                    get("status"),
                    get("state"),
                    get("since"),
                )
            }
            Err(u) if u.first_line.is_empty() => format!("{}: {holds}{}", self.path(), u.why),
            Err(u) => format!("{}: {holds}{}: {}", self.path(), u.why, u.first_line),
        }
    }
}

/// Every `*.md` file directly in `.decree/attention/`, by name; any other file or folder
/// is ignored. A missing folder holds none.
pub fn list(project_root: &Path) -> io::Result<Vec<Entry>> {
    let dir = dir(project_root);
    let read = match fs::read_dir(&dir) {
        Ok(read) => read,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut entries = Vec::new();
    for entry in read {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let is_md = name.ends_with(".md") && !name.starts_with('.');
        // A folder named `x.md` is not a file; a symlink to a file is read through.
        if !is_md || entry.path().is_dir() {
            continue;
        }
        entries.push(Entry::read(&entry.path(), name));
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// The frontmatter of an attention file's text, valid against `attention.schema.json`, or
/// why it is not: `no frontmatter` or `invalid frontmatter: <error>`.
pub fn parse(text: &str) -> Result<Map<String, Value>, String> {
    let fenced = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .lines()
        .next()
        .is_some_and(|l| l.trim_end() == "---");
    if !fenced {
        return Err("no frontmatter".to_string());
    }
    let invalid = |e: String| format!("invalid frontmatter: {e}");
    let message = Message::parse(text).map_err(|(line, e)| invalid(format!("line {line}: {e}")))?;
    let value = serde_json::to_value(&message.frontmatter).map_err(|e| invalid(e.to_string()))?;
    validate(&value).map_err(invalid)?;
    match value {
        Value::Object(map) => Ok(map),
        _ => Err(invalid("not a mapping".to_string())),
    }
}

/// Check `value` against `attention.schema.json`: an object with exactly the keys in
/// `KEYS`, each of its type.
pub fn validate(value: &Value) -> Result<(), String> {
    let Value::Object(map) = value else {
        return Err("not a mapping".to_string());
    };
    if let Some(key) = map.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(format!("unknown key `{key}`"));
    }
    if let Some(key) = KEYS.iter().find(|k| !map.contains_key(**k)) {
        return Err(format!("missing `{key}`"));
    }
    for key in ["run", "machine", "message", "state"] {
        match map.get(key) {
            Some(Value::String(s)) if !s.is_empty() => {}
            _ => return Err(format!("`{key}` is not a non-empty string")),
        }
    }
    if map.get("kind").and_then(Value::as_str) != Some(KIND_RUN) {
        return Err(format!("`kind` is not `{KIND_RUN}`"));
    }
    let status = map.get("status").and_then(Value::as_str);
    if !status.is_some_and(|s| STATUSES.contains(&s)) {
        return Err(format!("`status` is not one of {}", STATUSES.join(", ")));
    }
    let since = map.get("since").and_then(Value::as_str);
    if !since.is_some_and(|s| DateTime::parse_from_rfc3339(s).is_ok()) {
        return Err("`since` is not an RFC 3339 time".to_string());
    }
    if !map.get("holds").is_some_and(Value::is_boolean) {
        return Err("`holds` is not `true` or `false`".to_string());
    }
    Ok(())
}

/// What `process` and `daemon` print, and stop for, while a file holds the queue: each
/// holding file decree wrote in full, and each it cannot read named with why; `None` when
/// nothing holds. First deletes the `hold` of every run whose attention file a person
/// deleted, which releases it.
pub fn held(ctx: &Context) -> Result<Option<String>, InterpreterError> {
    release_dropped_holds(ctx)?;
    let dir = dir(&ctx.project_root);
    let entries = list(&ctx.project_root).map_err(io_err(&dir))?;
    let holding: Vec<&Entry> = entries.iter().filter(|e| e.holds()).collect();
    if holding.is_empty() {
        return Ok(None);
    }
    let mut out = format!(
        "nothing runs while a file in {DECREE_DIR}/{ATTENTION_DIR}/ holds the queue; \
         do what it says, or delete it:"
    );
    for entry in holding {
        out.push('\n');
        match &entry.frontmatter {
            Ok(_) => {
                let text = fs::read_to_string(dir.join(&entry.name)).unwrap_or_default();
                let body = Message::parse(&text).map(|m| m.body).unwrap_or(text);
                out.push_str(&format!("{}:\n{}", entry.path(), body.trim()));
            }
            Err(_) => out.push_str(&entry.summary()),
        }
    }
    Ok(Some(out))
}

/// The `hold` of each run whose attention file is gone, once the run is not active: a
/// person dropped the run, so the queue goes on. decree writes the file under the run's
/// lock, so a run that has just written its `hold` is never caught between.
fn release_dropped_holds(ctx: &Context) -> Result<(), InterpreterError> {
    let runs = ctx.runs_dir();
    let attention = dir(&ctx.project_root);
    for id in run_ids(&runs).map_err(io_err(&runs))? {
        let run_dir = runs.join(&id);
        let hold = run_dir.join(HOLD_FILE);
        if !hold.exists() || attention.join(format!("{id}.md")).exists() {
            continue;
        }
        let lock = lock_state(&run_dir).map_err(io_err(&run_dir))?;
        if matches!(lock, LockState::Live(_)) {
            continue;
        }
        remove_if_present(&hold).map_err(io_err(&hold))?;
    }
    Ok(())
}

/// Write or delete run `id`'s attention file for its status now, then do the same for each
/// run above it, whose status may follow from it (a migration waiting for an interrupted
/// child). The caller holds the run's lock, or knows no process is stepping it.
pub fn record(ctx: &Context, id: &str) -> Result<(), InterpreterError> {
    let mut next = Some(id.to_string());
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = next.take() {
        if !seen.insert(id.clone()) || !ctx.runs_dir().join(&id).is_dir() {
            break;
        }
        record_one(ctx, &id)?;
        let message = ctx.runs_dir().join(&id).join(MESSAGE_FILE);
        next = Message::read(&message)
            .ok()
            .filter(|m| m.text("trigger") == Some("invoke"))
            .and_then(|m| m.text("parent").map(String::from));
    }
    Ok(())
}

/// `decree process --retry` of run `id`: delete its `hold` and its attention file.
pub fn release(ctx: &Context, id: &str) -> Result<(), InterpreterError> {
    let hold = ctx.runs_dir().join(id).join(HOLD_FILE);
    if remove_if_present(&hold).map_err(io_err(&hold))? {
        println!("removed .decree/runs/{id}/{HOLD_FILE}");
    }
    remove(&ctx.project_root, id)
}

/// Delete `.decree/attention/<id>.md` if decree wrote it for run `id`.
pub fn remove(project_root: &Path, id: &str) -> Result<(), InterpreterError> {
    let path = dir(project_root).join(format!("{id}.md"));
    if ours(&path, id) {
        remove_if_present(&path).map_err(io_err(&path))?;
    }
    Ok(())
}

/// Whether decree may write or delete `path` for run `id`: it is missing, or it is a run's
/// attention file decree wrote for `id`.
fn ours(path: &Path, id: &str) -> bool {
    match fs::read_to_string(path) {
        Err(e) => e.kind() == io::ErrorKind::NotFound,
        Ok(text) => parse(&text).is_ok_and(|fm| fm.get("run").and_then(Value::as_str) == Some(id)),
    }
}

fn remove_if_present(path: &Path) -> io::Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// What a run needs from a person now, if anything.
enum Need {
    /// Finished in `failed`.
    Failed,
    /// Stopped by a signal, or by a crash found later.
    Interrupted,
    /// Waits in a `person` state for a reply.
    Person,
    /// A migration waiting for child run `.0`, interrupted in state `.1`.
    Child(String, String),
}

fn record_one(ctx: &Context, id: &str) -> Result<(), InterpreterError> {
    let run_dir = ctx.runs_dir().join(id);
    let events = ctx.events(id)?;
    let status = match ctx.run_machine(&events) {
        Some(m) => ctx.status(m, &events, false),
        None if current_state(&events) == Some(FAILED) => RunStatus::Finished,
        None => RunStatus::Interrupted,
    };
    let message = Message::read(&run_dir.join(MESSAGE_FILE)).ok();
    let trigger = first_text(&events, "trigger").unwrap_or_default();
    let has_parent = message.as_ref().and_then(|m| m.text("parent")).is_some();
    let held = run_dir.join(HOLD_FILE).is_file();
    let need = match status {
        // A child run's failure is its parent's event, unless it holds the queue.
        RunStatus::Finished if current_state(&events) == Some(FAILED) && (!has_parent || held) => {
            Some(Need::Failed)
        }
        RunStatus::Interrupted => Some(Need::Interrupted),
        RunStatus::Waiting => match waiting_child(&events) {
            None => events
                .last()
                .filter(|e| is_type(e, "waiting") && text(e, "wait_id").is_some())
                .map(|_| Need::Person),
            Some(_) if trigger == "migration" => ctx
                .stalled_child(&events)?
                .map(|(child, state)| Need::Child(child, state)),
            Some(_) => None,
        },
        _ => None,
    };
    let path = dir(&ctx.project_root).join(format!("{id}.md"));
    let Some(need) = need else {
        return remove(&ctx.project_root, id);
    };
    if !ours(&path, id) {
        return Ok(());
    }
    let text = render(ctx, id, &events, message.as_ref(), &need, held)?;
    let dir = dir(&ctx.project_root);
    fs::create_dir_all(&dir).map_err(io_err(&dir))?;
    write_replace(&path, text.as_bytes())
        .map_err(|(path, source)| InterpreterError::Io { path, source })
}

/// The attention file of run `id`, which needs `need`.
fn render(
    ctx: &Context,
    id: &str,
    events: &[Event],
    message: Option<&Message>,
    need: &Need,
    held: bool,
) -> Result<String, InterpreterError> {
    let run_dir = ctx.runs_dir().join(id);
    let machine = first_text(events, "machine")
        .unwrap_or("unknown")
        .to_string();
    // A failed run is in `failed`; the state it failed from is the one `--retry` continues in.
    let state = match need {
        Need::Failed => events
            .iter()
            .rev()
            .filter(|e| crate::events::is_transition(e))
            .find_map(|e| text(e, "from")),
        _ => None,
    }
    .or_else(|| current_state(events))
    .unwrap_or("none")
    .to_string();
    let last = events.last();
    let since = last
        .and_then(|e| text(e, "ts"))
        .map(String::from)
        .unwrap_or_else(|| crate::events::timestamp(chrono::Utc::now()));
    let migration = first_text(events, "trigger") == Some("migration");
    let file = migration
        .then(|| {
            crate::events::claim_event(events)
                .and_then(|e| text(e, "file"))
                .map(String::from)
        })
        .flatten();
    let status = match need {
        Need::Failed => "failed",
        Need::Interrupted => "interrupted",
        Need::Person | Need::Child(..) => "waiting",
    };
    let message_name = file.clone().unwrap_or_else(|| "inbox".to_string());
    let mut frontmatter = serde_norway::Mapping::new();
    for (key, value) in KEYS.into_iter().zip([
        KIND_RUN.into(),
        id.into(),
        machine.as_str().into(),
        message_name.into(),
        status.into(),
        state.as_str().into(),
        since.as_str().into(),
        serde_norway::Value::Bool(held),
    ]) {
        frontmatter.insert(key.into(), value);
    }
    // A `Mapping` of strings and a bool always serializes.
    let yaml = serde_norway::to_string(&frontmatter).unwrap_or_default();

    let heading = message
        .and_then(|m| {
            m.body
                .lines()
                .find_map(|l| l.strip_prefix("# ").map(str::trim).map(String::from))
        })
        .unwrap_or_else(|| format!("Run {id}"));
    let mut body = format!("# {heading}\n\n");
    let retry = |run: &str| format!("    decree process --retry {run}\n\n");
    match need {
        Need::Failed => {
            body.push_str(&format!(
                "Run `{id}` (machine `{machine}`) failed in `{state}` at {since}; \
                 its logs are in `.decree/runs/{id}/`. Continue it with:\n\n{}",
                retry(id)
            ));
        }
        Need::Interrupted => {
            let cause = match last.and_then(|e| text(e, "cause")) {
                Some("crash") => " (decree stopped without recording it)",
                Some("signal") => " (a signal)",
                _ => "",
            };
            body.push_str(&format!(
                "Run `{id}` (machine `{machine}`) was interrupted in `{state}`{cause} at \
                 {since}. Continue it with:\n\n{}",
                retry(id)
            ));
        }
        Need::Person => body.push_str(&person(ctx, id, events, &state, &since)),
        Need::Child(child, child_state) => {
            body.push_str(&format!(
                "Run `{id}` (machine `{machine}`) waits in `{state}` for child run \
                 `{child}`, which was interrupted in `{child_state}`. Continue the child \
                 with:\n\n{}",
                retry(child)
            ));
        }
    }
    if let Some(file) = &file {
        body.push_str(&format!(
            "This is migration `{file}`: later migrations are blocked until it finishes.\n\n"
        ));
    }
    if held {
        let hold = run_dir.join(HOLD_FILE);
        let text = fs::read_to_string(&hold).unwrap_or_default();
        body.push_str(&format!(
            "## Hold\n\nThis run holds the queue: no run starts or continues until it is \
             retried or this file is deleted. From `.decree/runs/{id}/{HOLD_FILE}`:\n\n{}\n\n",
            text.trim()
        ));
    }
    let stop = run_dir.join(STOP_FILE);
    if let Ok(text) = fs::read_to_string(&stop) {
        body.push_str(&format!(
            "## {STOP_FILE}\n\nThe agent asked, in `.decree/runs/{id}/{STOP_FILE}`:\n\n{}\n\n\
             In `.decree/runs/{id}/`, answer it in plan.md, delete STOP, then \
             decree process --retry {id}\n\n",
            text.trim()
        ));
    }
    Ok(format!(
        "---\n{yaml}---\n{}",
        body.trim_end().to_string() + "\n"
    ))
}

/// The part of the file for a run waiting in a `person` state: the question, each option
/// with its description, and a `decree event` line per option.
fn person(ctx: &Context, id: &str, events: &[Event], state: &str, since: &str) -> String {
    let Some(last) = events.last() else {
        return String::new();
    };
    let wait_id = text(last, "wait_id").unwrap_or_default();
    let machine = ctx.run_machine(events);
    let node = machine.and_then(|m| m.find(state).map(|s| (m, s)));
    let question = node
        .and_then(|(m, s)| match &m.nodes[s].invoke {
            Some(Invoke::Person(p)) => p.question.clone(),
            _ => None,
        })
        .or_else(|| node.and_then(|(m, s)| m.nodes[s].description.clone()))
        .unwrap_or_else(|| format!("Run {id} waits in `{state}`."));
    let mut out = format!(
        "Run `{id}` waits in `{state}` since {since} for a person to answer:\n\n{}\n\nOptions:\n\n",
        question.trim()
    );
    let options = strings(last, "options");
    for option in &options {
        let description = node.and_then(|(m, s)| {
            m.options(s)
                .find(|e| e.event == *option)
                .and_then(|e| e.description.clone())
        });
        match description {
            Some(d) => out.push_str(&format!("- `{option}`: {}\n", d.trim())),
            None => out.push_str(&format!("- `{option}`\n")),
        }
    }
    out.push_str("\nAnswer with one of:\n\n");
    for option in &options {
        out.push_str(&format!("    decree event {wait_id} {option}\n"));
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> Value {
        json!({
            "kind": "run", "run": "01-a", "machine": "develop", "message": "01-a.md",
            "status": "failed", "state": "failed", "since": "2026-10-15T08:09:00.000Z",
            "holds": true,
        })
    }

    #[test]
    fn test_validate_accepts_a_written_file() {
        assert_eq!(validate(&valid()), Ok(()));
    }

    #[test]
    fn test_validate_names_what_is_wrong() {
        let cases = [
            ("status", json!("done"), "`status` is not one of"),
            ("holds", json!("yes"), "`holds` is not"),
            (
                "since",
                json!("yesterday"),
                "`since` is not an RFC 3339 time",
            ),
            ("kind", json!("stop"), "`kind` is not `run`"),
            ("extra", json!(1), "unknown key `extra`"),
        ];
        for (key, value, why) in cases {
            let mut v = valid();
            v[key] = value;
            let err = validate(&v).unwrap_err();
            assert!(err.starts_with(why), "{key}: {err}");
        }
        let mut v = valid();
        v.as_object_mut().unwrap().remove("run");
        assert_eq!(validate(&v).unwrap_err(), "missing `run`");
    }

    #[test]
    fn test_remove_leaves_a_file_decree_did_not_write() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = dir(tmp.path());
        fs::create_dir_all(&dir).unwrap();
        let file = |fm: &Value| {
            let yaml = serde_norway::to_string(fm).unwrap();
            format!("---\n{yaml}---\n# A\n")
        };
        fs::write(dir.join("01-a.md"), "my own note\n").unwrap();
        let mut other = valid();
        other["run"] = json!("02-b");
        fs::write(dir.join("01-b.md"), file(&other)).unwrap();
        fs::write(dir.join("01-c.md"), file(&json!({ "run": "01-c" }))).unwrap();
        for id in ["01-a", "01-b", "01-c"] {
            remove(tmp.path(), id).unwrap();
            assert!(dir.join(format!("{id}.md")).exists(), "{id}");
        }
        // Its own file, for the run it names, goes; a missing one is no error.
        fs::write(dir.join("01-a.md"), file(&valid())).unwrap();
        remove(tmp.path(), "01-a").unwrap();
        assert!(!dir.join("01-a.md").exists());
        remove(tmp.path(), "01-a").unwrap();
    }

    #[test]
    fn test_parse_says_why_a_file_holds() {
        assert_eq!(parse("just a note\n").unwrap_err(), "no frontmatter");
        let err = parse("---\nkind: [\n---\n").unwrap_err();
        assert!(err.starts_with("invalid frontmatter: line"), "{err}");
        let err = parse("---\nkind: run\n---\n").unwrap_err();
        assert!(err.starts_with("invalid frontmatter: missing"), "{err}");
    }
}
