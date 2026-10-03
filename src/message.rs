use crate::config::{self, AppConfig};
use crate::config::{INBOX_DIR, RUNS_DIR};
use crate::error::DecreeError;
use crate::machine::LoadedMachine;
use crate::runtime::MESSAGE_FILE;
use chrono::Local;
use chrono::Utc;
use serde_norway::{Mapping, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

// =================================================================
// Message (spec section 4)
// =================================================================

/// One message: YAML frontmatter plus a body (section 4). The frontmatter is a `Mapping`,
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

    /// Parse a message (section 4, Parsing and writing): an optional UTF-8 BOM, `\n` or
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

    /// Set frontmatter `key`, keeping its position if it exists, else appending it.
    pub fn set(&mut self, key: &str, value: impl Into<Value>) {
        self.frontmatter.insert(key.into(), value.into());
    }

    /// The file line of top-level key `key`, or 1 if it is not there.
    pub fn line_of(&self, key: &str) -> usize {
        self.key_lines.get(key).copied().unwrap_or(1)
    }
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

/// A new message id (section 4, Frontmatter keys): the UTC time, `YYYYMMDDTHHMMSSZ`, then
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

/// Queue `message` in `inbox/` (section 4, Lifecycle step 1): give it a new `id` as its
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

/// A message claimed from `inbox/` (section 4, Lifecycle step 2): it was renamed to
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
    let path = decree_dir.join(config::INBOX_DIR).join(file);
    let message = match Message::read(&path) {
        Err(MessageError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(None)
        }
        other => other,
    };
    let runs = decree_dir.join(config::RUNS_DIR);
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

/// The run lock in a run folder (section 4, Run lock).
pub const LOCK_FILE: &str = ".lock";

/// What a run's `.lock` says (section 4, Run lock).
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
    /// Take the lock of the run in `run_dir` (section 4, Run lock): create `.lock` with
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

/// Section 4, Lifecycle step 3: the machine a message names (`machine`, or its alias
/// `routine`, else `default_machine`) exists, and its `params` fit that machine's `data`.
/// `machine_ids` holds every machine, `machines` those that loaded; a machine that fails to
/// load is reported on its own, so its `params` are not checked. Returns the machine name,
/// or every error as `(file line, message)`.
pub fn validate(
    msg: &Message,
    machines: &BTreeMap<String, LoadedMachine>,
    machine_ids: &BTreeSet<String>,
    default_machine: Option<&str>,
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
        (Some(v), None) => Some(("machine", v)),
        (None, Some(v)) => Some(("routine", v)),
        (None, None) => None,
    };
    let (name, line) = match machine {
        Some((key, Value::String(name))) => (name.as_str(), msg.line_of(key)),
        Some((key, _)) => {
            return Err(vec![(
                msg.line_of(key),
                format!("`{key}` must be a string"),
            )])
        }
        None => match default_machine {
            Some(name) => (name, 1),
            None => {
                return Err(vec![(
                    1,
                    "no `machine` key and no default machine is configured".to_string(),
                )]);
            }
        },
    };
    if !machine_ids.contains(name) {
        let msg = match machine {
            Some(_) => format!("unknown machine `{name}`"),
            None => format!("no `machine` key, and the default machine `{name}` does not exist"),
        };
        return Err(vec![(line, msg)]);
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

/// Write `bytes` to `.<name>.tmp` beside `path`, then rename it over `path` (section 4).
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

// Known frontmatter field names (everything else is "custom").
const KNOWN_FIELDS: &[&str] = &["id", "chain", "seq", "routine", "migration", "trigger"];

/// A parsed message ID with the form `<chain>-<seq>`.
///
/// Chain format: `D<NNNN>-HHmm-<name>`
/// Full ID: `D<NNNN>-HHmm-<name>-<seq>`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageId {
    pub chain: String,
    pub seq: u32,
}

impl MessageId {
    /// Construct a new MessageId.
    pub fn new(chain: &str, seq: u32) -> Self {
        Self {
            chain: chain.to_string(),
            seq,
        }
    }

    /// Full ID string: `<chain>-<seq>`.
    pub fn full_id(&self) -> String {
        format!("{}-{}", self.chain, self.seq)
    }

    /// Parse a full message ID string like `D0001-1432-01-add-auth-0`.
    ///
    /// The last `-<number>` segment is the sequence; everything before is the chain.
    pub fn parse(s: &str) -> Result<Self, DecreeError> {
        let Some(last_dash) = s.rfind('-') else {
            return Err(DecreeError::Other(format!("invalid message ID: {s}")));
        };
        let chain = &s[..last_dash];
        let seq_str = &s[last_dash + 1..];
        let seq: u32 = seq_str
            .parse()
            .map_err(|_| DecreeError::Other(format!("invalid sequence in message ID: {s}")))?;
        if chain.is_empty() {
            return Err(DecreeError::Other(format!(
                "empty chain in message ID: {s}"
            )));
        }
        Ok(Self {
            chain: chain.to_string(),
            seq,
        })
    }

    /// Directory name for this message in `.decree/runs/`.
    pub fn run_dir_name(&self) -> String {
        self.full_id()
    }
}

impl std::fmt::Display for MessageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.full_id())
    }
}

// =================================================================
// Day counter and chain utilities
// =================================================================

/// Resolve the next day counter by scanning existing run directories.
///
/// Logic:
/// - Find the highest existing day counter in `.decree/runs/`
/// - Compare the current HHmm to the last entry's HHmm
/// - If current >= last, reuse the same day counter
/// - If current < last (clock wrapped midnight), increment
/// - First run starts at D0001
pub fn next_day_counter(project_root: &Path, current_hhmm: &str) -> Result<String, DecreeError> {
    let runs_dir = project_root.join(config::DECREE_DIR).join(config::RUNS_DIR);

    if !runs_dir.exists() {
        return Ok("D0001".to_string());
    }

    let mut entries: Vec<String> = std::fs::read_dir(&runs_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| name.starts_with('D'))
        .collect();

    if entries.is_empty() {
        return Ok("D0001".to_string());
    }

    entries.sort();

    // Safe: we checked `is_empty()` above.
    let last_entry = &entries[entries.len() - 1];

    let last_day = extract_day_counter(last_entry).unwrap_or("D0001");
    let last_day_num: u32 = last_day[1..].parse().unwrap_or(1);

    let last_hhmm = extract_hhmm(last_entry).unwrap_or("0000");

    if current_hhmm >= last_hhmm {
        Ok(format!("D{:04}", last_day_num))
    } else {
        Ok(format!("D{:04}", last_day_num + 1))
    }
}

