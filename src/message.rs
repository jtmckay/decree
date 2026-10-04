use crate::error::DecreeError;
use crate::layout;
use crate::layout::MESSAGE_FILE;
use crate::layout::{INBOX_DIR, RUNS_DIR};
use crate::machine::LoadedMachine;
use chrono::Utc;
use serde_norway::{Mapping, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

// =================================================================
// Message (docs/reference/messages.md)
// =================================================================

/// One message: YAML frontmatter plus a body (docs/reference/messages.md). The frontmatter is a `Mapping`,
/// so unknown keys and key order survive a read and write; the body is kept byte for byte.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Message {
    pub frontmatter: Mapping,
    pub body: String,
    /// File line of each top-level frontmatter key, for error locations.
    key_lines: BTreeMap<String, usize>,
}

/// Why a message file could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum MessageError {
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },

    /// The frontmatter does not parse. `line` is the line in the file.
    #[error("{}: line {line}: {message}", path.display())]
    Parse {
        path: PathBuf,
        line: usize,
        message: String,
    },
}

impl Message {
    /// A message with no frontmatter and this body.
    pub fn new(body: impl Into<String>) -> Message {
        Message {
            body: body.into(),
            ..Message::default()
        }
    }

    /// Parse a message (docs/reference/messages.md, Parsing and writing): an optional UTF-8 BOM, `\n` or
    /// `\r\n` line ends, fences that are `---` once trailing whitespace is removed, and a
    /// YAML mapping without duplicate keys between them. No opening fence means an empty
    /// map and the whole text is the body. Errors are `(file line, message)`.
    pub fn parse(text: &str) -> Result<Message, (usize, String)> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let is_fence = |l: &str| l.trim_end() == "---";
        let mut lines = text.split_inclusive('\n');
        if !lines.next().is_some_and(is_fence) {
            return Ok(Message::new(text));
        }
        let mut yaml_lines = Vec::new();
        let mut offset = text.split_inclusive('\n').next().map_or(0, str::len);
        let mut closed = false;
        for line in lines {
            offset += line.len();
            if is_fence(line) {
                closed = true;
                break;
            }
            yaml_lines.push(line.trim_end_matches(['\r', '\n']));
        }
        if !closed {
            return Err((
                1,
                "frontmatter has an opening `---` but no closing `---`".into(),
            ));
        }
        // YAML line `n` is file line `n + 1`: the opening fence is line 1.
        let yaml = yaml_lines.join("\n");
        let value: Value = serde_norway::from_str(&yaml).map_err(|e| {
            let msg = strip_yaml_location(&e.to_string());
            let line = duplicate_key_line(&yaml_lines, &msg)
                .or_else(|| e.location().map(|l| l.line() + 1))
                .unwrap_or(1);
            (line, msg)
        })?;
        let frontmatter = match value {
            Value::Null => Mapping::new(),
            Value::Mapping(map) => map,
            _ => return Err((2, "frontmatter is not a YAML mapping".into())),
        };
        let mut key_lines = BTreeMap::new();
        for (i, line) in yaml_lines.iter().enumerate() {
            if let Some((key, _)) = line.split_once(':') {
                if !key.is_empty() && !key.starts_with([' ', '\t', '#', '-']) {
                    key_lines.entry(key.trim().to_string()).or_insert(i + 2);
                }
            }
        }
        Ok(Message {
            frontmatter,
            body: text[offset..].to_string(),
            key_lines,
        })
    }

    /// Read and parse the message at `path`.
    pub fn read(path: &Path) -> Result<Message, MessageError> {
        let bytes = std::fs::read(path).map_err(|source| MessageError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let parse_err = |line, message| MessageError::Parse {
            path: path.to_path_buf(),
            line,
            message,
        };
        let text = String::from_utf8(bytes)
            .map_err(|_| parse_err(1, "file is not valid UTF-8".to_string()))?;
        Message::parse(&text).map_err(|(line, message)| parse_err(line, message))
    }

    /// `---\n` + the frontmatter + `---\n` + the body bytes, unchanged.
    pub fn to_bytes(&self) -> Vec<u8> {
        let yaml = if self.frontmatter.is_empty() {
            String::new()
        } else {
            // A `Mapping` of YAML values always serializes.
            serde_norway::to_string(&self.frontmatter).unwrap_or_default()
        };
        format!("---\n{yaml}---\n{}", self.body).into_bytes()
    }

    /// Write the message to `path` through `.<name>.tmp` in the same directory and a rename,
    /// so `path` never holds a partial message.
    pub fn write(&self, path: &Path) -> Result<(), MessageError> {
        write_replace(path, &self.to_bytes())
            .map_err(|(path, source)| MessageError::Io { path, source })
    }

    /// Frontmatter `key` as a string, if it is one.
    pub fn text(&self, key: &str) -> Option<&str> {
        self.frontmatter.get(key).and_then(Value::as_str)
    }

    /// The machine the message names: `machine`, else its alias `routine`, as a string.
    pub fn machine(&self) -> Option<&str> {
        self.text("machine").or_else(|| self.text("routine"))
    }

    /// Frontmatter `params`, or an empty mapping if it is missing or not a mapping.
    pub fn params(&self) -> Mapping {
        match self.frontmatter.get("params") {
            Some(Value::Mapping(params)) => params.clone(),
            _ => Mapping::new(),
        }
    }

    /// Frontmatter `depth`: 0 if it is missing or not a non-negative integer.
    pub fn depth(&self) -> u32 {
        self.frontmatter
            .get("depth")
            .and_then(Value::as_u64)
            .map_or(0, |d| u32::try_from(d).unwrap_or(u32::MAX))
    }

    /// Set frontmatter `key`, keeping its position if it exists, else appending it.
    pub fn set(&mut self, key: &str, value: impl Into<Value>) {
        self.frontmatter.insert(key.into(), value.into());
    }

    /// The file line of top-level key `key`, or 1 if it is not there.
    pub fn line_of(&self, key: &str) -> usize {
        self.key_lines.get(key).copied().unwrap_or(1)
    }
}

