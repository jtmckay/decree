//! `.decree/attention/` (docs/reference/cli.md, Attention): one markdown file per run that
//! needs a person. decree writes `<id>.md` when run
//! `<id>` becomes `failed`, `interrupted` or `waiting` in a `reply` state; it deletes the file
//! when the run becomes `active` or `pending`, finishes in another state, or is pruned. It does so as the
//! status changes, never by rescanning, so a file a person deleted stays deleted.
//!
//! The files are read-only for people: answers go through `decree reply` or a reply, and a
//! run continues with `decree retry`. A run's file with `holds: true` (its run has a `hold`
//! file) holds the runs that share its machine's slots, its `concurrency_group` or the machine
//! alone: they neither start nor continue until it is gone. `pause.md`, and any `.md` file
//! decree cannot read as one of its own, holds every run. decree never rewrites or deletes a
//! file it did not write.
//!
//! `decree pause` writes the one file that is not about a run, `pause.md` (`kind: pause`): it
//! holds the queue until `decree resume` deletes it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::DateTime;
use serde_json::{json, Map, Value};

use crate::events::{current_state, first_text, is_type, strings, text, waiting_child, Event};
use crate::interpreter::recover::RunStatus;
use crate::interpreter::{io_err, Context, InterpreterError};
use crate::layout::{
    ATTENTION_DIR, DECREE_DIR, HOLD_FILE, LEFTOVERS_FILE, MESSAGE_FILE, STOP_FILE,
};
use crate::machine::{Invoke, FAILED};
use crate::message::{lock_state, run_ids, write_replace, LockState, Message};

/// The `kind` of a file decree writes for a run.
pub const KIND_RUN: &str = "run";

/// The `kind` of the file `decree pause` writes.
pub const KIND_PAUSE: &str = "pause";

/// The file `decree pause` writes in `.decree/attention/`, and `decree resume` deletes.
pub const PAUSE_FILE: &str = "pause.md";

/// The frontmatter keys of the file `decree pause` writes, in the order it writes them.
const PAUSE_KEYS: [&str; 3] = ["kind", "since", "holds"];