/// Extract `D<NNNN>` from a run directory name like `D0001-1432-name-0`.
fn extract_day_counter(name: &str) -> Option<&str> {
    if name.len() >= 5 && name.starts_with('D') {
        Some(&name[..5])
    } else {
        None
    }
}

/// Extract `HHmm` from a run directory name like `D0001-1432-name-0`.
fn extract_hhmm(name: &str) -> Option<&str> {
    if name.len() >= 10 && name.as_bytes()[5] == b'-' {
        Some(&name[6..10])
    } else {
        None
    }
}

/// Build a chain ID: `D<NNNN>-HHmm-<name>`.
pub fn build_chain_id(day_counter: &str, hhmm: &str, name: &str) -> String {
    format!("{}-{}-{}", day_counter, hhmm, name)
}

/// List all run directories, sorted by name (chronological).
pub fn list_runs(project_root: &Path) -> Result<Vec<String>, DecreeError> {
    let runs_dir = project_root.join(config::DECREE_DIR).join(config::RUNS_DIR);

    if !runs_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries: Vec<String> = std::fs::read_dir(&runs_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();

    entries.sort();
    Ok(entries)
}

/// Find runs matching a prefix or full ID.
pub fn find_matching_runs(project_root: &Path, query: &str) -> Result<Vec<String>, DecreeError> {
    let runs = list_runs(project_root)?;
    let matches: Vec<String> = runs
        .into_iter()
        .filter(|name| name.starts_with(query) || name == query)
        .collect();
    Ok(matches)
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
// Migration types and functions
// =================================================================

/// List all `*.md` files in `.decree/migrations/`, sorted alphabetically.
pub fn list_migration_files(project_root: &Path) -> Result<Vec<String>, DecreeError> {
    let dir = project_root
        .join(config::DECREE_DIR)
        .join(config::MIGRATIONS_DIR);

    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut files: Vec<String> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file() && e.path().extension().is_some_and(|ext| ext == "md"))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();

    files.sort();
    Ok(files)
}

/// Read the processed migration tracker (`.decree/processed.md`).
/// Creates the file if missing.
pub fn read_processed(project_root: &Path) -> Result<HashSet<String>, DecreeError> {
    let path = project_root
        .join(config::DECREE_DIR)
        .join(config::PROCESSED_FILE);

    if !path.exists() {
        std::fs::write(&path, "")?;
        return Ok(HashSet::new());
    }

    let content = std::fs::read_to_string(&path)?;
    let set: HashSet<String> = content
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    Ok(set)
}

/// Return unprocessed migration filenames in alphabetical order.
pub fn unprocessed_migrations(project_root: &Path) -> Result<Vec<String>, DecreeError> {
    let all = list_migration_files(project_root)?;
    let processed = read_processed(project_root)?;
    Ok(all.into_iter().filter(|f| !processed.contains(f)).collect())
}

/// Append a filename to `.decree/processed.md`.
pub fn mark_processed(project_root: &Path, filename: &str) -> Result<(), DecreeError> {
    use std::io::Write;
    let path = project_root
        .join(config::DECREE_DIR)
        .join(config::PROCESSED_FILE);

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;

    writeln!(file, "{}", filename)?;
    Ok(())
}

/// Remove a filename from `.decree/processed.md` (for token-exhaustion retry).
pub fn unmark_processed(project_root: &Path, filename: &str) -> Result<(), DecreeError> {
    let path = project_root
        .join(config::DECREE_DIR)
        .join(config::PROCESSED_FILE);

    if !path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(&path)?;
    let new_lines: Vec<&str> = content
        .lines()
        .filter(|line| line.trim() != filename.trim())
        .collect();

    let mut new_content = new_lines.join("\n");
    if !new_content.is_empty() {
        new_content.push('\n');
    }

    std::fs::write(&path, new_content)?;
    Ok(())
}

// =================================================================
// Inbox message
// =================================================================

/// A parsed inbox message from `.decree/inbox/`.
///
/// Fields are `Option` before normalization. After `normalize()`, `id`,
/// `chain`, `seq`, and `routine` are guaranteed `Some`.
#[derive(Debug, Clone)]
pub struct InboxMessage {
    pub id: Option<String>,
    pub chain: Option<String>,
    pub seq: Option<u32>,
    pub routine: Option<String>,
    pub migration: Option<String>,
    /// How the run was triggered: "inbox", "chain", or "cron:<stem>".
    pub trigger: Option<String>,
    pub body: String,
    pub custom_fields: BTreeMap<String, serde_norway::Value>,
    pub filename: String,
}

impl InboxMessage {
    /// Parse an inbox message from its filename and raw content.
    pub fn parse(filename: &str, content: &str) -> Result<Self, DecreeError> {
        let (fields, body) = parse_frontmatter(content)?;

        let id = fields.get("id").and_then(value_as_string);
        let chain = fields.get("chain").and_then(value_as_string);
        let seq = fields.get("seq").and_then(value_as_u32);
        let routine = fields.get("routine").and_then(value_as_string);
        let migration = fields.get("migration").and_then(value_as_string);
        let trigger = fields.get("trigger").and_then(value_as_string);

        let custom_fields: BTreeMap<String, serde_norway::Value> = fields
            .into_iter()
            .filter(|(k, _)| !KNOWN_FIELDS.contains(&k.as_str()))
            .collect();

        Ok(Self {
            id,
            chain,
            seq,
            routine,
            migration,
            trigger,
            body,
            custom_fields,
            filename: filename.to_string(),
        })
    }

    /// Read and parse an inbox message from the filesystem.
    pub fn from_file(project_root: &Path, filename: &str) -> Result<Self, DecreeError> {
        let path = project_root
            .join(config::DECREE_DIR)
            .join(config::INBOX_DIR)
            .join(filename);

        let content = std::fs::read_to_string(&path)?;
        Self::parse(filename, &content)
    }

    /// Whether all required fields are present (no normalization needed).
    pub fn is_complete(&self) -> bool {
        self.id.is_some() && self.chain.is_some() && self.seq.is_some() && self.routine.is_some()
    }

    /// Normalize the message, filling in missing fields.
    ///
    /// Returns `true` if the message was modified and should be rewritten.
    pub fn normalize(
        &mut self,
        project_root: &Path,
        config: &AppConfig,
    ) -> Result<bool, DecreeError> {
        let mut modified = false;

        // Default trigger to "inbox" when absent (applies even to otherwise-complete messages).
        if self.trigger.is_none() {
            self.trigger = Some("inbox".to_string());
            modified = true;
        }

        if self.is_complete() {
            return Ok(modified);
        }

        // 1. Derive chain and seq from filename if missing
        if self.chain.is_none() || self.seq.is_none() {
            // `<chain>-<seq>.md`, e.g. `D0001-1432-01-add-auth-0.md`.
            let parsed = self
                .filename
                .strip_suffix(".md")
                .and_then(|stem| stem.rsplit_once('-'))
                .filter(|(chain, _)| !chain.is_empty())
                .and_then(|(chain, seq)| Some((chain.to_string(), seq.parse::<u32>().ok()?)));
            if let Some((chain, seq)) = parsed {
                if self.chain.is_none() {
                    self.chain = Some(chain);
                }
                if self.seq.is_none() {
                    self.seq = Some(seq);
                }
            }
        }

        // 2. Generate new chain if still missing; use migration stem, then filename stem.
        if self.chain.is_none() {
            let now = Local::now();
            let hhmm = now.format("%H%M").to_string();
            let day = next_day_counter(project_root, &hhmm)?;
            let name: String = if let Some(ref mig) = self.migration {
                mig.trim_end_matches(".md").to_string()
            } else {
                self.filename
                    .strip_suffix(".md")
                    .unwrap_or(&self.filename)
                    .to_string()
            };
            self.chain = Some(build_chain_id(&day, &hhmm, &name));
        }

        // 3. Default seq to 0 if still missing
        if self.seq.is_none() {
            self.seq = Some(0);
        }

        // 4. Recompute id from chain + seq
        let chain = self
            .chain
            .as_ref()
            .ok_or_else(|| DecreeError::Other("chain not set during normalization".into()))?;
        let seq = self
            .seq
            .ok_or_else(|| DecreeError::Other("seq not set during normalization".into()))?;
        self.id = Some(format!("{}-{}", chain, seq));

        // 5. Routine selection
        if self.routine.is_none() {
            self.routine = Some(select_routine(config));
        }

        Ok(true)
    }

    /// Serialize the message to markdown with YAML frontmatter.
    pub fn serialize(&self) -> String {
        let mut map = serde_norway::Mapping::new();
        let str_key = |k: &str| serde_norway::Value::String(k.into());
        let str_val = |v: &str| serde_norway::Value::String(v.into());

        if let Some(ref v) = self.id {
            map.insert(str_key("id"), str_val(v));
        }
        if let Some(ref v) = self.chain {
            map.insert(str_key("chain"), str_val(v));
        }
        if let Some(seq) = self.seq {
            let seq_val = serde_norway::to_value(seq)
                .unwrap_or_else(|_| serde_norway::Value::String(seq.to_string()));
            map.insert(str_key("seq"), seq_val);
        }
        if let Some(ref v) = self.routine {
            map.insert(str_key("routine"), str_val(v));
        }
        if let Some(ref v) = self.migration {
            map.insert(str_key("migration"), str_val(v));
        }
        if let Some(ref v) = self.trigger {
            map.insert(str_key("trigger"), str_val(v));
        }

        for (k, v) in &self.custom_fields {
            map.insert(str_key(k), v.clone());
        }

        let yaml = serde_norway::to_string(&serde_norway::Value::Mapping(map)).unwrap_or_default();

        if self.body.is_empty() {
            format!("---\n{}---\n", yaml)
        } else {
            format!("---\n{}---\n{}", yaml, self.body)
        }
    }

    /// Write the message to `.decree/inbox/`.
    pub fn write_to_inbox(&self, project_root: &Path) -> Result<(), DecreeError> {
        let path = project_root
            .join(config::DECREE_DIR)
            .join(config::INBOX_DIR)
            .join(&self.filename);

        std::fs::write(&path, self.serialize())?;
        Ok(())
    }
}

/// List all `*.md` files in `.decree/inbox/`, sorted alphabetically.
pub fn list_inbox_messages(project_root: &Path) -> Result<Vec<String>, DecreeError> {
    let dir = project_root
        .join(config::DECREE_DIR)
        .join(config::INBOX_DIR);

    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut files: Vec<String> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file() && e.path().extension().is_some_and(|ext| ext == "md"))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();

    files.sort();
    Ok(files)
}