/// The deepest a message may sit in a chain of emits and child runs (docs/reference/README.md,
/// No configuration file).
pub const MAX_DEPTH: u32 = 10;

/// The run folders in `runs_dir`, in `id` order. A missing `runs_dir` holds none.
pub fn run_ids(runs_dir: &Path) -> io::Result<Vec<String>> {
    let mut ids: Vec<String> = match std::fs::read_dir(runs_dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    ids.sort();
    Ok(ids)
}

/// Create `runs/<id>/` for a new run, with a new id (`new_id`).
pub fn create_run_dir(decree_dir: &Path) -> Result<(String, PathBuf), MessageError> {
    let runs = decree_dir.join(RUNS_DIR);
    std::fs::create_dir_all(&runs).map_err(io_err(&runs))?;
    let id = new_id(decree_dir, |id| {
        let dir = runs.join(id);
        match std::fs::create_dir(&dir) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(io_err(&dir)(e)),
        }
    })?;
    let dir = runs.join(&id);
    Ok((id, dir))
}

/// A new message id (docs/reference/messages.md, Frontmatter keys): the UTC time, `YYYYMMDDTHHMMSSZ`, then
/// `-` and 6 lowercase hex chars, the low 24 bits of (sub-second nanoseconds XOR process
/// id). While that id exists in `inbox/` or `runs/`, or `take` declines it, add 1.
fn new_id(
    decree_dir: &Path,
    mut take: impl FnMut(&str) -> Result<bool, MessageError>,
) -> Result<String, MessageError> {
    let now = Utc::now();
    let stamp = now.format("%Y%m%dT%H%M%SZ");
    let mut low = (now.timestamp_subsec_nanos() ^ std::process::id()) & 0xff_ffff;
    for _ in 0..=0xff_ffff {
        let id = format!("{stamp}-{low:06x}");
        let queued = decree_dir.join(INBOX_DIR).join(format!("{id}.md")).exists();
        let has_run = decree_dir.join(RUNS_DIR).join(&id).exists();
        if !queued && !has_run && take(&id)? {
            return Ok(id);
        }
        low = (low + 1) & 0xff_ffff;
    }
    let runs = decree_dir.join(RUNS_DIR);
    Err(io_err(&runs)(io::Error::other(format!(
        "every id for {stamp} is taken"
    ))))
}

/// Queue `message` in `inbox/` (docs/reference/messages.md, Lifecycle step 1): give it a new `id` as its
/// first key, and write `inbox/<id>.md` through `.<id>.md.tmp` and a rename, so no partial
/// `*.md` is ever visible. The one writer for `decree emit`, `decree event` and cron.
/// Returns the id.
pub fn queue(decree_dir: &Path, message: &mut Message) -> Result<String, MessageError> {
    let inbox = decree_dir.join(INBOX_DIR);
    std::fs::create_dir_all(&inbox).map_err(io_err(&inbox))?;
    let id = new_id(decree_dir, |_| Ok(true))?;
    write_queued(&inbox, &id, message)?;
    Ok(id)
}

/// Set `id` as the first key of `message` and write it to `inbox/<id>.md` through
/// `inbox/.<id>.md.tmp`.
fn write_queued(inbox: &Path, id: &str, message: &mut Message) -> Result<(), MessageError> {
    let mut frontmatter = Mapping::new();
    frontmatter.insert("id".into(), id.into());
    for (key, value) in std::mem::take(&mut message.frontmatter) {
        if key.as_str() != Some("id") {
            frontmatter.insert(key, value);
        }
    }
    message.frontmatter = frontmatter;
    message.write(&inbox.join(format!("{id}.md")))
}

/// A message claimed from `inbox/` (docs/reference/messages.md, Lifecycle step 2): it was renamed to
/// `runs/<id>/message.md`.
#[derive(Debug)]
pub struct Claim {
    pub id: String,
    pub run_dir: PathBuf,
    /// The inbox filename.
    pub file: String,
    /// The message as claimed, or why its frontmatter does not parse.
    pub message: Result<Message, MessageError>,
    /// Why frontmatter `id` was not used: it cannot name a run folder, or has a run.
    pub id_problem: Option<String>,
}