/// The frontmatter keys of a run's attention file, in the order decree writes them
/// (`schema/v1/attention.schema.json`). All are required but `machine`, which is absent for
/// a run with no machine of its own (`machine_of`).
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

    /// What the file holds, if it holds: the runs of one slot key (`group/<g>` or
    /// `machine/<m>`) for a run's file whose machine is loaded, else every run.
    pub fn scope(&self, ctx: &Context) -> Option<Scope> {
        if !self.holds() {
            return None;
        }
        let machine = self.frontmatter.as_ref().ok().and_then(|fm| {
            (fm.get("kind").and_then(Value::as_str) == Some(KIND_RUN))
                .then(|| fm.get("machine").and_then(Value::as_str))
                .flatten()
        });
        Some(match machine.filter(|m| ctx.machines.contains_key(*m)) {
            Some(m) => Scope::Key(crate::interpreter::child::slot_of(ctx, m).0),
            None => Scope::All,
        })
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

    /// One line for `decree status` and `daemon`: the file, what it holds, then what it says.
    pub fn summary(&self, ctx: &Context) -> String {
        let holds = match self.scope(ctx) {
            None => String::new(),
            Some(Scope::All) => "holds the queue: ".to_string(),
            Some(Scope::Key(key)) => format!("holds {}: ", scope_name(&key)),
        };
        match &self.frontmatter {
            Ok(fm) if fm.get("kind").and_then(Value::as_str) == Some(KIND_PAUSE) => {
                let since = fm.get("since").and_then(Value::as_str).unwrap_or_default();
                format!("{}: {holds}`decree pause` since {since}", self.path())
            }
            Ok(fm) => {
                let get = |k: &str| fm.get(k).and_then(Value::as_str).unwrap_or_default();
                let machine = fm
                    .get("machine")
                    .and_then(Value::as_str)
                    .map_or(String::new(), |m| format!(" ({m})"));
                format!(
                    "{}: {holds}run {}{machine} {} in `{}` since {}",
                    self.path(),
                    get("run"),
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

/// Check `value` against `attention.schema.json`: a run's file, an object with the keys in
/// `KEYS` (`machine` may be absent: a run with no machine of its own, such as a rejected
/// reply to an unknown run), or the file `decree pause` writes, with exactly those in
/// `PAUSE_KEYS`; each key of its type.
pub fn validate(value: &Value) -> Result<(), String> {
    let Value::Object(map) = value else {
        return Err("not a mapping".to_string());
    };
    let kind = map.get("kind").and_then(Value::as_str);
    let keys: &[&str] = match kind {
        Some(KIND_PAUSE) => &PAUSE_KEYS,
        _ => &KEYS,
    };
    if let Some(key) = map.keys().find(|k| !keys.contains(&k.as_str())) {
        return Err(format!("unknown key `{key}`"));
    }
    if let Some(key) = keys
        .iter()
        .find(|&&k| !map.contains_key(k) && k != "machine")
    {
        return Err(format!("missing `{key}`"));
    }
    if kind != Some(KIND_PAUSE) {
        for key in ["run", "machine", "message", "state"] {
            match map.get(key) {
                Some(Value::String(s)) if !s.is_empty() => {}
                None if key == "machine" => {}
                _ => return Err(format!("`{key}` is not a non-empty string")),
            }
        }
        if kind != Some(KIND_RUN) {
            return Err(format!("`kind` is not `{KIND_RUN}` or `{KIND_PAUSE}`"));
        }
        let status = map.get("status").and_then(Value::as_str);
        if !status.is_some_and(|s| STATUSES.contains(&s)) {
            return Err(format!("`status` is not one of {}", STATUSES.join(", ")));
        }
    }
    let since = map.get("since").and_then(Value::as_str);
    if !since.is_some_and(|s| DateTime::parse_from_rfc3339(s).is_ok()) {
        return Err("`since` is not an RFC 3339 time".to_string());
    }
    match (kind, map.get("holds")) {
        (Some(KIND_PAUSE), Some(Value::Bool(true))) => {}
        (Some(KIND_PAUSE), _) => return Err("`holds` is not `true`".to_string()),
        (_, Some(Value::Bool(_))) => {}
        _ => return Err("`holds` is not `true` or `false`".to_string()),
    }
    Ok(())
}

/// What a holding file holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// Every run: `pause.md`, a file decree cannot read, or a run's file whose machine is not
    /// loaded.
    All,
    /// The runs whose machine takes this slot key's slots: `group/<g>` or `machine/<m>`.
    Key(String),
}

/// `group <g>` or `machine <m>`, for a slot key.
pub fn scope_name(key: &str) -> String {
    match key.split_once('/') {
        Some((kind, name)) => format!("{kind} `{name}`"),
        None => key.to_string(),
    }
}

/// The holding files in `.decree/attention/`, by what they hold.
#[derive(Debug, Default)]
pub struct Holds {
    /// Files that hold every run.
    pub all: Vec<Entry>,
    /// Files that hold the runs of one slot key, by key.
    pub keys: std::collections::BTreeMap<String, Vec<Entry>>,
}

impl Holds {
    /// What `daemon` prints, and stops for, while files hold every run: each one decree wrote
    /// in full, and each it cannot read named with why. `None` when none does.
    pub fn all_message(&self, ctx: &Context) -> Option<String> {
        (!self.all.is_empty()).then(|| {
            message(
                ctx,
                &format!(
                    "nothing runs while a file in {DECREE_DIR}/{ATTENTION_DIR}/ holds the queue; \
                     do what it says, or delete it:"
                ),
                &self.all,
            )
        })
    }

    /// What `daemon` prints while files hold the runs of some machines: for each slot key,
    /// what waits and each file in full. `None` when none does.
    pub fn keys_message(&self, ctx: &Context) -> Option<String> {
        let parts: Vec<String> = self
            .keys
            .iter()
            .map(|(key, entries)| {
                message(
                    ctx,
                    &format!(
                        "the runs of {} wait while a file in {DECREE_DIR}/{ATTENTION_DIR}/ \
                         holds them; do what it says, or delete it:",
                        scope_name(key)
                    ),
                    entries,
                )
            })
            .collect();
        (!parts.is_empty()).then(|| parts.join("\n"))
    }
}

/// `heading`, then each of `entries`: one decree wrote in full, one it cannot read named
/// with why.
fn message(ctx: &Context, heading: &str, entries: &[Entry]) -> String {
    let dir = dir(&ctx.project_root);
    let mut out = heading.to_string();
    for entry in entries {
        out.push('\n');
        match &entry.frontmatter {
            Ok(_) => {
                let text = fs::read_to_string(dir.join(&entry.name)).unwrap_or_default();
                let body = Message::parse(&text).map(|m| m.body).unwrap_or(text);
                out.push_str(&format!("{}:\n{}", entry.path(), body.trim()));
            }
            Err(_) => out.push_str(&entry.summary(ctx)),
        }
    }
    out
}

/// What the files in `.decree/attention/` hold now. With `release`, first deletes the `hold`
/// of every run whose attention file a person deleted, which releases it.
pub fn holds(ctx: &Context, release: bool) -> Result<Holds, InterpreterError> {
    if release {
        release_dropped_holds(ctx)?;
    }
    let dir = dir(&ctx.project_root);
    let mut holds = Holds::default();
    for entry in list(&ctx.project_root).map_err(io_err(&dir))? {
        match entry.scope(ctx) {
            None => {}
            Some(Scope::All) => holds.all.push(entry),
            Some(Scope::Key(key)) => holds.keys.entry(key).or_default().push(entry),
        }
    }
    Ok(holds)
}

/// What `decree pause` did.
#[derive(Debug, PartialEq)]
pub enum Paused {
    /// It wrote `pause.md`.
    Written,
    /// `pause.md` was there already, and is left as it is.
    Exists,
}

/// `decree pause [-m <note>]`: write `.decree/attention/pause.md`, which holds the queue
/// until `decree resume` deletes it, unless it exists. The file appears whole or not at all
/// (a temp file, then a hard link that fails if the file exists), so two `pause`s at once
/// write one and the first `since` stays.
pub fn pause(
    project_root: &Path,
    note: Option<&str>,
    now: DateTime<chrono::Utc>,
) -> io::Result<Paused> {
    let dir = dir(project_root);
    let path = dir.join(PAUSE_FILE);
    if path.exists() {
        return Ok(Paused::Exists);
    }
    fs::create_dir_all(&dir)?;
    let since = crate::events::timestamp(now);
    let note = note
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(|n| format!("\n\n{n}"))
        .unwrap_or_default();
    let text = format!(
        "---\nkind: {KIND_PAUSE}\nsince: {since}\nholds: true\n---\n# decree pause\n\n\
         `decree pause` was run at {since}.{note}\n\n\
         Runs going now finish their current step loop, each ending in a final state or \
         waiting; no new run starts and no run continues while this file exists. To go on, \
         run the command below, which deletes this file; a daemon goes on without a \
         restart:\n\n    \
         decree resume\n"
    );
    let temp = dir.join(format!(".{PAUSE_FILE}.{}.tmp", std::process::id()));
    fs::write(&temp, text)?;
    let linked = fs::hard_link(&temp, &path);
    let removed = fs::remove_file(&temp);
    match linked {
        Ok(()) => removed.map(|()| Paused::Written),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(Paused::Exists),
        Err(e) => Err(e),
    }
}

/// `decree resume`: delete `.decree/attention/pause.md`. Returns whether it was there. A
/// file there that `decree pause` did not write (not `kind: pause`) is left alone, and
/// refused.
pub fn resume(project_root: &Path) -> io::Result<bool> {
    let path = dir(project_root).join(PAUSE_FILE);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    let ours = parse(&text)
        .ok()
        .is_some_and(|fm| fm.get("kind").and_then(Value::as_str) == Some(KIND_PAUSE));
    if !ours {
        return Err(io::Error::other(format!(
            "{DECREE_DIR}/{ATTENTION_DIR}/{PAUSE_FILE} was not written by `decree pause`; \
             read it, and delete it yourself"
        )));
    }
    match fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
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
/// run above it, whose status may follow from it. The caller holds the run's lock, or knows no process is stepping it.
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

/// `decree retry` of run `id`: delete its `hold` and its attention file.
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
    /// Waits in a `reply` state for a reply.
    Reply,
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
    let has_parent = message.as_ref().and_then(|m| m.text("parent")).is_some();
    let held = run_dir.join(HOLD_FILE).is_file();
    let need = match status {
        // A child run's failure is its parent's event, unless it holds the queue.
        // A run `decree cancel` ended needs no one.
        RunStatus::Finished if events.last().is_some_and(crate::events::cancelled) => None,
        RunStatus::Finished if current_state(&events) == Some(FAILED) && (!has_parent || held) => {
            Some(Need::Failed)
        }
        RunStatus::Interrupted => Some(Need::Interrupted),
        RunStatus::Waiting => match waiting_child(&events) {
            None => events
                .last()
                .filter(|e| is_type(e, "waiting") && text(e, "wait_id").is_some())
                .filter(|e| wants_attention(e))
                .map(|_| Need::Reply),
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
    let machine = machine_of(ctx, events, message);
    // A failed run is in `failed`; the state it failed from is the one `decree retry` continues in.
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
    let status = match need {
        Need::Failed => "failed",
        Need::Interrupted => "interrupted",
        Need::Reply => "waiting",
    };
    let message_name = "inbox".to_string();
    let mut frontmatter = serde_norway::Mapping::new();
    for (key, value) in KEYS.into_iter().zip([
        Some(KIND_RUN.into()),
        Some(id.into()),
        machine.as_deref().map(Into::into),
        Some(message_name.into()),
        Some(status.into()),
        Some(state.as_str().into()),
        Some(since.as_str().into()),
        Some(serde_norway::Value::Bool(held)),
    ]) {
        if let Some(value) = value {
            frontmatter.insert(key.into(), value);
        }
    }
    let who = match &machine {
        Some(machine) => format!("Run `{id}` (machine `{machine}`)"),
        None => format!("Run `{id}`"),
    };
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
    let retry = |run: &str| format!("    decree retry {run}\n\n");
    match need {
        Need::Failed => {
            body.push_str(&format!(
                "{who} failed in `{state}` at {since}; \
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
                "{who} was interrupted in `{state}`{cause} at \
                 {since}. Continue it with:\n\n{}",
                retry(id)
            ));
        }
        Need::Reply => body.push_str(&reply(ctx, id, events, &state, &since)),
    }
    if held {
        let hold = run_dir.join(HOLD_FILE);
        let text = fs::read_to_string(&hold).unwrap_or_default();
        // What the file holds: its machine's group, or every run (docs/reference/cli.md,
        // Attention).
        let what = match machine_of(ctx, events, message).filter(|m| ctx.machines.contains_key(m)) {
            Some(m) => format!(
                "the runs of {}",
                scope_name(&crate::interpreter::child::slot_of(ctx, &m).0)
            ),
            None => "every run".to_string(),
        };
        body.push_str(&format!(
            "## Hold\n\nThis run holds {what}: none of them starts or continues until it is \
             retried or this file is deleted; other machines go on. From \
             `.decree/runs/{id}/{HOLD_FILE}`:\n\n{}\n\n",
            text.trim()
        ));
    }
    let leftovers = run_dir.join(LEFTOVERS_FILE);
    if let Ok(text) = fs::read_to_string(&leftovers) {
        body.push_str(&format!(
            "## Leftovers\n\nThe work tree had changes when this run started. From \
             `.decree/runs/{id}/{LEFTOVERS_FILE}`:\n\n{}\n\n",
            text.trim()
        ));
    }
    let stop = run_dir.join(STOP_FILE);
    if let Ok(text) = fs::read_to_string(&stop) {
        body.push_str(&format!(
            "## {STOP_FILE}\n\nThe agent asked, in `.decree/runs/{id}/{STOP_FILE}`:\n\n{}\n\n\
             In `.decree/runs/{id}/`, answer it in plan.md, delete STOP, then \
             decree retry {id}\n\n",
            text.trim()
        ));
    }
    Ok(format!(
        "---\n{yaml}---\n{}",
        body.trim_end().to_string() + "\n"
    ))
}

/// The machine an attention file names: the run's own; for a reply decree rejected (a run
/// with no machine of its own), the machine of the run it was addressed to; `None` when there
/// is neither, as for a reply to a run id that does not exist.
fn machine_of(ctx: &Context, events: &[Event], message: Option<&Message>) -> Option<String> {
    let own = first_text(events, "machine").filter(|m| !m.is_empty());
    if let Some(machine) = own {
        return Some(machine.to_string());
    }
    let to = message?.text("to")?;
    let target = crate::reply::target_run(&ctx.runs_dir(), to)?;
    let events = ctx.events(&target).ok()?;
    first_text(&events, "machine")
        .filter(|m| !m.is_empty())
        .map(String::from)
}

/// The part of the file for a run waiting in a `reply` state: the question, each option
/// with its description, and a `decree reply` line per option.
fn reply(ctx: &Context, id: &str, events: &[Event], state: &str, since: &str) -> String {
    let Some(last) = events.last() else {
        return String::new();
    };
    let wait_id = text(last, "wait_id").unwrap_or_default();
    let machine = ctx.run_machine(events);
    let node = machine.and_then(|m| m.find(state).map(|s| (m, s)));
    let question = node
        .and_then(|(m, s)| match &m.nodes[s].invoke {
            Some(Invoke::Reply(p)) => p.question.clone(),
            _ => None,
        })
        .or_else(|| node.and_then(|(m, s)| m.nodes[s].description.clone()))
        .unwrap_or_else(|| format!("Run {id} waits in `{state}`."));
    let mut out = format!(
        "Run `{id}` waits in `{state}` since {since} for a reply:\n\n{}\n\nOptions:\n\n",
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
        out.push_str(&format!("    decree reply {wait_id} {option}\n"));
    }
    out.push('\n');
    out
}

/// Whether a `waiting` event for a reply needs a person: every one except a `reply` invoke
/// with `attention: false`, which only programs answer.
pub fn wants_attention(waiting: &Event) -> bool {
    waiting.get("attention").and_then(Value::as_bool) != Some(false)
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

    /// A run with no machine of its own (a rejected reply to an unknown run) has no
    /// `machine`; one that is there is never empty.
    #[test]
    fn test_validate_allows_machine_to_be_absent_but_not_empty() {
        let mut v = valid();
        v.as_object_mut().unwrap().remove("machine");
        assert_eq!(validate(&v), Ok(()));
        v["machine"] = json!("");
        assert_eq!(
            validate(&v).unwrap_err(),
            "`machine` is not a non-empty string"
        );
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
    fn test_validate_accepts_a_pause_and_names_what_is_wrong() {
        let stop =
            || json!({ "kind": "pause", "since": "2026-10-15T08:09:00.000Z", "holds": true });
        assert_eq!(validate(&stop()), Ok(()));
        let mut v = stop();
        v["holds"] = json!(false);
        assert_eq!(validate(&v).unwrap_err(), "`holds` is not `true`");
        let mut v = stop();
        v["run"] = json!("01-a");
        assert_eq!(validate(&v).unwrap_err(), "unknown key `run`");
        let mut v = stop();
        v.as_object_mut().unwrap().remove("since");
        assert_eq!(validate(&v).unwrap_err(), "missing `since`");
    }

    #[test]
    fn test_pause_writes_a_file_that_holds_and_keeps_the_first_and_resume_deletes_it() {
        let tmp = tempfile::TempDir::new().unwrap();
        let first = DateTime::parse_from_rfc3339("2026-10-15T08:09:00Z")
            .unwrap()
            .to_utc();
        assert_eq!(
            pause(tmp.path(), Some("release freeze"), first).unwrap(),
            Paused::Written
        );
        let path = dir(tmp.path()).join(PAUSE_FILE);
        let text = fs::read_to_string(&path).unwrap();
        let fm = parse(&text).unwrap();
        assert_eq!(fm["kind"], "pause");
        assert_eq!(fm["since"], "2026-10-15T08:09:00.000Z");
        assert!(text.contains("\n\nrelease freeze\n\n"), "{text}");
        assert!(text.contains("\n    decree resume\n"), "{text}");
        let entries = list(tmp.path()).unwrap();
        assert_eq!(entries.len(), 1, "no temp file is left");
        assert!(entries[0].holds());
        assert_eq!(
            entries[0].summary(&Context {
                project_root: tmp.path().to_path_buf(),
                machines: Default::default(),
                shutdown: Default::default(),
                env: Default::default(),
                slots: crate::slots::Slots::new(tmp.path()),
            }),
            ".decree/attention/pause.md: holds the queue: `decree pause` since \
             2026-10-15T08:09:00.000Z"
        );

        let later = first + chrono::Duration::hours(1);
        assert_eq!(pause(tmp.path(), None, later).unwrap(), Paused::Exists);
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        // Nothing decree does for a run named `pause` touches it.
        remove(tmp.path(), "pause").unwrap();
        assert!(path.exists());

        assert!(resume(tmp.path()).unwrap());
        assert!(!path.exists());
        assert!(!resume(tmp.path()).unwrap());
        // A `pause.md` a person wrote is theirs: resume leaves it, and says so.
        fs::write(&path, "hold everything\n").unwrap();
        assert!(resume(tmp.path()).is_err());
        assert!(path.exists());
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