// =================================================================
// Routine listing and router
// =================================================================

/// Information about an available routine.
#[derive(Debug, Clone)]
pub struct RoutineInfo {
    /// Routine name (relative path without extension, e.g., "develop").
    pub name: String,
    /// Description extracted from comment header.
    pub description: String,
}

/// List available routines, respecting the config registry.
///
/// - With a `routines` section: only enabled project-local routines are listed.
/// - Without a `routines` section (legacy): all filesystem routines are listed.
/// - With `routine_source`: enabled shared routines are also included.
pub fn list_routines(
    project_root: &Path,
    config: &AppConfig,
) -> Result<Vec<RoutineInfo>, DecreeError> {
    let routines_dir = project_root
        .join(config::DECREE_DIR)
        .join(config::ROUTINES_DIR);

    let mut routines = Vec::new();

    if let Some(ref registry) = config.routines {
        // Strict mode: only enabled routines from registry
        for (name, entry) in registry {
            if !entry.is_active() {
                continue;
            }
            if let Ok(path) = crate::routine::find_routine_script(&routines_dir, name) {
                let content = std::fs::read_to_string(&path)?;
                let description = extract_routine_description(&content);
                routines.push(RoutineInfo {
                    name: name.clone(),
                    description,
                });
            }
        }
    } else {
        // Legacy mode: all filesystem routines
        routines = scan_routines_dir(&routines_dir)?;
    }

    // Add enabled shared routines
    if let Some(shared_dir) = config.resolved_routine_source() {
        if let Some(ref shared_registry) = config.shared_routines {
            for (name, entry) in shared_registry {
                if !entry.is_active() {
                    continue;
                }
                // Skip if already listed from project-local (precedence)
                if routines.iter().any(|r| r.name == *name) {
                    continue;
                }
                if let Ok(path) = crate::routine::find_routine_script(&shared_dir, name) {
                    let content = std::fs::read_to_string(&path)?;
                    let description = extract_routine_description(&content);
                    routines.push(RoutineInfo {
                        name: name.clone(),
                        description,
                    });
                }
            }
        }
    }

    routines.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(routines)
}