/// Claim `inbox/<file>`: create `runs/<id>/` (frontmatter `id`, else a new id) and rename the
/// file to `runs/<id>/message.md`. The rename is the claim: `None` means another process
/// won, or is claiming a message with this `id` right now. A frontmatter `id` that cannot
/// name a run folder, or that already has a run, is replaced by a new id, so the message is
/// still claimed and its run can be failed with the reason.
pub fn claim(decree_dir: &Path, file: &str) -> Result<Option<Claim>, MessageError> {
    let path = decree_dir.join(layout::INBOX_DIR).join(file);
    let message = match Message::read(&path) {
        Err(MessageError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(None)
        }
        other => other,
    };
    let runs = decree_dir.join(layout::RUNS_DIR);
    let wanted = message.as_ref().ok().and_then(|m| m.frontmatter.get("id"));
    let mut id_problem = None;
    let (id, run_dir) = match wanted {
        None => create_run_dir(decree_dir)?,
        Some(Value::String(id)) if is_valid_id(id) => {
            std::fs::create_dir_all(&runs).map_err(io_err(&runs))?;
            let dir = runs.join(id);
            match std::fs::create_dir(&dir) {
                Ok(()) => (id.clone(), dir),
                // Another process created the folder and is about to rename the file.
                Err(e)
                    if e.kind() == io::ErrorKind::AlreadyExists
                        && !dir.join(MESSAGE_FILE).exists() =>
                {
                    return Ok(None)
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    id_problem = Some(format!("`id` {id} already has a run"));
                    create_run_dir(decree_dir)?
                }
                Err(e) => return Err(io_err(&dir)(e)),
            }
        }
        Some(_) => {
            id_problem = Some(
                "`id` must be a string of ASCII letters, digits, `.`, `_` and `-`, \
                 not starting with `.`"
                    .to_string(),
            );
            create_run_dir(decree_dir)?
        }
    };
    let target = run_dir.join(MESSAGE_FILE);
    match std::fs::rename(&path, &target) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let _ = std::fs::remove_dir(&run_dir);
            return Ok(None);
        }
        Err(e) => {
            let _ = std::fs::remove_dir(&run_dir);
            return Err(io_err(&path)(e));
        }
    }
    // Errors name the file where it now is.
    let message = message.map_err(|e| match e {
        MessageError::Parse { line, message, .. } => MessageError::Parse {
            path: target.clone(),
            line,
            message,
        },
        other => other,
    });
    Ok(Some(Claim {
        id,
        run_dir,
        file: file.to_string(),
        message,
        id_problem,
    }))
}

/// An `id` that can name a run folder: ASCII letters, digits, `.`, `_` and `-`, not
/// starting with `.`.
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The run lock in a run folder (docs/reference/messages.md, Run lock).
pub const LOCK_FILE: &str = ".lock";

/// What a run's `.lock` says (docs/reference/messages.md, Run lock).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockState {
    /// No lock: no process is stepping the run.
    Free,
    /// The lock holds this live pid: the run is `active`.
    Live(u32),
    /// The lock's process is gone, or the lock holds no pid: a crash left it.
    Stale,
}

/// Read the `.lock` in `run_dir` and check its pid with `kill(pid, 0)`.
pub fn lock_state(run_dir: &Path) -> io::Result<LockState> {
    match std::fs::read_to_string(run_dir.join(LOCK_FILE)) {
        Ok(text) => Ok(match text.trim().parse::<u32>() {
            Ok(pid) if pid_alive(pid) => LockState::Live(pid),
            _ => LockState::Stale,
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(LockState::Free),
        Err(e) => Err(e),
    }
}

/// Whether process `pid` exists. `EPERM` means it exists but belongs to another user.
fn pid_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        // 0 and negative pids address process groups, not a process.
        return false;
    }
    // SAFETY: signal 0 only checks that the process exists.
    let sent = unsafe { libc::kill(pid, 0) } == 0;
    sent || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// A held run lock: `runs/<id>/.lock` holding this process's id. Dropping it deletes the
/// lock, so a run that finishes, waits, is interrupted by a signal or hits an error
/// releases it; only a killed process leaves a stale one behind.
#[derive(Debug)]
pub struct RunLock {
    path: PathBuf,
}

impl RunLock {
    /// Take the lock of the run in `run_dir` (docs/reference/messages.md, Run lock): create `.lock` with
    /// `create_new` and write the process id into it. A stale lock is deleted first; it
    /// never causes a takeover of a live one. `Ok(None)` means another live process holds
    /// the lock: the run is `active`.
    pub fn acquire(run_dir: &Path) -> io::Result<Option<RunLock>> {
        use std::io::Write;
        let path = run_dir.join(LOCK_FILE);
        // Two tries: the second follows deleting a stale lock. Losing again means another
        // process took the lock in between.
        for _ in 0..2 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    let lock = RunLock { path };
                    file.write_all(std::process::id().to_string().as_bytes())?;
                    return Ok(Some(lock));
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
            match lock_state(run_dir)? {
                LockState::Live(_) => return Ok(None),
                LockState::Stale => match std::fs::remove_file(&path) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                    _ => {}
                },
                LockState::Free => {}
            }
        }
        Ok(None)
    }
}

impl Drop for RunLock {
    fn drop(&mut self) {
        // Only this process's lock: never one another process created after a stale one.
        let ours = std::fs::read_to_string(&self.path)
            .is_ok_and(|text| text.trim() == std::process::id().to_string());
        if ours {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> MessageError + '_ {
    move |source| MessageError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// docs/reference/messages.md, Lifecycle step 3: the message names a machine (`machine`, or its alias
/// `routine`) that exists, and its `params` fit that machine's `data`.
/// `machine_ids` holds every machine, `machines` those that loaded; a machine that fails to
/// load is reported on its own, so its `params` are not checked. Returns the machine name,
/// or every error as `(file line, message)`.
pub fn validate(
    msg: &Message,
    machines: &BTreeMap<String, LoadedMachine>,
    machine_ids: &BTreeSet<String>,
) -> Result<String, Vec<(usize, String)>> {
    let machine = match (
        msg.frontmatter.get("machine"),
        msg.frontmatter.get("routine"),
    ) {
        (Some(_), Some(_)) => {
            return Err(vec![(
                msg.line_of("routine"),
                "both `machine` and its alias `routine` are set".to_string(),
            )]);
        }
        (Some(v), None) => ("machine", v),
        (None, Some(v)) => ("routine", v),
        (None, None) => return Err(vec![(1, "no `machine` key".to_string())]),
    };
    let (key, value) = machine;
    let line = msg.line_of(key);
    let Value::String(name) = value else {
        return Err(vec![(line, format!("`{key}` must be a string"))]);
    };
    if !machine_ids.contains(name) {
        return Err(vec![(line, format!("unknown machine `{name}`"))]);
    }
    let errors = machines
        .get(name)
        .map(|m| check_params(msg, m))
        .unwrap_or_default();
    if errors.is_empty() {
        Ok(name.to_string())
    } else {
        Err(errors)
    }
}

/// `params` must be a map from a name in the machine's `data` to a value of its type.
fn check_params(fm: &Message, m: &LoadedMachine) -> Vec<(usize, String)> {
    let line = fm.line_of("params");
    let params = match fm.frontmatter.get("params") {
        None => return Vec::new(),
        Some(Value::Mapping(params)) => params,
        Some(_) => return vec![(line, "`params` must be a mapping".to_string())],
    };
    let mut errors = Vec::new();
    for (key, value) in params {
        let Some(key) = key.as_str() else {
            errors.push((line, "`params` keys must be strings".to_string()));
            continue;
        };
        match m.data.get(key) {
            None => errors.push((
                line,
                format!(
                    "unknown param `{key}`: machine `{}` has no data `{key}`",
                    m.id
                ),
            )),
            Some(spec) if !spec.kind.matches(value) => errors.push((
                line,
                format!("param `{key}` must be of type `{}`", spec.kind.as_str()),
            )),
            Some(_) => {}
        }
    }
    errors
}

/// Write `bytes` to `.<name>.tmp` beside `path`, then rename it over `path` (docs/reference/messages.md).
/// The error carries the path that failed.
pub fn write_replace(path: &Path, bytes: &[u8]) -> Result<(), (PathBuf, io::Error)> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    std::fs::write(&tmp, bytes).map_err(|e| (tmp.clone(), e))?;
    std::fs::rename(&tmp, path).map_err(|e| (path.to_path_buf(), e))
}

/// serde_norway reports a duplicate key at the start of its mapping; find the file line of
/// the second top-level occurrence instead.
fn duplicate_key_line(yaml_lines: &[&str], msg: &str) -> Option<usize> {
    let key = msg
        .strip_prefix("duplicate entry with key \"")?
        .strip_suffix('"')?;
    yaml_lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.strip_prefix(key).is_some_and(|r| r.starts_with(':')))
        .nth(1)
        .map(|(i, _)| i + 2)
}

/// serde_norway messages end with `at line N column M`, a line inside the YAML; the caller
/// reports the file line instead.
fn strip_yaml_location(msg: &str) -> String {
    match msg.find(" at line ") {
        Some(i) => msg[..i].to_string(),
        None => msg.to_string(),
    }
}

// =================================================================
// YAML frontmatter parsing
// =================================================================

/// Parse YAML frontmatter from markdown content.
///
/// Returns `(fields_map, body)`. If no frontmatter is found, returns an
/// empty map and the entire content as body. The body is returned with
/// its original whitespace preserved.
pub fn parse_frontmatter(
    content: &str,
) -> Result<(BTreeMap<String, serde_norway::Value>, String), DecreeError> {
    if !content.starts_with("---\n") {
        return Ok((BTreeMap::new(), content.to_string()));
    }

    let after_open = &content[4..]; // skip "---\n"

    // Find closing "---" delimiter
    let (yaml_str, body) = if let Some(pos) = after_open.find("\n---\n") {
        (&after_open[..pos], &after_open[pos + 5..]) // skip "\n---\n"
    } else if let Some(yaml) = after_open.strip_suffix("\n---") {
        (yaml, "")
    } else if let Some(body) = after_open.strip_prefix("---\n") {
        // Empty frontmatter: ---\n---\n...
        ("", body)
    } else if after_open == "---" {
        ("", "")
    } else {
        // No closing delimiter
        return Ok((BTreeMap::new(), content.to_string()));
    };

    let map: BTreeMap<String, serde_norway::Value> = if yaml_str.trim().is_empty() {
        BTreeMap::new()
    } else {
        // `Mapping` rejects duplicate keys; a `BTreeMap` would keep the last one.
        let mapping: serde_norway::Mapping = serde_norway::from_str(yaml_str)?;
        serde_norway::from_value(serde_norway::Value::Mapping(mapping))?
    };

    Ok((map, body.to_string()))
}

// =================================================================
// Tests
// =================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // --- Message (docs/reference/messages.md, Parsing and writing) ---