/// Scan a routines directory for all script files (legacy mode).
fn scan_routines_dir(routines_dir: &Path) -> Result<Vec<RoutineInfo>, DecreeError> {
    if !routines_dir.exists() {
        return Ok(Vec::new());
    }

    let mut routines = Vec::new();

    for entry in WalkDir::new(routines_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        let rel = path
            .strip_prefix(routines_dir)
            .map_err(|e| DecreeError::Other(e.to_string()))?;

        let name = rel.with_extension("").to_string_lossy().to_string();
        if name.is_empty() {
            continue;
        }

        let content = std::fs::read_to_string(path)?;
        let description = extract_routine_description(&content);

        routines.push(RoutineInfo { name, description });
    }

    Ok(routines)
}

/// Extract a description from a routine script's comment header.
///
/// Expected format:
/// ```text
/// #!/usr/bin/env bash
/// # Title
/// #
/// # Description line 1
/// # Description line 2
/// ```
pub fn extract_routine_description(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();

    // Skip shebang if present
    let start = if lines.first().is_some_and(|l| l.starts_with("#!")) {
        1
    } else {
        0
    };

    // Skip title line (# Title) and blank comment (#)
    let desc_start = start + 2;

    if desc_start >= lines.len() {
        return String::new();
    }

    let mut desc_lines = Vec::new();
    for line in &lines[desc_start..] {
        if let Some(text) = line.strip_prefix("# ") {
            desc_lines.push(text);
        } else {
            break;
        }
    }

    desc_lines.join(" ")
}