    fn parse_err(text: &str) -> (usize, String) {
        Message::parse(text).unwrap_err()
    }

    #[test]
    fn message_round_trip_keeps_unknown_keys_order_and_crlf_body() {
        let text = "---\r\nzeta: 1\r\nmachine: hello\r\nalpha: [a, b]\r\ncustom:\r\n  nested: x\r\n---\r\n# Title\r\nline two\nmixed\r\n";
        let msg = Message::parse(text).unwrap();
        let keys: Vec<&str> = msg.frontmatter.keys().filter_map(Value::as_str).collect();
        assert_eq!(keys, ["zeta", "machine", "alpha", "custom"]);
        assert_eq!(msg.body, "# Title\r\nline two\nmixed\r\n");

        let written = msg.to_bytes();
        let again = Message::parse(std::str::from_utf8(&written).unwrap()).unwrap();
        assert_eq!(again.frontmatter, msg.frontmatter);
        assert_eq!(again.body.as_bytes(), b"# Title\r\nline two\nmixed\r\n");
        assert!(written.ends_with(b"---\n# Title\r\nline two\nmixed\r\n"));
    }

    #[test]
    fn message_write_goes_through_a_temp_file_and_keeps_the_body() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("m.md");
        let mut msg = Message::parse("---\nmachine: a\nx: 1\n---\r\nbody\r\n").unwrap();
        msg.set("state", "verify");
        msg.set("machine", "b");
        msg.write(&path).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "---\nmachine: b\nx: 1\nstate: verify\n---\nbody\r\n"
        );
        assert!(!dir.path().join(".m.md.tmp").exists());
        assert_eq!(Message::read(&path).unwrap().body, "body\r\n");
    }

    #[test]
    fn message_without_fence_is_all_body() {
        let msg = Message::parse("# Title\nbody\n").unwrap();
        assert!(msg.frontmatter.is_empty());
        assert_eq!(msg.body, "# Title\nbody\n");
        assert_eq!(msg.to_bytes(), b"---\n---\n# Title\nbody\n");
    }

    #[test]
    fn message_bom_crlf_and_trailing_spaces_on_fences_parse() {
        let msg = Message::parse(
            "\u{feff}---  \r\nmachine: hello\r\nparams:\n  x: 1\n--- \t\r\nbody\r\n",
        )
        .unwrap();
        assert_eq!(msg.text("machine"), Some("hello"));
        assert_eq!(msg.line_of("machine"), 2);
        assert_eq!(msg.line_of("params"), 3);
        assert_eq!(msg.body, "body\r\n");
    }

    #[test]
    fn message_empty_frontmatter_is_empty_map() {
        let msg = Message::parse("---\n---\nbody").unwrap();
        assert!(msg.frontmatter.is_empty());
        assert_eq!(msg.body, "body");
    }

    #[test]
    fn message_unclosed_fence_fails_naming_file_and_line() {
        let (line, msg) = parse_err("---\nmachine: hello\nbody\n");
        assert_eq!(line, 1);
        assert!(msg.contains("no closing"), "{msg}");

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("01-open.md");
        std::fs::write(&path, "---\nmachine: hello\n# body, not frontmatter\n").unwrap();
        let err = Message::read(&path).unwrap_err().to_string();
        assert!(
            err.starts_with(&format!("{}: line 1: ", path.display())),
            "{err}"
        );
    }

    #[test]
    fn message_yaml_error_names_the_file_line() {
        let (line, msg) = parse_err("---\nmachine: a\nid: x\nmachine: b\n---\n");
        assert_eq!(line, 4, "{msg}");
        assert!(msg.contains("duplicate"), "{msg}");
        assert!(!msg.contains("at line"), "{msg}");

        let (line, _) = parse_err("---\nmachine: a\nparams: [\n---\n");
        assert!(line >= 3, "{line}");
    }

    fn machines() -> (BTreeMap<String, LoadedMachine>, BTreeSet<String>) {
        let text = "name: x\ndescription: Test.\ninitial: a\ndata:\n  rounds: { type: int, default: 1 }\nstates:\n  a:\n    invoke: a\n    transitions: { done: done }\n  done: { final: true }\n  failed: { final: true }\n";
        let m = crate::machine::load_machine_text("x", text).unwrap();
        let machines = BTreeMap::from([("x".to_string(), m)]);
        let ids = machines.keys().cloned().collect();
        (machines, ids)
    }

    fn validated(text: &str) -> Result<String, Vec<(usize, String)>> {
        let (machines, ids) = machines();
        validate(&Message::parse(text).unwrap(), &machines, &ids)
    }

    #[test]
    fn validate_reads_routine_as_machine() {
        assert_eq!(validated("---\nroutine: x\n---\n").unwrap(), "x");
        assert_eq!(validated("---\nmachine: x\n---\n").unwrap(), "x");
    }

    #[test]
    fn validate_rejects_unknown_machine_and_params() {
        let errors = validated("---\nid: a\nmachine: nope\n---\n").unwrap_err();
        assert_eq!(errors, [(3, "unknown machine `nope`".to_string())]);
        let errors =
            validated("---\nmachine: x\nparams:\n  rounds: two\n  other: 1\n---\n").unwrap_err();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(
            errors[0].1.contains("`rounds` must be of type `int`"),
            "{errors:?}"
        );
        assert!(errors[1].1.contains("unknown param `other`"), "{errors:?}");
        let errors = validated("---\nmachine: x\nroutine: x\n---\n").unwrap_err();
        assert!(errors[0].1.contains("both"), "{errors:?}");
        assert_eq!(
            validated("body\n").unwrap_err(),
            [(1, "no `machine` key".to_string())]
        );
    }

    fn inbox(dir: &TempDir, name: &str, text: &str) -> PathBuf {
        let decree = dir.path().join(".decree");
        std::fs::create_dir_all(decree.join("inbox")).unwrap();
        std::fs::write(decree.join("inbox").join(name), text).unwrap();
        decree
    }

    /// Both threads try to claim the same file at once; returns how many succeeded.
    fn race(text: &str) -> usize {
        let dir = TempDir::new().unwrap();
        let decree = inbox(&dir, "a.md", text);
        let barrier = std::sync::Barrier::new(2);
        let won = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..2)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        claim(&decree, "a.md").unwrap()
                    })
                })
                .collect();
            handles
                .into_iter()
                .filter_map(|h| h.join().unwrap())
                .collect::<Vec<_>>()
        });
        // The loser leaves no run folder behind.
        let runs = std::fs::read_dir(decree.join("runs")).unwrap().count();
        assert_eq!(runs, won.len());
        assert!(!decree.join("inbox/a.md").exists());
        won.len()
    }

    #[test]
    fn claim_two_threads_exactly_one_succeeds() {
        for _ in 0..50 {
            assert_eq!(race("---\nmachine: x\n---\nbody\n"), 1);
            assert_eq!(race("---\nid: fixed-1\nmachine: x\n---\nbody\n"), 1);
        }
    }

    #[test]
    fn claim_moves_the_file_into_the_run_folder() {
        let dir = TempDir::new().unwrap();
        let decree = inbox(&dir, "b.md", "---\nid: run-b\n---\nbody\r\n");
        let c = claim(&decree, "b.md").unwrap().unwrap();
        assert_eq!(c.id, "run-b");
        assert_eq!(c.file, "b.md");
        assert_eq!(c.run_dir, decree.join("runs/run-b"));
        assert_eq!(
            std::fs::read_to_string(c.run_dir.join(MESSAGE_FILE)).unwrap(),
            "---\nid: run-b\n---\nbody\r\n"
        );
        assert_eq!(c.message.unwrap().body, "body\r\n");
        assert!(claim(&decree, "b.md").unwrap().is_none());
    }

    #[test]
    fn claim_gives_a_new_id_when_the_id_is_taken_or_unusable() {
        let dir = TempDir::new().unwrap();
        let decree = inbox(&dir, "c.md", "---\nid: run-c\n---\n");
        claim(&decree, "c.md").unwrap().unwrap();
        inbox(&dir, "c.md", "---\nid: run-c\n---\n");
        let again = claim(&decree, "c.md").unwrap().unwrap();
        assert_ne!(again.id, "run-c");
        assert_eq!(again.id_problem.unwrap(), "`id` run-c already has a run");
        inbox(&dir, "d.md", "---\nid: ../escape\n---\n");
        let bad = claim(&decree, "d.md").unwrap().unwrap();
        assert!(is_valid_id(&bad.id) && bad.id != "../escape");
        assert!(bad.id_problem.unwrap().starts_with("`id` must be"));
    }

    #[test]
    fn claim_keeps_an_unparsable_message_and_names_its_new_path() {
        let dir = TempDir::new().unwrap();
        let decree = inbox(&dir, "e.md", "---\nmachine: x\n");
        let c = claim(&decree, "e.md").unwrap().unwrap();
        let err = c.message.unwrap_err().to_string();
        assert!(err.contains("message.md: line 1:"), "{err}");
        assert_eq!(
            std::fs::read_to_string(c.run_dir.join(MESSAGE_FILE)).unwrap(),
            "---\nmachine: x\n"
        );
    }

    #[test]
    fn queue_writes_inbox_id_md_with_the_id_first() {
        let dir = TempDir::new().unwrap();
        let decree = dir.path().join(".decree");
        let mut message = Message::parse("---\nmachine: x\nid: old\n---\nbody\r\n").unwrap();
        let id = queue(&decree, &mut message).unwrap();
        assert!(id.len() == 23 && id.as_bytes()[8] == b'T' && id.as_bytes()[15] == b'Z');
        assert!(id[17..]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        let names: Vec<_> = std::fs::read_dir(decree.join("inbox"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, [format!("{id}.md")]);
        assert_eq!(
            std::fs::read_to_string(decree.join("inbox").join(&names[0])).unwrap(),
            format!("---\nid: {id}\nmachine: x\n---\nbody\r\n")
        );
        // An id that is queued or has a run is never reused.
        let second = queue(&decree, &mut Message::new("b")).unwrap();
        assert_ne!(second, id);
    }

    #[test]
    fn queue_writes_through_the_temp_name() {
        let dir = TempDir::new().unwrap();
        let inbox = dir.path().join("inbox");
        std::fs::create_dir_all(inbox.join(".x-1.md.tmp")).unwrap();
        let err = write_queued(&inbox, "x-1", &mut Message::new("b")).unwrap_err();
        assert!(err.to_string().contains(".x-1.md.tmp"), "{err}");
        assert!(!inbox.join("x-1.md").exists());
    }

    #[test]
    fn queue_never_shows_a_partial_md_in_inbox() {
        let dir = TempDir::new().unwrap();
        let decree = dir.path().join(".decree");
        std::fs::create_dir_all(decree.join("inbox")).unwrap();
        let body = "x".repeat(1 << 20);
        let done = std::sync::atomic::AtomicBool::new(false);
        let seen = std::thread::scope(|scope| {
            let lister = scope.spawn(|| {
                let mut seen = 0;
                // One more pass after the writer is done, so every file is read at least once.
                let mut last = false;
                while !last {
                    last = done.load(std::sync::atomic::Ordering::Relaxed);
                    for entry in std::fs::read_dir(decree.join("inbox")).unwrap() {
                        let name = entry.unwrap().file_name().into_string().unwrap();
                        if name.starts_with('.') || !name.ends_with(".md") {
                            continue;
                        }
                        let text = std::fs::read_to_string(decree.join("inbox").join(&name));
                        let message = Message::parse(&text.unwrap()).unwrap();
                        assert_eq!(message.body.len(), body.len(), "{name} is partial");
                        seen += 1;
                    }
                }
                seen
            });
            for _ in 0..20 {
                queue(&decree, &mut Message::new(body.as_str())).unwrap();
            }
            done.store(true, std::sync::atomic::Ordering::Relaxed);
            lister.join().unwrap()
        });
        assert!(seen >= 20);
        let left = std::fs::read_dir(decree.join("inbox"))
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp")
            })
            .count();
        assert_eq!(left, 0);
    }

    #[test]
    fn message_non_mapping_is_an_error() {
        let (_, msg) = parse_err("---\n- a\n- b\n---\n");
        assert!(msg.contains("not a YAML mapping"), "{msg}");
    }

    // --- Frontmatter parsing tests ---

    #[test]
    fn test_parse_frontmatter_none() {
        let (map, body) = parse_frontmatter("Just some text.\n").unwrap();
        assert!(map.is_empty());
        assert_eq!(body, "Just some text.\n");
    }

    #[test]
    fn test_parse_frontmatter_empty() {
        let (map, body) = parse_frontmatter("---\n---\n").unwrap();
        assert!(map.is_empty());
        assert_eq!(body, "");
    }

    #[test]
    fn test_parse_frontmatter_with_fields() {
        let content = "---\nroutine: develop\n---\nHello world.\n";
        let (map, body) = parse_frontmatter(content).unwrap();
        assert_eq!(
            map.get("routine"),
            Some(&serde_norway::Value::String("develop".into()))
        );
        assert_eq!(body, "Hello world.\n");
    }

    #[test]
    fn test_parse_frontmatter_full_message() {
        let content = "---\n\
            id: D0001-1432-01-add-auth-0\n\
            chain: D0001-1432-01-add-auth\n\
            seq: 0\n\
            routine: develop\n\
            migration: 01-add-auth.md\n\
            ---\n\
            # Add Auth\n\
            \n\
            Add authentication.\n";

        let (map, body) = parse_frontmatter(content).unwrap();
        assert_eq!(map.len(), 5);
        assert_eq!(
            map.get("id"),
            Some(&serde_norway::Value::String(
                "D0001-1432-01-add-auth-0".into()
            ))
        );
        assert!(body.starts_with("# Add Auth"));
    }

    #[test]
    fn test_parse_frontmatter_no_trailing_newline() {
        let content = "---\nroutine: develop\n---\nHello";
        let (map, body) = parse_frontmatter(content).unwrap();
        assert_eq!(
            map.get("routine"),
            Some(&serde_norway::Value::String("develop".into()))
        );
        assert_eq!(body, "Hello");
    }

    #[test]
    fn test_parse_frontmatter_empty_body() {
        let content = "---\nroutine: develop\n---\n";
        let (map, body) = parse_frontmatter(content).unwrap();
        assert_eq!(
            map.get("routine"),
            Some(&serde_norway::Value::String("develop".into()))
        );
        assert_eq!(body, "");
    }

    #[test]
    fn test_parse_frontmatter_no_closing() {
        let content = "---\nroutine: develop\nNo closing delimiter.\n";
        let (map, body) = parse_frontmatter(content).unwrap();
        // Treated as no frontmatter
        assert!(map.is_empty());
        assert_eq!(body, content);
    }

    #[test]
    fn test_parse_frontmatter_preserves_body_whitespace() {
        let content = "---\nroutine: develop\n---\n\nHello\n\nWorld\n";
        let (_, body) = parse_frontmatter(content).unwrap();
        assert_eq!(body, "\nHello\n\nWorld\n");
    }

    #[test]
    fn test_parse_frontmatter_custom_fields() {
        let content = "---\nroutine: develop\npriority: high\n---\nBody.\n";
        let (map, _) = parse_frontmatter(content).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(
            map.get("priority"),
            Some(&serde_norway::Value::String("high".into()))
        );
    }

    #[test]
    fn test_parse_frontmatter_rejects_duplicate_keys() {
        let content = "---\nroutine: develop\nroutine: rust-develop\n---\nBody.\n";
        let err = parse_frontmatter(content).unwrap_err();
        assert!(
            err.to_string()
                .contains("duplicate entry with key \"routine\""),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_parse_frontmatter_norway_words_are_strings() {
        // YAML 1.1 reads bare `on` and `no` as booleans; YAML 1.2 does not.
        let content = "---\non: no\n---\nBody.\n";
        let (map, _) = parse_frontmatter(content).unwrap();
        assert_eq!(
            map.get("on"),
            Some(&serde_norway::Value::String("no".into()))
        );
    }

    // --- Run lock (docs/reference/messages.md) ---

    /// The pid of a process that has exited and been reaped.
    fn dead_pid() -> u32 {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        pid
    }

    #[test]
    fn lock_holds_the_pid_and_is_deleted_on_drop() {
        let dir = TempDir::new().unwrap();
        assert_eq!(lock_state(dir.path()).unwrap(), LockState::Free);
        let lock = RunLock::acquire(dir.path()).unwrap().expect("lock is free");
        let text = std::fs::read_to_string(dir.path().join(LOCK_FILE)).unwrap();
        assert_eq!(text, std::process::id().to_string());
        assert_eq!(
            lock_state(dir.path()).unwrap(),
            LockState::Live(std::process::id())
        );
        drop(lock);
        assert!(!dir.path().join(LOCK_FILE).exists());
    }

    #[test]
    fn lock_with_a_live_pid_is_never_taken_over() {
        let dir = TempDir::new().unwrap();
        let mut sleeper = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = sleeper.id();
        std::fs::write(dir.path().join(LOCK_FILE), pid.to_string()).unwrap();
        assert_eq!(lock_state(dir.path()).unwrap(), LockState::Live(pid));
        assert!(RunLock::acquire(dir.path()).unwrap().is_none());
        let text = std::fs::read_to_string(dir.path().join(LOCK_FILE)).unwrap();
        assert_eq!(text, pid.to_string());
        sleeper.kill().unwrap();
        sleeper.wait().unwrap();
    }

    #[test]
    fn stale_lock_is_replaced() {
        for stale in [dead_pid().to_string(), String::new(), "0".to_string()] {
            let dir = TempDir::new().unwrap();
            std::fs::write(dir.path().join(LOCK_FILE), &stale).unwrap();
            assert_eq!(
                lock_state(dir.path()).unwrap(),
                LockState::Stale,
                "{stale:?}"
            );
            let lock = RunLock::acquire(dir.path()).unwrap().expect("stale lock");
            assert_eq!(
                lock_state(dir.path()).unwrap(),
                LockState::Live(std::process::id())
            );
            drop(lock);
            assert!(!dir.path().join(LOCK_FILE).exists());
        }
    }

    #[test]
    fn dropping_a_lock_keeps_another_process_lock() {
        let dir = TempDir::new().unwrap();
        let lock = RunLock::acquire(dir.path()).unwrap().unwrap();
        std::fs::write(dir.path().join(LOCK_FILE), "1").unwrap();
        drop(lock);
        assert!(dir.path().join(LOCK_FILE).exists());
    }
}