/// Build the router prompt for AI-based routine selection.
///
/// Reads `.decree/router.md` and populates `{routines}` and `{message}`.
pub fn build_router_prompt(
    project_root: &Path,
    routines: &[RoutineInfo],
    message_body: &str,
) -> Result<String, DecreeError> {
    let router_path = project_root
        .join(config::DECREE_DIR)
        .join(config::ROUTER_FILE);

    let template = std::fs::read_to_string(&router_path)?;

    let routines_text: String = routines
        .iter()
        .map(|r| {
            if r.description.is_empty() {
                format!("- **{}**", r.name)
            } else {
                format!("- **{}**: {}", r.name, r.description)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = template
        .replace("{routines}", &routines_text)
        .replace("{message}", message_body);

    Ok(prompt)
}

/// The routine for a message that names none: config `default_routine`, else `"develop"`.
/// Choosing one with a model is a `choose: model` state now (spec section 7).
fn select_routine(config: &AppConfig) -> String {
    if config.default_routine.is_empty() {
        "develop".to_string()
    } else {
        config.default_routine.clone()
    }
}

// =================================================================
// Helpers
// =================================================================

fn value_as_string(v: &serde_norway::Value) -> Option<String> {
    match v {
        serde_norway::Value::String(s) => Some(s.clone()),
        serde_norway::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn value_as_u32(v: &serde_norway::Value) -> Option<u32> {
    match v {
        serde_norway::Value::Number(n) => n.as_u64().map(|n| n as u32),
        serde_norway::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

// =================================================================
// Tests
// =================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // --- Message (section 4, Parsing and writing) ---

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
        let m = crate::machine::load_machine_text("x", Path::new("machines/x.yml"), text).unwrap();
        let machines = BTreeMap::from([("x".to_string(), m)]);
        let ids = machines.keys().cloned().collect();
        (machines, ids)
    }

    fn validated(
        text: &str,
        default_machine: Option<&str>,
    ) -> Result<String, Vec<(usize, String)>> {
        let (machines, ids) = machines();
        validate(
            &Message::parse(text).unwrap(),
            &machines,
            &ids,
            default_machine,
        )
    }

    #[test]
    fn validate_reads_routine_as_machine() {
        assert_eq!(validated("---\nroutine: x\n---\n", None).unwrap(), "x");
        assert_eq!(validated("---\nmachine: x\n---\n", None).unwrap(), "x");
        assert_eq!(validated("body\n", Some("x")).unwrap(), "x");
    }

    #[test]
    fn validate_rejects_unknown_machine_and_params() {
        let errors = validated("---\nid: a\nmachine: nope\n---\n", Some("x")).unwrap_err();
        assert_eq!(errors, [(3, "unknown machine `nope`".to_string())]);
        let errors = validated(
            "---\nmachine: x\nparams:\n  rounds: two\n  other: 1\n---\n",
            None,
        )
        .unwrap_err();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(
            errors[0].1.contains("`rounds` must be of type `int`"),
            "{errors:?}"
        );
        assert!(errors[1].1.contains("unknown param `other`"), "{errors:?}");
        let errors = validated("---\nmachine: x\nroutine: x\n---\n", None).unwrap_err();
        assert!(errors[0].1.contains("both"), "{errors:?}");
        assert!(validated("body\n", None).is_err());
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
        assert!(id[17..].bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
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
            .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(left, 0);
    }

    #[test]
    fn message_non_mapping_is_an_error() {
        let (_, msg) = parse_err("---\n- a\n- b\n---\n");
        assert!(msg.contains("not a YAML mapping"), "{msg}");
    }

    // --- MessageId tests (existing) ---

    #[test]
    fn test_parse_message_id() {
        let id = MessageId::parse("D0001-1432-01-add-auth-0").unwrap();
        assert_eq!(id.chain, "D0001-1432-01-add-auth");
        assert_eq!(id.seq, 0);
        assert_eq!(id.full_id(), "D0001-1432-01-add-auth-0");
    }

    #[test]
    fn test_parse_followup() {
        let id = MessageId::parse("D0001-1432-01-add-auth-1").unwrap();
        assert_eq!(id.seq, 1);
    }

    #[test]
    fn test_parse_invalid() {
        assert!(MessageId::parse("invalid").is_err());
        assert!(MessageId::parse("-0").is_err());
    }

    #[test]
    fn test_display() {
        let id = MessageId::new("D0001-1432-develop", 0);
        assert_eq!(format!("{id}"), "D0001-1432-develop-0");
    }

    #[test]
    fn test_extract_day_counter() {
        assert_eq!(extract_day_counter("D0001-1432-test-0"), Some("D0001"));
        assert_eq!(extract_day_counter("D0042-0900-foo-1"), Some("D0042"));
    }

    #[test]
    fn test_extract_hhmm() {
        assert_eq!(extract_hhmm("D0001-1432-test-0"), Some("1432"));
        assert_eq!(extract_hhmm("D0001-0900-foo-0"), Some("0900"));
    }

    #[test]
    fn test_build_chain_id() {
        assert_eq!(
            build_chain_id("D0001", "1432", "01-add-auth"),
            "D0001-1432-01-add-auth"
        );
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

    // --- Migration tests ---

    fn setup_decree_dir(dir: &TempDir) {
        let decree = dir.path().join(".decree");
        std::fs::create_dir_all(decree.join("migrations")).unwrap();
        std::fs::create_dir_all(decree.join("inbox")).unwrap();
        std::fs::create_dir_all(decree.join("routines")).unwrap();
        std::fs::create_dir_all(decree.join("runs")).unwrap();
        std::fs::write(decree.join("processed.md"), "").unwrap();
        std::fs::write(decree.join("config.yml"), "max_attempts: 3\n").unwrap();
    }

    #[test]
    fn test_list_migration_files_empty() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let files = list_migration_files(dir.path()).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_list_migration_files_sorted() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let mig_dir = dir.path().join(".decree/migrations");
        std::fs::write(mig_dir.join("03-api.md"), "").unwrap();
        std::fs::write(mig_dir.join("01-auth.md"), "").unwrap();
        std::fs::write(mig_dir.join("02-db.md"), "").unwrap();
        // Non-md file should be excluded
        std::fs::write(mig_dir.join("notes.txt"), "").unwrap();

        let files = list_migration_files(dir.path()).unwrap();
        assert_eq!(files, vec!["01-auth.md", "02-db.md", "03-api.md"]);
    }

    #[test]
    fn test_list_migration_files_no_dir() {
        let dir = TempDir::new().unwrap();
        // No .decree at all
        let files = list_migration_files(dir.path()).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_read_processed_empty() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let processed = read_processed(dir.path()).unwrap();
        assert!(processed.is_empty());
    }

    #[test]
    fn test_read_processed_with_entries() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        std::fs::write(
            dir.path().join(".decree/processed.md"),
            "01-auth.md\n02-db.md\n",
        )
        .unwrap();

        let processed = read_processed(dir.path()).unwrap();
        assert_eq!(processed.len(), 2);
        assert!(processed.contains("01-auth.md"));
        assert!(processed.contains("02-db.md"));
    }

    #[test]
    fn test_read_processed_creates_if_missing() {
        let dir = TempDir::new().unwrap();
        let decree = dir.path().join(".decree");
        std::fs::create_dir_all(&decree).unwrap();
        // No processed.md file
        assert!(!decree.join("processed.md").exists());

        let processed = read_processed(dir.path()).unwrap();
        assert!(processed.is_empty());
        assert!(decree.join("processed.md").exists());
    }

    #[test]
    fn test_unprocessed_migrations() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let mig_dir = dir.path().join(".decree/migrations");
        std::fs::write(mig_dir.join("01-auth.md"), "").unwrap();
        std::fs::write(mig_dir.join("02-db.md"), "").unwrap();
        std::fs::write(mig_dir.join("03-api.md"), "").unwrap();
        std::fs::write(dir.path().join(".decree/processed.md"), "01-auth.md\n").unwrap();

        let unprocessed = unprocessed_migrations(dir.path()).unwrap();
        assert_eq!(unprocessed, vec!["02-db.md", "03-api.md"]);
    }

    #[test]
    fn test_unprocessed_migrations_all_processed() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let mig_dir = dir.path().join(".decree/migrations");
        std::fs::write(mig_dir.join("01-auth.md"), "").unwrap();
        std::fs::write(dir.path().join(".decree/processed.md"), "01-auth.md\n").unwrap();

        let unprocessed = unprocessed_migrations(dir.path()).unwrap();
        assert!(unprocessed.is_empty());
    }

    #[test]
    fn test_mark_processed() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        mark_processed(dir.path(), "01-auth.md").unwrap();
        mark_processed(dir.path(), "02-db.md").unwrap();

        let processed = read_processed(dir.path()).unwrap();
        assert_eq!(processed.len(), 2);
        assert!(processed.contains("01-auth.md"));
        assert!(processed.contains("02-db.md"));
    }

    // --- InboxMessage tests ---

    #[test]
    fn test_inbox_parse_full_message() {
        let content = "---\n\
            id: D0001-1432-01-add-auth-0\n\
            chain: D0001-1432-01-add-auth\n\
            seq: 0\n\
            routine: develop\n\
            migration: 01-add-auth.md\n\
            ---\n\
            Add auth.\n";

        let msg = InboxMessage::parse("D0001-1432-01-add-auth-0.md", content).unwrap();
        assert_eq!(msg.id.as_deref(), Some("D0001-1432-01-add-auth-0"));
        assert_eq!(msg.chain.as_deref(), Some("D0001-1432-01-add-auth"));
        assert_eq!(msg.seq, Some(0));
        assert_eq!(msg.routine.as_deref(), Some("develop"));
        assert_eq!(msg.migration.as_deref(), Some("01-add-auth.md"));
        assert!(msg.is_complete());
    }

    #[test]
    fn test_inbox_parse_bare_message() {
        let content = "Fix type errors in src/auth.rs.\n";
        let msg = InboxMessage::parse("fix-errors.md", content).unwrap();
        assert!(msg.id.is_none());
        assert!(msg.chain.is_none());
        assert!(msg.seq.is_none());
        assert!(msg.routine.is_none());
        assert_eq!(msg.body, "Fix type errors in src/auth.rs.\n");
        assert!(!msg.is_complete());
    }

    #[test]
    fn test_inbox_parse_partial_frontmatter() {
        let content = "---\nroutine: rust-develop\n---\nFix errors.\n";
        let msg = InboxMessage::parse("D0001-1432-fix-0.md", content).unwrap();
        assert!(msg.id.is_none());
        assert!(msg.chain.is_none());
        assert!(msg.seq.is_none());
        assert_eq!(msg.routine.as_deref(), Some("rust-develop"));
        assert!(!msg.is_complete());
    }

    #[test]
    fn test_inbox_parse_custom_fields() {
        let content = "---\nroutine: develop\npriority: high\ntags: urgent\n---\nBody.\n";
        let msg = InboxMessage::parse("test.md", content).unwrap();
        assert_eq!(msg.routine.as_deref(), Some("develop"));
        assert_eq!(msg.custom_fields.len(), 2);
        assert_eq!(
            msg.custom_fields.get("priority"),
            Some(&serde_norway::Value::String("high".into()))
        );
        assert_eq!(
            msg.custom_fields.get("tags"),
            Some(&serde_norway::Value::String("urgent".into()))
        );
    }

    #[test]
    fn test_inbox_parse_empty_body_valid() {
        let content = "---\nroutine: develop\n---\n";
        let msg = InboxMessage::parse("test.md", content).unwrap();
        assert_eq!(msg.body, "");
        assert_eq!(msg.routine.as_deref(), Some("develop"));
    }

    #[test]
    fn test_inbox_parse_entirely_empty() {
        let msg = InboxMessage::parse("test.md", "").unwrap();
        assert!(msg.id.is_none());
        assert_eq!(msg.body, "");
    }

    // --- Normalization tests ---

    #[test]
    fn test_normalize_complete_message_not_modified() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig::default();

        let mut msg = InboxMessage {
            id: Some("D0001-1432-test-0".into()),
            chain: Some("D0001-1432-test".into()),
            seq: Some(0),
            routine: Some("develop".into()),
            migration: None,
            trigger: Some("inbox".into()),
            body: "Test.".into(),
            custom_fields: BTreeMap::new(),
            filename: "D0001-1432-test-0.md".into(),
        };

        let modified = msg.normalize(dir.path(), &config).unwrap();
        assert!(!modified, "complete message should not be modified");
    }

    #[test]
    fn test_normalize_from_filename() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig::default();

        let mut msg = InboxMessage::parse("D0001-1432-01-add-auth-0.md", "Add auth.\n").unwrap();

        let modified = msg.normalize(dir.path(), &config).unwrap();
        assert!(modified);
        assert_eq!(msg.chain.as_deref(), Some("D0001-1432-01-add-auth"));
        assert_eq!(msg.seq, Some(0));
        assert_eq!(msg.id.as_deref(), Some("D0001-1432-01-add-auth-0"));
        // Routine falls back to config default
        assert_eq!(msg.routine.as_deref(), Some("develop"));
    }

    #[test]
    fn test_normalize_preserves_frontmatter_over_filename() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig::default();

        let content = "---\nchain: my-chain\nseq: 5\n---\nBody.\n";
        let mut msg = InboxMessage::parse("D0001-1432-01-add-auth-0.md", content).unwrap();

        msg.normalize(dir.path(), &config).unwrap();
        // Frontmatter values should take priority over filename
        assert_eq!(msg.chain.as_deref(), Some("my-chain"));
        assert_eq!(msg.seq, Some(5));
        assert_eq!(msg.id.as_deref(), Some("my-chain-5"));
    }

    #[test]
    fn test_normalize_generates_chain_when_missing() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig::default();

        // Filename without chain-seq pattern
        let mut msg = InboxMessage::parse("random-name.md", "Body.\n").unwrap();

        let modified = msg.normalize(dir.path(), &config).unwrap();
        assert!(modified);
        assert!(msg.chain.is_some());
        assert!(msg.chain.as_ref().unwrap().starts_with("D0001-"));
        assert_eq!(msg.seq, Some(0));
        assert!(msg.id.is_some());
    }

    #[test]
    fn test_normalize_routine_fallback_config_default() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig {
            default_routine: "rust-develop".to_string(),
            ..AppConfig::default()
        };

        let mut msg = InboxMessage::parse("D0001-1432-test-0.md", "Body.\n").unwrap();
        msg.normalize(dir.path(), &config).unwrap();
        assert_eq!(msg.routine.as_deref(), Some("rust-develop"));
    }

    #[test]
    fn test_normalize_routine_ultimate_fallback() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig {
            default_routine: String::new(),
            ..AppConfig::default()
        };

        let mut msg = InboxMessage::parse("D0001-1432-test-0.md", "Body.\n").unwrap();
        msg.normalize(dir.path(), &config).unwrap();
        assert_eq!(msg.routine.as_deref(), Some("develop"));
    }

    #[test]
    fn test_normalize_custom_fields_preserved() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig::default();

        let content = "---\npriority: high\ntags: urgent\n---\nBody.\n";
        let mut msg = InboxMessage::parse("D0001-1432-test-0.md", content).unwrap();

        msg.normalize(dir.path(), &config).unwrap();
        assert_eq!(msg.custom_fields.len(), 2);
        assert_eq!(
            msg.custom_fields.get("priority"),
            Some(&serde_norway::Value::String("high".into()))
        );
    }

    #[test]
    fn test_normalize_migration_field_preserved() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig::default();

        let content = "---\nmigration: 01-auth.md\n---\nBody.\n";
        let mut msg = InboxMessage::parse("D0001-1432-01-auth-0.md", content).unwrap();

        msg.normalize(dir.path(), &config).unwrap();
        assert_eq!(msg.migration.as_deref(), Some("01-auth.md"));
    }

    #[test]
    fn test_normalize_trigger_defaults_to_inbox() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig::default();

        let mut msg = InboxMessage::parse("D0001-1432-test-0.md", "Body.\n").unwrap();
        assert!(msg.trigger.is_none());
        msg.normalize(dir.path(), &config).unwrap();
        assert_eq!(msg.trigger.as_deref(), Some("inbox"));
    }

    #[test]
    fn test_normalize_trigger_preserved_when_set() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig::default();

        let content = "---\ntrigger: chain\n---\nBody.\n";
        let mut msg = InboxMessage::parse("D0001-1432-test-0.md", content).unwrap();
        msg.normalize(dir.path(), &config).unwrap();
        assert_eq!(msg.trigger.as_deref(), Some("chain"));
    }

    #[test]
    fn test_normalize_trigger_defaults_on_complete_message() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig::default();

        // Complete message but no trigger — should still be defaulted
        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\nroutine: develop\n---\nBody.\n";
        let mut msg = InboxMessage::parse("D0001-1432-test-0.md", content).unwrap();
        assert!(msg.trigger.is_none());
        let modified = msg.normalize(dir.path(), &config).unwrap();
        assert!(modified);
        assert_eq!(msg.trigger.as_deref(), Some("inbox"));
    }

    #[test]
    fn test_normalize_filename_stem_as_chain_fallback() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig::default();

        // File "fix-errors.md" has no chain-seq pattern and no migration field
        let mut msg = InboxMessage::parse("fix-errors.md", "Body.\n").unwrap();
        msg.normalize(dir.path(), &config).unwrap();

        let chain = msg.chain.as_deref().unwrap();
        assert!(
            chain.contains("fix-errors"),
            "chain should use filename stem: {chain}"
        );
        assert_eq!(msg.id.as_deref().unwrap(), format!("{chain}-0"));
    }

    #[test]
    fn test_normalize_migration_stem_takes_priority_over_filename() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig::default();

        // File "random.md" with migration: 01-auth.md — migration stem wins
        let content = "---\nmigration: 01-auth.md\n---\nBody.\n";
        let mut msg = InboxMessage::parse("random.md", content).unwrap();
        msg.normalize(dir.path(), &config).unwrap();

        let chain = msg.chain.as_deref().unwrap();
        assert!(
            chain.contains("01-auth"),
            "migration stem should take priority: {chain}"
        );
        assert!(
            !chain.contains("random"),
            "filename stem should not appear: {chain}"
        );
    }

    // --- Serialization tests ---

    #[test]
    fn test_serialize_full_message() {
        let msg = InboxMessage {
            id: Some("D0001-1432-test-0".into()),
            chain: Some("D0001-1432-test".into()),
            seq: Some(0),
            routine: Some("develop".into()),
            migration: Some("01-test.md".into()),
            trigger: Some("inbox".into()),
            body: "Hello.\n".into(),
            custom_fields: BTreeMap::new(),
            filename: "D0001-1432-test-0.md".into(),
        };

        let output = msg.serialize();
        assert!(output.starts_with("---\n"));
        assert!(output.contains("id: D0001-1432-test-0"));
        assert!(output.contains("chain: D0001-1432-test"));
        assert!(output.contains("seq: 0"));
        assert!(output.contains("routine: develop"));
        assert!(output.contains("migration: 01-test.md"));
        assert!(output.ends_with("Hello.\n"));
    }

    #[test]
    fn test_serialize_with_custom_fields() {
        let mut custom = BTreeMap::new();
        custom.insert(
            "priority".to_string(),
            serde_norway::Value::String("high".into()),
        );

        let msg = InboxMessage {
            id: Some("D0001-1432-test-0".into()),
            chain: Some("D0001-1432-test".into()),
            seq: Some(0),
            routine: Some("develop".into()),
            migration: None,
            trigger: None,
            body: "Body.\n".into(),
            custom_fields: custom,
            filename: "D0001-1432-test-0.md".into(),
        };

        let output = msg.serialize();
        assert!(output.contains("priority: high"));
    }

    #[test]
    fn test_serialize_empty_body() {
        let msg = InboxMessage {
            id: Some("D0001-1432-test-0".into()),
            chain: Some("D0001-1432-test".into()),
            seq: Some(0),
            routine: Some("develop".into()),
            migration: None,
            trigger: None,
            body: String::new(),
            custom_fields: BTreeMap::new(),
            filename: "D0001-1432-test-0.md".into(),
        };

        let output = msg.serialize();
        assert!(output.starts_with("---\n"));
        assert!(output.ends_with("---\n"));
    }

    #[test]
    fn test_serialize_roundtrip() {
        let original = InboxMessage {
            id: Some("D0001-1432-test-0".into()),
            chain: Some("D0001-1432-test".into()),
            seq: Some(0),
            routine: Some("develop".into()),
            migration: Some("01-test.md".into()),
            trigger: Some("chain".into()),
            body: "Hello world.\n".into(),
            custom_fields: BTreeMap::new(),
            filename: "D0001-1432-test-0.md".into(),
        };

        let serialized = original.serialize();
        let parsed = InboxMessage::parse("D0001-1432-test-0.md", &serialized).unwrap();

        assert_eq!(parsed.id, original.id);
        assert_eq!(parsed.chain, original.chain);
        assert_eq!(parsed.seq, original.seq);
        assert_eq!(parsed.routine, original.routine);
        assert_eq!(parsed.migration, original.migration);
        assert_eq!(parsed.trigger, original.trigger);
        assert_eq!(parsed.body, original.body);
    }

    // --- write_to_inbox / from_file tests ---

    #[test]
    fn test_write_and_read_inbox() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        let msg = InboxMessage {
            id: Some("D0001-1432-test-0".into()),
            chain: Some("D0001-1432-test".into()),
            seq: Some(0),
            routine: Some("develop".into()),
            migration: None,
            trigger: Some("inbox".into()),
            body: "Hello.\n".into(),
            custom_fields: BTreeMap::new(),
            filename: "D0001-1432-test-0.md".into(),
        };

        msg.write_to_inbox(dir.path()).unwrap();

        let read_back = InboxMessage::from_file(dir.path(), "D0001-1432-test-0.md").unwrap();
        assert_eq!(read_back.id.as_deref(), Some("D0001-1432-test-0"));
        assert_eq!(read_back.chain.as_deref(), Some("D0001-1432-test"));
        assert_eq!(read_back.seq, Some(0));
        assert_eq!(read_back.routine.as_deref(), Some("develop"));
        assert_eq!(read_back.body, "Hello.\n");
    }

    // --- list_inbox_messages tests ---

    #[test]
    fn test_list_inbox_messages_empty() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let msgs = list_inbox_messages(dir.path()).unwrap();
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_list_inbox_messages_sorted() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let inbox = dir.path().join(".decree/inbox");
        std::fs::write(inbox.join("D0001-1432-beta-0.md"), "").unwrap();
        std::fs::write(inbox.join("D0001-1432-alpha-0.md"), "").unwrap();
        // Non-md should be excluded
        std::fs::write(inbox.join("notes.txt"), "").unwrap();

        let msgs = list_inbox_messages(dir.path()).unwrap();
        assert_eq!(msgs, vec!["D0001-1432-alpha-0.md", "D0001-1432-beta-0.md"]);
    }

    // --- Routine listing tests ---

    #[test]
    fn test_extract_routine_description_standard() {
        let content = "#!/usr/bin/env bash\n# Develop\n#\n# General-purpose development.\n";
        let desc = extract_routine_description(content);
        assert_eq!(desc, "General-purpose development.");
    }

    #[test]
    fn test_extract_routine_description_multiline() {
        let content =
            "#!/usr/bin/env bash\n# Develop\n#\n# Line one.\n# Line two.\nset -euo pipefail\n";
        let desc = extract_routine_description(content);
        assert_eq!(desc, "Line one. Line two.");
    }

    #[test]
    fn test_extract_routine_description_no_desc() {
        let content = "#!/usr/bin/env bash\n# Title\n#\nset -euo pipefail\n";
        let desc = extract_routine_description(content);
        assert_eq!(desc, "");
    }

    #[test]
    fn test_extract_routine_description_no_shebang() {
        let content = "# Title\n#\n# Description here.\n";
        let desc = extract_routine_description(content);
        assert_eq!(desc, "Description here.");
    }

    #[test]
    fn test_list_routines() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        let routines_dir = dir.path().join(".decree/routines");
        std::fs::write(
            routines_dir.join("develop.sh"),
            "#!/usr/bin/env bash\n# Develop\n#\n# General purpose.\n",
        )
        .unwrap();
        std::fs::write(
            routines_dir.join("rust-develop.sh"),
            "#!/usr/bin/env bash\n# Rust Develop\n#\n# Rust specific.\n",
        )
        .unwrap();

        let config = AppConfig::default();
        let routines = list_routines(dir.path(), &config).unwrap();
        assert_eq!(routines.len(), 2);
        assert_eq!(routines[0].name, "develop");
        assert_eq!(routines[0].description, "General purpose.");
        assert_eq!(routines[1].name, "rust-develop");
        assert_eq!(routines[1].description, "Rust specific.");
    }

    #[test]
    fn test_list_routines_nested() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        let routines_dir = dir.path().join(".decree/routines");
        std::fs::create_dir_all(routines_dir.join("hooks")).unwrap();
        std::fs::write(
            routines_dir.join("develop.sh"),
            "#!/usr/bin/env bash\n# Develop\n#\n# General.\n",
        )
        .unwrap();
        std::fs::write(
            routines_dir.join("hooks/git-baseline.sh"),
            "#!/usr/bin/env bash\n# Git Baseline\n#\n# Captures baseline.\n",
        )
        .unwrap();

        let config = AppConfig::default();
        let routines = list_routines(dir.path(), &config).unwrap();
        assert_eq!(routines.len(), 2);
        let names: Vec<&str> = routines.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"develop"));
        assert!(names.contains(&"hooks/git-baseline"));
    }

    #[test]
    fn test_list_routines_no_dir() {
        let dir = TempDir::new().unwrap();
        let config = AppConfig::default();
        let routines = list_routines(dir.path(), &config).unwrap();
        assert!(routines.is_empty());
    }

    // --- Router prompt tests ---

    #[test]
    fn test_build_router_prompt() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        std::fs::write(
            dir.path().join(".decree/router.md"),
            "Select routine.\n\n## Routines\n{routines}\n\n## Message\n{message}\n",
        )
        .unwrap();

        let routines = vec![
            RoutineInfo {
                name: "develop".into(),
                description: "General purpose.".into(),
            },
            RoutineInfo {
                name: "rust-develop".into(),
                description: "Rust specific.".into(),
            },
        ];

        let prompt = build_router_prompt(dir.path(), &routines, "Add auth.").unwrap();

        assert!(prompt.contains("- **develop**: General purpose."));
        assert!(prompt.contains("- **rust-develop**: Rust specific."));
        assert!(prompt.contains("Add auth."));
    }

    #[test]
    fn test_build_router_prompt_no_description() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        std::fs::write(
            dir.path().join(".decree/router.md"),
            "{routines}\n{message}\n",
        )
        .unwrap();

        let routines = vec![RoutineInfo {
            name: "develop".into(),
            description: String::new(),
        }];

        let prompt = build_router_prompt(dir.path(), &routines, "Body.").unwrap();
        assert!(prompt.contains("- **develop**"));
        assert!(!prompt.contains("- **develop**:"));
    }

    // --- Run lock (section 4) ---

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

    // --- value helper tests ---

    #[test]
    fn test_value_as_string() {
        assert_eq!(
            value_as_string(&serde_norway::Value::String("hello".into())),
            Some("hello".to_string())
        );
        assert_eq!(value_as_string(&serde_norway::Value::Null), None);
    }

    #[test]
    fn test_value_as_u32() {
        let num = serde_norway::to_value(42u32).unwrap();
        assert_eq!(value_as_u32(&num), Some(42));

        assert_eq!(
            value_as_u32(&serde_norway::Value::String("7".into())),
            Some(7)
        );
        assert_eq!(value_as_u32(&serde_norway::Value::Null), None);
    }
}
