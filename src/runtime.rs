//! Runtime: resolving and running scripts (spec section 6).
//!
//! `resolve_script` turns a script name into exactly one executable file. Validation (V12)
//! and the executor both call it, so a machine that passes `decree check` runs the same files.
//! `Executor` runs scripts for one run: environment, log, process group, timeout, attempts,
//! event parsing, and one `script` event in `events.jsonl` per execution (section 7).

use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{SecondsFormat, Utc};
use serde_json::{json, Map, Value};

use crate::cond::is_ident;
use crate::machine::{DataSpec, LoadedMachine};

/// Directory holding scripts, relative to `.decree/` or `shared_source`.
const SCRIPTS_DIR: &str = "scripts";

/// Why a script name does not resolve to exactly one executable file (V12).
#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    #[error("script name `{0}` does not match ^[a-z][a-z0-9_]*$")]
    InvalidName(String),

    #[error("script `{name}` not found; searched {}", join_paths(searched))]
    Missing {
        name: String,
        searched: Vec<PathBuf>,
    },

    #[error("script `{name}` is ambiguous: {}", join_paths(matches))]
    Ambiguous { name: String, matches: Vec<PathBuf> },

    #[error("script `{name}`: {} is not a regular file", path.display())]
    NotRegularFile { name: String, path: PathBuf },

    #[error("script `{name}`: {} is not executable", path.display())]
    NotExecutable { name: String, path: PathBuf },

    #[error("script `{name}`: cannot read {}: {source}", dir.display())]
    Io {
        name: String,
        dir: PathBuf,
        source: io::Error,
    },
}

fn join_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The directories script `name` is looked up in for `machine`, in order:
/// `scripts/<machine>/`, `scripts/`, then the same two under `shared_source`.
pub fn search_dirs(decree_dir: &Path, shared_source: Option<&Path>, machine: &str) -> Vec<PathBuf> {
    std::iter::once(decree_dir)
        .chain(shared_source)
        .flat_map(|base| {
            let scripts = base.join(SCRIPTS_DIR);
            [scripts.join(machine), scripts]
        })
        .collect()
}

/// Resolve script `name` used by `machine` (section 6, Resolution). The first directory from
/// `search_dirs` holding a match wins; a match is a file named `name` or `name.<ext>` with one
/// extension. The winner must be the only match in its directory, a regular file, and have
/// an execute bit set. Lower directories are not read once a match is found.
pub fn resolve_script(
    decree_dir: &Path,
    shared_source: Option<&Path>,
    machine: &str,
    name: &str,
) -> Result<PathBuf, ScriptError> {
    if !is_ident(name) {
        return Err(ScriptError::InvalidName(name.to_string()));
    }
    let searched = search_dirs(decree_dir, shared_source, machine);
    for dir in &searched {
        let mut matches = matches_in(dir, name).map_err(|source| ScriptError::Io {
            name: name.to_string(),
            dir: dir.clone(),
            source,
        })?;
        match matches.len() {
            0 => continue,
            1 => return check_executable(name, matches.remove(0)),
            _ => {
                return Err(ScriptError::Ambiguous {
                    name: name.to_string(),
                    matches,
                })
            }
        }
    }
    Err(ScriptError::Missing {
        name: name.to_string(),
        searched,
    })
}

/// Entries of `dir` named `name` or `name.<ext>`, sorted. Directories are skipped: they are
/// per-machine script directories, never scripts. A missing `dir` holds no matches.
fn matches_in(dir: &Path, name: &str) -> io::Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut matches = Vec::new();
    for entry in entries {
        let path = entry?.path();
        let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if is_match(file_name, name) && !path.is_dir() {
            matches.push(path);
        }
    }
    matches.sort();
    Ok(matches)
}

/// `file_name` is `name`, or `name.<ext>` where `<ext>` is non-empty and has no dot.
fn is_match(file_name: &str, name: &str) -> bool {
    match file_name.strip_prefix(name) {
        Some("") => true,
        Some(rest) => rest
            .strip_prefix('.')
            .is_some_and(|ext| !ext.is_empty() && !ext.contains('.')),
        None => false,
    }
}

/// The single match must be a regular file (symlinks followed) with `mode & 0o111` non-zero.
fn check_executable(name: &str, path: PathBuf) -> Result<PathBuf, ScriptError> {
    let meta = match fs::metadata(&path) {
        Ok(meta) if meta.is_file() => meta,
        _ => {
            return Err(ScriptError::NotRegularFile {
                name: name.to_string(),
                path,
            })
        }
    };
    if meta.permissions().mode() & 0o111 == 0 {
        return Err(ScriptError::NotExecutable {
            name: name.to_string(),
            path,
        });
    }
    Ok(path)
}

/// `DECREE_STATE`, and the state in log names, for root `onentry` and `onexit` scripts.
pub const ROOT_STATE: &str = "_root";

/// The run's event log, in the run directory (section 7).
pub const EVENTS_FILE: &str = "events.jsonl";

/// The claimed message, in the run directory (section 3).
pub const MESSAGE_FILE: &str = "message.md";

/// Folder in the run directory that delivered replies are moved into (section 4).
pub const RECEIVED_DIR: &str = "received";

/// `events.jsonl` schema version (section 7).
const EVENTS_VERSION: u64 = 1;

/// stdout lines of an invoke kept in memory for the router prompt.
pub const STDOUT_TAIL_LINES: usize = 50;

/// How long a stopped script's process group gets between SIGTERM and SIGKILL.
pub const KILL_GRACE: Duration = Duration::from_secs(10);

/// How often a running script is checked for exit, timeout and signals.
const POLL: Duration = Duration::from_millis(10);

/// Prefix of stderr lines in a script log, with one trailing space.
const STDERR_PREFIX: &[u8] = b"[stderr] ";

/// Events an invoke may not print (section 5, Rules).
pub fn is_reserved_event(event: &str) -> bool {
    matches!(event, "done" | "error") || event.starts_with("done.") || event.starts_with("error.")
}

/// When a script runs: SCXML `<onentry>`, `<invoke>` or `<onexit>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    OnEntry,
    Invoke,
    OnExit,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::OnEntry => "onentry",
            Phase::Invoke => "invoke",
            Phase::OnExit => "onexit",
        }
    }
}

/// Why a script could not be run to completion.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Script(#[from] ScriptError),

    #[error("script `{script}`: cannot run {}: {source}", path.display())]
    Spawn {
        script: String,
        path: PathBuf,
        source: io::Error,
    },

    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },

    /// SIGINT or SIGTERM arrived while the script ran. It was stopped and has no `script`
    /// event; the caller interrupts the run (section 4, Stopping).
    #[error("interrupted by a signal while script `{script}` was running")]
    Interrupted { script: String },
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> RuntimeError + '_ {
    move |source| RuntimeError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Set `flag` on SIGINT and SIGTERM instead of terminating (section 6, Signals). The
/// executor polls the flag and stops the running script's process group.
pub fn register_signals(flag: &Arc<AtomicBool>) -> io::Result<()> {
    for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        signal_hook::flag::register(signal, Arc::clone(flag))?;
    }
    Ok(())
}

/// RFC 3339 UTC with milliseconds, as every timestamp in `events.jsonl` is written.
pub fn timestamp(t: chrono::DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// `runs/<id>/events.jsonl`: one JSON object per line, each appended with a single write
/// to a file opened with `O_APPEND` (section 7).
#[derive(Debug)]
pub struct EventLog {
    file: File,
    next_seq: u64,
    run_id: String,
    machine: String,
    trigger: String,
}

impl EventLog {
    /// Open or create the log in `run_dir`. `seq` continues after the lines already in it.
    pub fn open(run_dir: &Path, run_id: &str, machine: &str, trigger: &str) -> io::Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(run_dir.join(EVENTS_FILE))?;
        let mut existing = Vec::new();
        file.read_to_end(&mut existing)?;
        let lines = existing
            .split(|&b| b == b'\n')
            .filter(|line| !line.trim_ascii().is_empty())
            .count();
        Ok(EventLog {
            file,
            next_seq: lines as u64 + 1,
            run_id: run_id.to_string(),
            machine: machine.to_string(),
            trigger: trigger.to_string(),
        })
    }

    /// Append one event of type `kind`: the fields every event carries, plus `fields`.
    /// Returns its `seq`.
    pub fn append(&mut self, kind: &str, fields: Map<String, Value>) -> io::Result<u64> {
        let seq = self.next_seq;
        let mut event = Map::new();
        event.insert("v".into(), json!(EVENTS_VERSION));
        event.insert("seq".into(), json!(seq));
        event.insert("ts".into(), json!(timestamp(Utc::now())));
        event.insert("type".into(), json!(kind));
        event.insert("run_id".into(), json!(self.run_id));
        event.insert("machine".into(), json!(self.machine));
        event.insert("trigger".into(), json!(self.trigger));
        event.extend(fields);
        let mut line = serde_json::to_vec(&Value::Object(event)).map_err(io::Error::other)?;
        line.push(b'\n');
        self.file.write_all(&line)?;
        self.next_seq += 1;
        Ok(seq)
    }
}

/// `DECREE_DATA_<NAME>` for each `data` entry: the message's `params` value, else the
/// default. Ints as decimal, bools as `true` or `false` (section 6, Environment).
pub fn data_env(
    data: &BTreeMap<String, DataSpec>,
    params: &serde_norway::Mapping,
) -> Vec<(String, String)> {
    data.iter()
        .map(|(name, spec)| {
            let value = params.get(name.as_str()).unwrap_or(&spec.default);
            let text = match value {
                serde_norway::Value::String(s) => s.clone(),
                serde_norway::Value::Bool(b) => b.to_string(),
                serde_norway::Value::Number(n) => n.to_string(),
                other => serde_norway::to_string(other)
                    .unwrap_or_default()
                    .trim_end()
                    .to_string(),
            };
            (format!("DECREE_DATA_{}", name.to_uppercase()), text)
        })
        .collect()
}

/// The run an `Executor` runs scripts for.
#[derive(Debug, Clone)]
pub struct RunInfo {
    /// Absolute path of the directory containing `.decree/`.
    pub project_root: PathBuf,
    pub shared_source: Option<PathBuf>,
    /// Absolute path of `runs/<id>/`, which must exist.
    pub run_dir: PathBuf,
    pub run_id: String,
    pub machine: String,
    pub trigger: String,
    /// `DECREE_DATA_*` variables, from `data_env`.
    pub data: Vec<(String, String)>,
    /// Config `max_attempts`, for states that do not set their own.
    pub max_attempts: u32,
    /// Config `max_log_size`; 0 disables truncation.
    pub max_log_size: u64,
}

/// One script execution: what to run and the per-execution `DECREE_*` values.
#[derive(Debug, Clone)]
pub struct ScriptRun<'a> {
    pub script: &'a str,
    /// The state the script runs for, or `ROOT_STATE`.
    pub state: &'a str,
    pub phase: Phase,
    pub visits: u32,
    pub attempt: u32,
    pub max_attempts: u32,
    /// Set for `onentry` scripts of a waiting state, empty otherwise.
    pub wait_id: &'a str,
    pub accepts: &'a [String],
    pub timeout: Option<Duration>,
}

impl<'a> ScriptRun<'a> {
    /// `script` for `state` in `phase`: attempt 1 of 1, no visits, no wait, no timeout.
    pub fn new(script: &'a str, state: &'a str, phase: Phase) -> Self {
        ScriptRun {
            script,
            state,
            phase,
            visits: 0,
            attempt: 1,
            max_attempts: 1,
            wait_id: "",
            accepts: &[],
            timeout: None,
        }
    }
}

/// A finished script execution.
#[derive(Debug, Clone)]
pub struct Execution {
    /// `None` if the script was killed by a signal.
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    /// The last `STDOUT_TAIL_LINES` stdout lines, without line endings.
    pub stdout_tail: Vec<String>,
    /// The last non-empty stdout line, trimmed.
    pub last_line: Option<String>,
    /// Log filename in the run folder.
    pub log: String,
    /// The file that ran.
    pub path: PathBuf,
}

impl Execution {
    /// Exited 0 within its timeout. A timed-out script counts as a non-zero exit.
    pub fn succeeded(&self) -> bool {
        self.exit_code == Some(0) && !self.timed_out
    }

    /// The `event` string of a JSON object on the last non-empty stdout line.
    pub fn printed_event(&self) -> Option<String> {
        let value: Value = serde_json::from_str(self.last_line.as_deref()?).ok()?;
        value
            .as_object()?
            .get("event")?
            .as_str()
            .map(str::to_string)
    }
}

/// The event an invoke raises (section 6, Events from an invoke).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvokeEvent {
    /// From the exit code: `done` (exit 0, no printed event) or `error`.
    ExitCode(&'static str),
    /// Printed on the last stdout line of an invoke that exited 0.
    Stdout(String),
    /// Printed, but reserved or matching no transition of the state or its ancestors:
    /// the event is `error`, and the `transition` event records `invalid_event`.
    Invalid(String),
    /// A router state's invoke exited 0 without printing an event: the router decides.
    Router,
}

impl InvokeEvent {
    /// The event name, or `None` when the router decides.
    pub fn event(&self) -> Option<&str> {
        match self {
            InvokeEvent::ExitCode(e) => Some(e),
            InvokeEvent::Stdout(e) => Some(e),
            InvokeEvent::Invalid(_) => Some("error"),
            InvokeEvent::Router => None,
        }
    }
}

/// The result of a state's invoke, after its attempts.
#[derive(Debug, Clone)]
pub struct InvokeOutcome {
    pub event: InvokeEvent,
    /// Attempts made, from 1.
    pub attempts: u32,
    /// The last attempt's execution.
    pub execution: Execution,
}

/// Runs the scripts of one run (section 6, Execution) and appends their `script` events.
#[derive(Debug)]
pub struct Executor {
    info: RunInfo,
    events: EventLog,
    shutdown: Arc<AtomicBool>,
    next_log: u32,
    /// `DECREE_RECEIVED`: the last reply this run received.
    pub received: Option<PathBuf>,
}

impl Executor {
    /// Open the run's event log. Log numbers continue after the logs already in the run
    /// folder. A set `shutdown` flag (see `register_signals`) stops the running script.
    pub fn open(info: RunInfo, shutdown: Arc<AtomicBool>) -> Result<Self, RuntimeError> {
        let events = EventLog::open(&info.run_dir, &info.run_id, &info.machine, &info.trigger)
            .map_err(io_err(&info.run_dir.join(EVENTS_FILE)))?;
        let next_log = next_log_number(&info.run_dir).map_err(io_err(&info.run_dir))?;
        Ok(Executor {
            info,
            events,
            shutdown,
            next_log,
            received: None,
        })
    }

    pub fn info(&self) -> &RunInfo {
        &self.info
    }

    /// The run's event log, for the events the interpreter appends.
    pub fn events(&mut self) -> &mut EventLog {
        &mut self.events
    }

    /// Attempts allowed for `state`: its `max_attempts`, else config `max_attempts`.
    pub fn max_attempts(&self, machine: &LoadedMachine, state: usize) -> u32 {
        machine.nodes[state]
            .max_attempts
            .unwrap_or(self.info.max_attempts)
            .max(1)
    }

    /// Run `state`'s invoke, re-running it in place while it fails and attempts remain,
    /// and pick its event. Each failed attempt but the last appends a `transition` event
    /// with `source: "attempt"`. `None` if the state has no invoke.
    pub fn run_invoke(
        &mut self,
        machine: &LoadedMachine,
        state: usize,
        visits: u32,
    ) -> Result<Option<InvokeOutcome>, RuntimeError> {
        let node = &machine.nodes[state];
        let Some(script) = node.invoke.as_deref() else {
            return Ok(None);
        };
        let max_attempts = self.max_attempts(machine, state);
        let mut attempt = 1;
        let execution = loop {
            let execution = self.run_script(&ScriptRun {
                visits,
                attempt,
                max_attempts,
                timeout: node.timeout_s.map(Duration::from_secs),
                ..ScriptRun::new(script, &node.id, Phase::Invoke)
            })?;
            if execution.succeeded() || attempt >= max_attempts {
                break execution;
            }
            self.append_attempt(&node.id, execution.exit_code)?;
            attempt += 1;
        };
        let event = if !execution.succeeded() {
            InvokeEvent::ExitCode("error")
        } else {
            match execution.printed_event() {
                Some(e) if is_reserved_event(&e) || !machine.handles(state, &e) => {
                    InvokeEvent::Invalid(e)
                }
                Some(e) => InvokeEvent::Stdout(e),
                None if node.router.is_some() => InvokeEvent::Router,
                None => InvokeEvent::ExitCode("done"),
            }
        };
        Ok(Some(InvokeOutcome {
            event,
            attempts: attempt,
            execution,
        }))
    }

    /// A failed attempt that will be re-run: `error`, with `from` and `to` equal.
    fn append_attempt(&mut self, state: &str, exit_code: Option<i32>) -> Result<(), RuntimeError> {
        let fields = json!({
            "from": state,
            "event": "error",
            "to": state,
            "source": "attempt",
            "exit_code": exit_code,
        });
        self.append(
            "transition",
            fields.as_object().cloned().unwrap_or_default(),
        )
    }

    fn append(&mut self, kind: &str, fields: Map<String, Value>) -> Result<(), RuntimeError> {
        let path = self.info.run_dir.join(EVENTS_FILE);
        self.events
            .append(kind, fields)
            .map(drop)
            .map_err(io_err(&path))
    }

    /// The next `NNNN-<state>-<name>.log` filename in the run folder. Script logs and the
    /// logs decree writes itself (`_router`) share the counter, so the folder reads in order.
    pub fn reserve_log(&mut self, state: &str, name: &str) -> String {
        let log = format!("{:04}-{state}-{name}.log", self.next_log);
        self.next_log += 1;
        log
    }

    /// Resolve and run one script, log its output, and append its `script` event. On
    /// SIGINT or SIGTERM the script's process group is stopped, no event is written, and
    /// the result is `RuntimeError::Interrupted`.
    pub fn run_script(&mut self, run: &ScriptRun) -> Result<Execution, RuntimeError> {
        let decree_dir = self.info.project_root.join(crate::config::DECREE_DIR);
        let path = resolve_script(
            &decree_dir,
            self.info.shared_source.as_deref(),
            &self.info.machine,
            run.script,
        )?;
        let interrupted = || RuntimeError::Interrupted {
            script: run.script.to_string(),
        };
        if self.shutdown.load(Ordering::SeqCst) {
            return Err(interrupted());
        }

        let log = self.reserve_log(run.state, run.script);
        let log_path = self.info.run_dir.join(&log);
        let log_file = File::create(&log_path).map_err(io_err(&log_path))?;

        let mut cmd = Command::new(&path);
        cmd.current_dir(&self.info.project_root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .envs(self.env(run))
            // Reused from 0.4.2: its own process group, so a stop reaches the whole tree.
            .process_group(0);
        // Also from 0.4.2: the background group must not be stopped for touching the TTY.
        // SAFETY: signal(2) is async-signal-safe, and the closure allocates nothing.
        unsafe {
            cmd.pre_exec(|| {
                libc::signal(libc::SIGTTIN, libc::SIG_IGN);
                libc::signal(libc::SIGTTOU, libc::SIG_IGN);
                Ok(())
            });
        }

        let started_at = Utc::now();
        let start = Instant::now();
        let mut child = cmd.spawn().map_err(|source| RuntimeError::Spawn {
            script: run.script.to_string(),
            path: path.clone(),
            source,
        })?;
        let log_file = Arc::new(Mutex::new(log_file));
        let stdout = child
            .stdout
            .take()
            .map(|out| spawn_reader(out, Arc::clone(&log_file), b"", STDOUT_TAIL_LINES));
        let stderr = child
            .stderr
            .take()
            .map(|err| spawn_reader(err, Arc::clone(&log_file), STDERR_PREFIX, 0));

        let deadline = run.timeout.map(|t| start + t);
        let (status, stop) =
            wait_child(&mut child, deadline, &self.shutdown).map_err(io_err(&path))?;
        // The readers end once every process holding the pipes has exited.
        let tail = join_reader(stdout).map_err(io_err(&log_path))?;
        join_reader(stderr).map_err(io_err(&log_path))?;
        let duration = start.elapsed();
        truncate_log_if_needed(&log_path, self.info.max_log_size).map_err(io_err(&log_path))?;
        if stop == Stop::Signal {
            return Err(interrupted());
        }

        let execution = Execution {
            exit_code: status.code(),
            timed_out: stop == Stop::Timeout,
            stdout_tail: tail.lines.into(),
            last_line: tail.last,
            log,
            path,
        };
        let mut fields = Map::new();
        fields.insert("state".into(), json!(run.state));
        fields.insert("phase".into(), json!(run.phase.as_str()));
        fields.insert("script".into(), json!(run.script));
        fields.insert("path".into(), json!(self.display_path(&execution.path)));
        fields.insert("attempt".into(), json!(run.attempt));
        fields.insert("started_at".into(), json!(timestamp(started_at)));
        fields.insert("duration_ms".into(), json!(duration.as_millis() as u64));
        fields.insert("exit_code".into(), json!(execution.exit_code));
        if execution.timed_out {
            fields.insert("timed_out".into(), json!(true));
        }
        fields.insert("log".into(), json!(execution.log));
        self.append("script", fields)?;
        Ok(execution)
    }

    /// The section 6 environment for `run`, added to the inherited one.
    fn env(&self, run: &ScriptRun) -> Vec<(String, std::ffi::OsString)> {
        let info = &self.info;
        let mut accepts = run.accepts.to_vec();
        accepts.sort();
        let received = self.received.as_deref().unwrap_or(Path::new(""));
        let mut vars: Vec<(String, std::ffi::OsString)> = [
            ("PROJECT_ROOT", info.project_root.as_os_str().into()),
            ("MESSAGE", info.run_dir.join(MESSAGE_FILE).into()),
            ("MESSAGE_ID", info.run_id.clone().into()),
            ("MACHINE", info.machine.clone().into()),
            ("STATE", run.state.into()),
            ("PHASE", run.phase.as_str().into()),
            ("VISITS", run.visits.to_string().into()),
            ("RUN_DIR", info.run_dir.as_os_str().into()),
            ("ATTEMPT", run.attempt.to_string().into()),
            ("MAX_ATTEMPTS", run.max_attempts.to_string().into()),
            (
                "FINAL_ATTEMPT",
                (run.attempt == run.max_attempts).to_string().into(),
            ),
            ("TRIGGER", info.trigger.clone().into()),
            ("WAIT_ID", run.wait_id.into()),
            ("ACCEPTS", accepts.join(" ").into()),
            ("RECEIVED", received.as_os_str().into()),
        ]
        .into_iter()
        .map(|(name, value)| (format!("DECREE_{name}"), value))
        .collect();
        vars.extend(
            info.data
                .iter()
                .map(|(name, value)| (name.clone(), value.into())),
        );
        vars
    }

    /// A script path as the `script` event records it: relative to the project root, or
    /// `shared:` plus the path relative to `shared_source`.
    fn display_path(&self, path: &Path) -> String {
        let decree_dir = self.info.project_root.join(crate::config::DECREE_DIR);
        if path.starts_with(&decree_dir) {
            if let Ok(rel) = path.strip_prefix(&self.info.project_root) {
                return rel.display().to_string();
            }
        }
        if let Some(rel) = self
            .info
            .shared_source
            .as_deref()
            .and_then(|shared| path.strip_prefix(shared).ok())
        {
            return format!("shared:{}", rel.display());
        }
        path.display().to_string()
    }
}

/// One more than the highest `NNNN-` prefix of a `.log` file in `run_dir`.
fn next_log_number(run_dir: &Path) -> io::Result<u32> {
    let mut highest = 0;
    for entry in fs::read_dir(run_dir)? {
        let name = entry?.file_name();
        let Some(name) = name.to_str().filter(|n| n.ends_with(".log")) else {
            continue;
        };
        let number = name
            .split_once('-')
            .map(|(n, _)| n)
            .filter(|n| n.len() >= 4 && n.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|n| n.parse::<u32>().ok());
        highest = highest.max(number.unwrap_or(0));
    }
    Ok(highest + 1)
}

/// What a stream reader keeps: the last lines and the last non-empty line.
#[derive(Debug, Default)]
struct Tail {
    lines: VecDeque<String>,
    last: Option<String>,
}

/// Copy `source` into `log` line by line, each line prefixed with `prefix`, keeping the
/// last `keep` lines.
fn spawn_reader<R: Read + Send + 'static>(
    source: R,
    log: Arc<Mutex<File>>,
    prefix: &'static [u8],
    keep: usize,
) -> thread::JoinHandle<io::Result<Tail>> {
    thread::spawn(move || {
        let mut reader = BufReader::new(source);
        let mut tail = Tail::default();
        let mut buf = Vec::new();
        loop {
            buf.clear();
            if reader.read_until(b'\n', &mut buf)? == 0 {
                return Ok(tail);
            }
            let mut line = Vec::with_capacity(prefix.len() + buf.len() + 1);
            line.extend_from_slice(prefix);
            line.extend_from_slice(&buf);
            if !buf.ends_with(b"\n") {
                line.push(b'\n');
            }
            log.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .write_all(&line)?;
            if keep == 0 {
                continue;
            }
            let text = String::from_utf8_lossy(&buf);
            let text = text.trim_end_matches(['\n', '\r']);
            if !text.trim().is_empty() {
                tail.last = Some(text.trim().to_string());
            }
            if tail.lines.len() == keep {
                tail.lines.pop_front();
            }
            tail.lines.push_back(text.to_string());
        }
    })
}

fn join_reader(handle: Option<thread::JoinHandle<io::Result<Tail>>>) -> io::Result<Tail> {
    match handle {
        Some(handle) => handle
            .join()
            .map_err(|_| io::Error::other("log reader panicked"))?,
        None => Ok(Tail::default()),
    }
}

/// Why a script stopped running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    Exited,
    Timeout,
    Signal,
}

/// Wait for `child`, stopping its group at `deadline` or when `shutdown` is set.
fn wait_child(
    child: &mut Child,
    deadline: Option<Instant>,
    shutdown: &AtomicBool,
) -> io::Result<(ExitStatus, Stop)> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok((status, Stop::Exited));
        }
        let stop = if shutdown.load(Ordering::SeqCst) {
            Stop::Signal
        } else if deadline.is_some_and(|d| Instant::now() >= d) {
            Stop::Timeout
        } else {
            thread::sleep(POLL);
            continue;
        };
        return Ok((stop_group(child)?, stop));
    }
}

/// Send SIGTERM to the process group `child` leads, wait up to `KILL_GRACE` for every
/// process in it to exit, then send SIGKILL (section 6, Execution). 0.4.2 sent SIGTERM
/// and waited forever (`docs/0.5-inventory.md`, C3).
fn stop_group(child: &mut Child) -> io::Result<ExitStatus> {
    let pgid = child.id() as libc::pid_t;
    // SAFETY: kill(2) only sends a signal; a negative pid addresses the child's group.
    unsafe { libc::kill(-pgid, libc::SIGTERM) };
    let deadline = Instant::now() + KILL_GRACE;
    let mut status = None;
    while Instant::now() < deadline {
        if status.is_none() {
            status = child.try_wait()?;
        }
        // SAFETY: signal 0 only checks whether any process in the group exists.
        if status.is_some() && unsafe { libc::kill(-pgid, 0) } != 0 {
            break;
        }
        thread::sleep(POLL);
    }
    // SAFETY: as above. Harmless if the group is already gone.
    unsafe { libc::kill(-pgid, libc::SIGKILL) };
    match status {
        Some(status) => Ok(status),
        None => child.wait(),
    }
}

/// Keep only the last `max_size` bytes of the log at `path`, behind a marker line; 0
/// disables truncation (section 6, Logs). Moved here unchanged from 0.4.2.
pub fn truncate_log_if_needed(path: &Path, max_size: u64) -> io::Result<()> {
    if max_size == 0 {
        return Ok(());
    }

    let metadata = fs::metadata(path)?;
    if metadata.len() <= max_size {
        return Ok(());
    }

    let content = fs::read(path)?;
    let skip = content.len() - max_size as usize;
    let truncated = &content[skip..];

    let marker = format!(
        "[log truncated — showing last {} of output]\n",
        format_bytes(max_size)
    );
    let mut new_content = marker.into_bytes();
    new_content.extend_from_slice(truncated);

    fs::write(path, &new_content)
}

/// Format a byte count as human-readable.
fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{}MB", bytes / 1_048_576)
    } else if bytes >= 1024 {
        format!("{}KB", bytes / 1024)
    } else {
        format!("{bytes}B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A temp directory with `project/.decree/` and `shared/`, both empty.
    struct Fixture {
        tmp: TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let tmp = TempDir::new().unwrap();
            fs::create_dir_all(tmp.path().join("project/.decree")).unwrap();
            fs::create_dir_all(tmp.path().join("shared")).unwrap();
            Fixture { tmp }
        }

        fn decree_dir(&self) -> PathBuf {
            self.tmp.path().join("project/.decree")
        }

        fn shared(&self) -> PathBuf {
            self.tmp.path().join("shared")
        }

        /// Write a script at `rel` (relative to the temp root) with the given mode.
        fn script(&self, rel: &str, mode: u32) -> PathBuf {
            let path = self.tmp.path().join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "#!/usr/bin/env bash\nexit 0\n").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            path
        }

        fn resolve(&self, machine: &str, name: &str) -> Result<PathBuf, ScriptError> {
            resolve_script(&self.decree_dir(), Some(&self.shared()), machine, name)
        }
    }

    #[test]
    fn exact_name_resolves() {
        let f = Fixture::new();
        let x = f.script("project/.decree/scripts/x", 0o755);
        assert_eq!(f.resolve("m", "x").unwrap(), x);
    }

    #[test]
    fn one_extension_resolves() {
        let f = Fixture::new();
        let x = f.script("project/.decree/scripts/x.sh", 0o755);
        assert_eq!(f.resolve("m", "x").unwrap(), x);
    }

    #[test]
    fn machine_dir_overrides_flat_dir_for_that_machine_only() {
        let f = Fixture::new();
        let own = f.script("project/.decree/scripts/m/x.sh", 0o755);
        let flat = f.script("project/.decree/scripts/x.sh", 0o755);
        assert_eq!(f.resolve("m", "x").unwrap(), own);
        assert_eq!(f.resolve("n", "x").unwrap(), flat);
    }

    #[test]
    fn project_overrides_shared() {
        let f = Fixture::new();
        let shared = f.script("shared/scripts/x.sh", 0o755);
        assert_eq!(f.resolve("m", "x").unwrap(), shared);
        let project = f.script("project/.decree/scripts/x.sh", 0o755);
        assert_eq!(f.resolve("m", "x").unwrap(), project);
    }

    #[test]
    fn full_precedence_order() {
        let f = Fixture::new();
        let paths = [
            f.script("shared/scripts/x", 0o755),
            f.script("shared/scripts/m/x", 0o755),
            f.script("project/.decree/scripts/x", 0o755),
            f.script("project/.decree/scripts/m/x", 0o755),
        ];
        // Remove the winner each time; the next directory down must win.
        for winner in paths.iter().rev() {
            assert_eq!(&f.resolve("m", "x").unwrap(), winner);
            fs::remove_file(winner).unwrap();
        }
        assert!(matches!(
            f.resolve("m", "x"),
            Err(ScriptError::Missing { .. })
        ));
    }

    #[test]
    fn two_matches_in_one_dir_fail_naming_both() {
        let f = Fixture::new();
        let sh = f.script("project/.decree/scripts/x.sh", 0o755);
        let py = f.script("project/.decree/scripts/x.py", 0o755);
        let err = f.resolve("m", "x").unwrap_err();
        match &err {
            ScriptError::Ambiguous { matches, .. } => assert_eq!(matches, &vec![py, sh]),
            other => panic!("expected Ambiguous, got {other:?}"),
        }
        let msg = err.to_string();
        assert!(msg.contains("x.sh") && msg.contains("x.py"), "{msg}");
    }

    #[test]
    fn higher_match_hides_lower_ambiguous_pair() {
        let f = Fixture::new();
        f.script("shared/scripts/x.sh", 0o755);
        f.script("shared/scripts/x.py", 0o755);
        let project = f.script("project/.decree/scripts/x", 0o755);
        assert_eq!(f.resolve("m", "x").unwrap(), project);
    }

    #[test]
    fn not_executable_fails() {
        let f = Fixture::new();
        let x = f.script("project/.decree/scripts/x.sh", 0o644);
        match f.resolve("m", "x").unwrap_err() {
            ScriptError::NotExecutable { path, .. } => assert_eq!(path, x),
            other => panic!("expected NotExecutable, got {other:?}"),
        }
    }

    #[test]
    fn any_execute_bit_is_enough() {
        let f = Fixture::new();
        let x = f.script("project/.decree/scripts/x", 0o640 | 0o001);
        assert_eq!(f.resolve("m", "x").unwrap(), x);
    }

    #[test]
    fn non_executable_winner_does_not_fall_through() {
        let f = Fixture::new();
        f.script("shared/scripts/x.sh", 0o755);
        f.script("project/.decree/scripts/x.sh", 0o644);
        assert!(matches!(
            f.resolve("m", "x"),
            Err(ScriptError::NotExecutable { .. })
        ));
    }

    #[test]
    fn missing_fails_listing_searched_dirs() {
        let f = Fixture::new();
        f.script("project/.decree/scripts/y.sh", 0o755);
        let err = f.resolve("m", "x").unwrap_err();
        match &err {
            ScriptError::Missing { searched, .. } => {
                assert_eq!(
                    searched,
                    &search_dirs(&f.decree_dir(), Some(&f.shared()), "m")
                );
                assert_eq!(searched.len(), 4);
            }
            other => panic!("expected Missing, got {other:?}"),
        }
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn missing_without_shared_source_searches_two_dirs() {
        let f = Fixture::new();
        match resolve_script(&f.decree_dir(), None, "m", "x").unwrap_err() {
            ScriptError::Missing { searched, .. } => assert_eq!(searched.len(), 2),
            other => panic!("expected Missing, got {other:?}"),
        }
    }

    #[test]
    fn similar_names_do_not_match() {
        let f = Fixture::new();
        f.script("project/.decree/scripts/xy.sh", 0o755);
        f.script("project/.decree/scripts/x.tar.gz", 0o755);
        f.script("project/.decree/scripts/x.", 0o755);
        f.script("project/.decree/scripts/.x", 0o755);
        assert!(matches!(
            f.resolve("m", "x"),
            Err(ScriptError::Missing { .. })
        ));
    }

    #[test]
    fn machine_dir_named_like_script_is_not_a_match() {
        let f = Fixture::new();
        // `scripts/x/` is machine x's directory, not script x.
        f.script("project/.decree/scripts/x/other.sh", 0o755);
        let flat = f.script("project/.decree/scripts/x.sh", 0o755);
        assert_eq!(f.resolve("m", "x").unwrap(), flat);
    }

    #[test]
    fn non_regular_file_fails() {
        let f = Fixture::new();
        let dir = f.decree_dir().join("scripts");
        fs::create_dir_all(&dir).unwrap();
        let link = dir.join("x");
        std::os::unix::fs::symlink(dir.join("nowhere"), &link).unwrap();
        match f.resolve("m", "x").unwrap_err() {
            ScriptError::NotRegularFile { path, .. } => assert_eq!(path, link),
            other => panic!("expected NotRegularFile, got {other:?}"),
        }
    }

    #[test]
    fn symlink_to_executable_resolves() {
        let f = Fixture::new();
        let target = f.script("elsewhere/real.sh", 0o755);
        let dir = f.decree_dir().join("scripts");
        fs::create_dir_all(&dir).unwrap();
        let link = dir.join("x.sh");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert_eq!(f.resolve("m", "x").unwrap(), link);
    }

    #[test]
    fn invalid_names_fail() {
        let f = Fixture::new();
        f.script("project/.decree/scripts/X.sh", 0o755);
        for name in ["", "X", "../x", "m/x", "x.sh", "1x", "x-y"] {
            assert!(
                matches!(f.resolve("m", name), Err(ScriptError::InvalidName(_))),
                "{name:?}"
            );
        }
    }
}

#[cfg(test)]
mod executor_tests {
    use super::*;
    use crate::machine::load_machine_text;
    use tempfile::TempDir;

    const RUN_ID: &str = "20261001T143005Z-3fa9c1";

    /// A temp project whose `.decree/scripts/` holds copies of `tests/fixtures/scripts/`
    /// fixtures, with one run folder, `.decree/runs/<RUN_ID>/`.
    struct Project {
        tmp: TempDir,
        shutdown: Arc<AtomicBool>,
    }

    impl Project {
        fn new(scripts: &[&str]) -> Self {
            let tmp = TempDir::new().unwrap();
            let project = Project {
                tmp,
                shutdown: Arc::new(AtomicBool::new(false)),
            };
            fs::create_dir_all(project.run_dir()).unwrap();
            let dir = project.root().join(".decree/scripts");
            fs::create_dir_all(&dir).unwrap();
            let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scripts");
            for name in scripts {
                let file = format!("{name}.sh");
                // fs::copy keeps the fixture's executable bit.
                fs::copy(fixtures.join(&file), dir.join(&file)).unwrap();
            }
            project
        }

        fn root(&self) -> PathBuf {
            self.tmp.path().to_path_buf()
        }

        fn run_dir(&self) -> PathBuf {
            self.root().join(".decree/runs").join(RUN_ID)
        }

        fn info(&self) -> RunInfo {
            RunInfo {
                project_root: self.root(),
                shared_source: None,
                run_dir: self.run_dir(),
                run_id: RUN_ID.to_string(),
                machine: "m".to_string(),
                trigger: "inbox".to_string(),
                data: Vec::new(),
                max_attempts: 3,
                max_log_size: 0,
            }
        }

        fn executor(&self) -> Executor {
            Executor::open(self.info(), Arc::clone(&self.shutdown)).unwrap()
        }

        fn events(&self) -> Vec<Map<String, Value>> {
            let text = fs::read_to_string(self.run_dir().join(EVENTS_FILE)).unwrap_or_default();
            text.lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .map(|v| v.as_object().unwrap().clone())
                .collect()
        }

        fn events_of(&self, kind: &str) -> Vec<Map<String, Value>> {
            self.events()
                .into_iter()
                .filter(|e| e["type"] == kind)
                .collect()
        }

        fn log(&self, name: &str) -> String {
            fs::read_to_string(self.run_dir().join(name)).unwrap()
        }

        /// The pid the `sleep_long` and `ignore_term` fixtures record.
        fn child_pid(&self) -> Option<libc::pid_t> {
            fs::read_to_string(self.run_dir().join("child.pid"))
                .ok()?
                .trim()
                .parse()
                .ok()
        }
    }

    /// Machine `m` whose state `s` is `state` (YAML flow mapping), plus `done` and `failed`.
    fn machine(state: &str) -> LoadedMachine {
        let text = format!(
            "name: m\ndescription: Test.\ninitial: s\nstates:\n  s: {state}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        load_machine_text("m", Path::new("m.yml"), &text).unwrap()
    }

    fn invoke(project: &Project, state: &str) -> InvokeOutcome {
        let m = machine(state);
        let s = m.find("s").unwrap();
        project.executor().run_invoke(&m, s, 1).unwrap().unwrap()
    }

    /// Whether `pid` is a live process: it exists and is not a zombie.
    fn alive(pid: libc::pid_t) -> bool {
        // SAFETY: signal 0 only checks that the process exists.
        if unsafe { libc::kill(pid, 0) } != 0 {
            return false;
        }
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        let state = stat.rsplit_once(") ").map(|(_, rest)| rest.chars().next());
        state != Some(Some('Z'))
    }

    fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if done() {
                return true;
            }
            thread::sleep(Duration::from_millis(20));
        }
        done()
    }

    #[test]
    fn exit_zero_gives_done() {
        let p = Project::new(&["exit_zero"]);
        let out = invoke(&p, "{ invoke: exit_zero, transitions: { done: done } }");
        assert_eq!(out.event, InvokeEvent::ExitCode("done"));
        assert_eq!(out.event.event(), Some("done"));
        assert_eq!(out.execution.exit_code, Some(0));
        assert_eq!(p.events_of("script").len(), 1);
    }

    #[test]
    fn exit_three_gives_error_when_attempts_run_out() {
        let p = Project::new(&["exit_three"]);
        let out = invoke(
            &p,
            "{ invoke: exit_three, max_attempts: 2, transitions: { done: done } }",
        );
        assert_eq!(out.event, InvokeEvent::ExitCode("error"));
        assert_eq!(out.attempts, 2);
        assert_eq!(out.execution.exit_code, Some(3));
        let scripts = p.events_of("script");
        assert_eq!(scripts.len(), 2);
        assert!(scripts.iter().all(|e| e["exit_code"] == 3));
        // One attempt line between the two executions, none after the last.
        let attempts = p.events_of("transition");
        assert_eq!(attempts.len(), 1);
        let a = &attempts[0];
        assert_eq!(
            (
                &a["from"],
                &a["event"],
                &a["to"],
                &a["source"],
                &a["exit_code"]
            ),
            (
                &json!("s"),
                &json!("error"),
                &json!("s"),
                &json!("attempt"),
                &json!(3)
            )
        );
        let types: Vec<_> = p.events().iter().map(|e| e["type"].clone()).collect();
        assert_eq!(
            types,
            [json!("script"), json!("transition"), json!("script")]
        );
    }

    #[test]
    fn json_last_line_gives_its_event() {
        let p = Project::new(&["print_pass"]);
        let out = invoke(
            &p,
            "{ invoke: print_pass, transitions: { done: done, pass: done } }",
        );
        assert_eq!(out.event, InvokeEvent::Stdout("pass".to_string()));
        assert_eq!(out.event.event(), Some("pass"));
    }

    #[test]
    fn json_last_line_then_exit_one_gives_error() {
        let p = Project::new(&["print_pass_exit_one"]);
        let out = invoke(
            &p,
            "{ invoke: print_pass_exit_one, max_attempts: 1, transitions: { done: done, pass: done } }",
        );
        assert_eq!(out.event, InvokeEvent::ExitCode("error"));
        assert_eq!(out.execution.exit_code, Some(1));
    }

    #[test]
    fn undeclared_printed_event_becomes_error() {
        let p = Project::new(&["print_undeclared"]);
        let out = invoke(
            &p,
            "{ invoke: print_undeclared, transitions: { done: done } }",
        );
        assert_eq!(out.event, InvokeEvent::Invalid("nope".to_string()));
        assert_eq!(out.event.event(), Some("error"));
    }

    #[test]
    fn reserved_printed_event_becomes_error() {
        let p = Project::new(&["print_reserved"]);
        let out = invoke(
            &p,
            "{ invoke: print_reserved, transitions: { done: done, error.custom: failed } }",
        );
        assert_eq!(out.event, InvokeEvent::Invalid("error.custom".to_string()));
        for event in ["done", "error", "done.state.x", "error.custom"] {
            assert!(is_reserved_event(event), "{event}");
        }
        for event in ["pass", "doner", "errors", "pass.done"] {
            assert!(!is_reserved_event(event), "{event}");
        }
    }

    #[test]
    fn printed_event_handled_by_an_ancestor_is_valid() {
        let p = Project::new(&["print_pass"]);
        let text = "name: m\ndescription: Test.\ninitial: w\nstates:\n  w:\n    initial: s\n    \
                    transitions: { pass: done }\n    states:\n      \
                    s: { invoke: print_pass, transitions: { done: done } }\n  \
                    done: { final: true }\n  failed: { final: true }\n";
        let m = load_machine_text("m", Path::new("m.yml"), text).unwrap();
        let out = p
            .executor()
            .run_invoke(&m, m.find("s").unwrap(), 1)
            .unwrap()
            .unwrap();
        assert_eq!(out.event, InvokeEvent::Stdout("pass".to_string()));
    }

    #[test]
    fn router_state_without_printed_event_leaves_the_choice_to_the_router() {
        let p = Project::new(&["exit_zero", "print_pass"]);
        let router = "{ router: llm, default: pass, description: D., transitions: { pass: done, ask: done } }";
        let out = invoke(&p, &router.replace("router:", "invoke: exit_zero, router:"));
        assert_eq!(out.event, InvokeEvent::Router);
        assert_eq!(out.event.event(), None);
        // A printed event skips the router.
        let out = invoke(
            &p,
            &router.replace("router:", "invoke: print_pass, router:"),
        );
        assert_eq!(out.event, InvokeEvent::Stdout("pass".to_string()));
    }

    #[test]
    fn state_without_invoke_runs_nothing() {
        let p = Project::new(&[]);
        let m = machine("{ transitions: { done: done } }");
        let out = p
            .executor()
            .run_invoke(&m, m.find("s").unwrap(), 1)
            .unwrap();
        assert!(out.is_none());
        assert!(p.events().is_empty());
    }

    #[test]
    fn attempts_rerun_the_invoke_in_place_until_it_succeeds() {
        let p = Project::new(&["fail_until_final"]);
        let out = invoke(
            &p,
            "{ invoke: fail_until_final, max_attempts: 3, transitions: { done: done } }",
        );
        assert_eq!(out.event, InvokeEvent::ExitCode("done"));
        assert_eq!(out.attempts, 3);
        let scripts = p.events_of("script");
        let attempts: Vec<_> = scripts.iter().map(|e| e["attempt"].clone()).collect();
        assert_eq!(attempts, [json!(1), json!(2), json!(3)]);
        let logs: Vec<_> = scripts.iter().map(|e| e["log"].clone()).collect();
        assert_eq!(
            logs,
            [
                json!("0001-s-fail_until_final.log"),
                json!("0002-s-fail_until_final.log"),
                json!("0003-s-fail_until_final.log")
            ]
        );
        assert_eq!(p.events_of("transition").len(), 2);
        assert_eq!(p.log("0003-s-fail_until_final.log"), "attempt 3 of 3\n");
    }

    #[test]
    fn config_max_attempts_applies_when_the_state_sets_none() {
        let p = Project::new(&["exit_three"]);
        let out = invoke(&p, "{ invoke: exit_three, transitions: { done: done } }");
        assert_eq!(out.attempts, 3);
        assert_eq!(p.events_of("script").len(), 3);
    }

    #[test]
    fn stderr_lines_carry_the_prefix() {
        let p = Project::new(&["stderr"]);
        let out = p
            .executor()
            .run_script(&ScriptRun::new("stderr", "s", Phase::OnEntry))
            .unwrap();
        let log = p.log(&out.log);
        assert!(log.contains("to stdout\n"), "{log}");
        assert!(log.contains("[stderr] to stderr\n"), "{log}");
        assert!(!log.contains("[stderr] to stdout"), "{log}");
        assert_eq!(out.stdout_tail, ["to stdout"]);
    }

    #[test]
    fn every_section_6_variable_is_set() {
        let p = Project::new(&["print_env"]);
        let mut info = p.info();
        let m = load_machine_text(
            "m",
            Path::new("m.yml"),
            "name: m\ndescription: T.\ndata:\n  max_rounds: { type: int, default: 2 }\n  \
             strict: { type: bool, default: false }\n  label: { type: string, default: x }\n\
             initial: s\nstates:\n  s: { transitions: { done: done } }\n  done: { final: true }\n",
        )
        .unwrap();
        let params: serde_norway::Mapping =
            serde_norway::from_str("label: release\nmax_rounds: 5").unwrap();
        info.data = data_env(&m.data, &params);
        let mut exec = Executor::open(info, Arc::clone(&p.shutdown)).unwrap();
        let received = p.run_dir().join("received/reply.md");
        exec.received = Some(received.clone());
        let accepts = ["retry".to_string(), "approve".to_string()];
        let out = exec
            .run_script(&ScriptRun {
                visits: 2,
                attempt: 1,
                max_attempts: 1,
                wait_id: "w-id",
                accepts: &accepts,
                ..ScriptRun::new("print_env", "review", Phase::OnEntry)
            })
            .unwrap();
        let log = p.log(&out.log);
        let vars: BTreeMap<&str, &str> = log
            .lines()
            .filter_map(|line| line.split_once('='))
            .collect();
        let root = p.root();
        let run_dir = p.run_dir();
        let expected = [
            ("DECREE_PROJECT_ROOT", root.to_str().unwrap().to_string()),
            (
                "DECREE_MESSAGE",
                run_dir.join("message.md").to_str().unwrap().to_string(),
            ),
            ("DECREE_MESSAGE_ID", RUN_ID.to_string()),
            ("DECREE_MACHINE", "m".to_string()),
            ("DECREE_STATE", "review".to_string()),
            ("DECREE_PHASE", "onentry".to_string()),
            ("DECREE_VISITS", "2".to_string()),
            ("DECREE_RUN_DIR", run_dir.to_str().unwrap().to_string()),
            ("DECREE_ATTEMPT", "1".to_string()),
            ("DECREE_MAX_ATTEMPTS", "1".to_string()),
            ("DECREE_FINAL_ATTEMPT", "true".to_string()),
            ("DECREE_TRIGGER", "inbox".to_string()),
            ("DECREE_WAIT_ID", "w-id".to_string()),
            ("DECREE_ACCEPTS", "approve retry".to_string()),
            ("DECREE_RECEIVED", received.to_str().unwrap().to_string()),
            ("DECREE_DATA_MAX_ROUNDS", "5".to_string()),
            ("DECREE_DATA_STRICT", "false".to_string()),
            ("DECREE_DATA_LABEL", "release".to_string()),
        ];
        for (name, value) in &expected {
            assert_eq!(vars.get(name), Some(&value.as_str()), "{name}\n{log}");
        }
    }

    #[test]
    fn root_and_invoke_variables_have_their_defaults() {
        let p = Project::new(&["print_env"]);
        let m = machine("{ invoke: print_env, max_attempts: 2, transitions: { done: done } }");
        let mut exec = p.executor();
        let out = exec
            .run_invoke(&m, m.find("s").unwrap(), 4)
            .unwrap()
            .unwrap();
        let log = p.log(&out.execution.log);
        for line in [
            "DECREE_PHASE=invoke",
            "DECREE_VISITS=4",
            "DECREE_ATTEMPT=1",
            "DECREE_MAX_ATTEMPTS=2",
            "DECREE_FINAL_ATTEMPT=false",
            "DECREE_WAIT_ID=",
            "DECREE_ACCEPTS=",
            "DECREE_RECEIVED=",
        ] {
            assert!(log.lines().any(|l| l == line), "{line}\n{log}");
        }
        let out = exec
            .run_script(&ScriptRun::new("print_env", ROOT_STATE, Phase::OnExit))
            .unwrap();
        assert_eq!(out.log, "0002-_root-print_env.log");
        let log = p.log(&out.log);
        assert!(log.lines().any(|l| l == "DECREE_STATE=_root"), "{log}");
        assert!(log.lines().any(|l| l == "DECREE_VISITS=0"), "{log}");
    }

    #[test]
    fn script_event_has_every_section_7_field() {
        let p = Project::new(&["sleep_one"]);
        let out = invoke(&p, "{ invoke: sleep_one, transitions: { done: done } }");
        assert_eq!(out.event, InvokeEvent::ExitCode("done"));
        let events = p.events();
        assert_eq!(events.len(), 1);
        let e = &events[0];
        for field in [
            "v",
            "seq",
            "ts",
            "type",
            "run_id",
            "machine",
            "trigger",
            "state",
            "phase",
            "script",
            "path",
            "attempt",
            "started_at",
            "duration_ms",
            "exit_code",
            "log",
        ] {
            assert!(e.contains_key(field), "missing {field}: {e:?}");
        }
        assert_eq!(e["v"], 1);
        assert_eq!(e["seq"], 1);
        assert_eq!(e["type"], "script");
        assert_eq!(e["run_id"], RUN_ID);
        assert_eq!(e["machine"], "m");
        assert_eq!(e["trigger"], "inbox");
        assert_eq!(e["state"], "s");
        assert_eq!(e["phase"], "invoke");
        assert_eq!(e["script"], "sleep_one");
        assert_eq!(e["path"], ".decree/scripts/sleep_one.sh");
        assert_eq!(e["attempt"], 1);
        assert_eq!(e["exit_code"], 0);
        assert_eq!(e["log"], "0001-s-sleep_one.log");
        // `timed_out` appears only when true.
        assert!(!e.contains_key("timed_out"));
        for field in ["ts", "started_at"] {
            let ts = e[field].as_str().unwrap();
            assert!(
                chrono::DateTime::parse_from_rfc3339(ts).is_ok() && ts.ends_with('Z'),
                "{field}: {ts}"
            );
            // Milliseconds: `.mmm` before the `Z`.
            assert_eq!(ts.len(), "2026-10-01T14:30:05.123Z".len(), "{ts}");
        }
        let ms = e["duration_ms"].as_u64().unwrap();
        assert!((1000..=2000).contains(&ms), "duration_ms {ms}");
    }

    #[test]
    fn shared_script_path_is_recorded_with_shared_prefix() {
        let p = Project::new(&[]);
        let shared = p.root().join("shared");
        fs::create_dir_all(shared.join("scripts/m")).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scripts");
        fs::copy(
            fixtures.join("exit_zero.sh"),
            shared.join("scripts/m/exit_zero.sh"),
        )
        .unwrap();
        let mut info = p.info();
        info.shared_source = Some(shared);
        let mut exec = Executor::open(info, Arc::clone(&p.shutdown)).unwrap();
        exec.run_script(&ScriptRun::new("exit_zero", "s", Phase::OnEntry))
            .unwrap();
        assert_eq!(p.events()[0]["path"], "shared:scripts/m/exit_zero.sh");
    }

    #[test]
    fn missing_script_is_a_resolution_error() {
        let p = Project::new(&[]);
        let err = p
            .executor()
            .run_script(&ScriptRun::new("absent", "s", Phase::OnEntry))
            .unwrap_err();
        assert!(matches!(
            err,
            RuntimeError::Script(ScriptError::Missing { .. })
        ));
        assert!(p.events().is_empty());
    }

    #[test]
    fn timeout_stops_the_invoke_with_error_and_timed_out() {
        let p = Project::new(&["sleep_long"]);
        let start = Instant::now();
        let out = invoke(
            &p,
            "{ invoke: sleep_long, timeout_s: 1, max_attempts: 1, transitions: { done: done } }",
        );
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_secs(12), "took {elapsed:?}");
        assert_eq!(out.event, InvokeEvent::ExitCode("error"));
        assert!(out.execution.timed_out);
        let scripts = p.events_of("script");
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0]["timed_out"], true);
        // Killed by SIGTERM, so no exit code.
        assert_eq!(scripts[0]["exit_code"], Value::Null);
        let child = p.child_pid().unwrap();
        assert!(wait_until(Duration::from_secs(2), || !alive(child)));
    }

    #[test]
    fn stop_escalates_to_sigkill_after_the_grace_period() {
        let p = Project::new(&["ignore_term"]);
        let start = Instant::now();
        let out = invoke(
            &p,
            "{ invoke: ignore_term, timeout_s: 1, max_attempts: 1, transitions: { done: done } }",
        );
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_secs(1) + KILL_GRACE,
            "took {elapsed:?}"
        );
        assert!(elapsed < Duration::from_secs(14), "took {elapsed:?}");
        assert!(out.execution.timed_out);
        assert_eq!(out.event, InvokeEvent::ExitCode("error"));
        let child = p.child_pid().unwrap();
        assert!(wait_until(Duration::from_secs(2), || !alive(child)));
    }

    /// The only test that signals the test process. `register_signals` replaces SIGTERM's
    /// default action for the whole process, so the signal sets this test's flag only.
    #[test]
    fn sigterm_stops_the_child_and_writes_no_script_event() {
        let p = Project::new(&["sleep_long"]);
        register_signals(&p.shutdown).unwrap();
        let pid_file = p.run_dir().join("child.pid");
        let signaller = thread::spawn(move || {
            assert!(wait_until(Duration::from_secs(10), || pid_file.exists()));
            // SAFETY: sends SIGTERM to this process, whose handler only sets a flag.
            unsafe { libc::kill(libc::getpid(), libc::SIGTERM) };
            Instant::now()
        });
        let m = machine("{ invoke: sleep_long, transitions: { done: done } }");
        let err = p
            .executor()
            .run_invoke(&m, m.find("s").unwrap(), 1)
            .unwrap_err();
        let signalled = signaller.join().unwrap();
        assert!(
            matches!(&err, RuntimeError::Interrupted { script } if script == "sleep_long"),
            "{err:?}"
        );
        let child = p.child_pid().unwrap();
        assert!(wait_until(Duration::from_secs(10), || !alive(child)));
        assert!(signalled.elapsed() < Duration::from_secs(10));
        assert!(p.events_of("script").is_empty(), "{:?}", p.events());
        assert!(p.events().is_empty());
        // The log is kept, so the interrupted output can be read.
        assert!(p.run_dir().join("0001-s-sleep_long.log").exists());
    }

    #[test]
    fn set_shutdown_flag_runs_nothing() {
        let p = Project::new(&["exit_zero"]);
        p.shutdown.store(true, Ordering::SeqCst);
        let err = p
            .executor()
            .run_script(&ScriptRun::new("exit_zero", "s", Phase::OnEntry))
            .unwrap_err();
        assert!(matches!(err, RuntimeError::Interrupted { .. }));
        assert!(p.events().is_empty());
    }

    #[test]
    fn stdout_tail_keeps_the_last_50_lines() {
        let p = Project::new(&["print_lines"]);
        let out = p
            .executor()
            .run_script(&ScriptRun::new("print_lines", "s", Phase::Invoke))
            .unwrap();
        assert_eq!(out.stdout_tail.len(), STDOUT_TAIL_LINES);
        assert_eq!(out.stdout_tail[0], "line 13");
        assert_eq!(out.stdout_tail[47], "line 60");
        assert_eq!(out.stdout_tail[49], "");
        assert_eq!(out.last_line.as_deref(), Some("line 60"));
        assert_eq!(out.printed_event(), None);
    }

    #[test]
    fn log_is_truncated_at_max_log_size() {
        let p = Project::new(&["print_lines"]);
        let mut info = p.info();
        info.max_log_size = 100;
        let mut exec = Executor::open(info, Arc::clone(&p.shutdown)).unwrap();
        let out = exec
            .run_script(&ScriptRun::new("print_lines", "s", Phase::Invoke))
            .unwrap();
        let log = p.log(&out.log);
        assert!(log.starts_with("[log truncated"), "{log}");
        assert!(log.ends_with("line 60\n\n\n"), "{log}");
    }

    #[test]
    fn numbering_continues_after_existing_logs_and_events() {
        let p = Project::new(&["exit_zero"]);
        p.executor()
            .run_script(&ScriptRun::new("exit_zero", "s", Phase::OnEntry))
            .unwrap();
        // An interrupted execution leaves a log without an event.
        fs::write(p.run_dir().join("0007-s-exit_zero.log"), "").unwrap();
        let out = p
            .executor()
            .run_script(&ScriptRun::new("exit_zero", "t", Phase::OnExit))
            .unwrap();
        assert_eq!(out.log, "0008-t-exit_zero.log");
        let seqs: Vec<_> = p.events().iter().map(|e| e["seq"].clone()).collect();
        assert_eq!(seqs, [json!(1), json!(2)]);
    }

    #[test]
    fn data_env_formats_values() {
        let m = load_machine_text(
            "m",
            Path::new("m.yml"),
            "name: m\ndescription: T.\ndata:\n  n: { type: int, default: 2 }\n  \
             b: { type: bool, default: true }\n  s: { type: string, default: hi }\n\
             initial: x\nstates:\n  x: { final: true }\n",
        )
        .unwrap();
        let none = serde_norway::Mapping::new();
        assert_eq!(
            data_env(&m.data, &none),
            [
                ("DECREE_DATA_B".to_string(), "true".to_string()),
                ("DECREE_DATA_N".to_string(), "2".to_string()),
                ("DECREE_DATA_S".to_string(), "hi".to_string()),
            ]
        );
        let params: serde_norway::Mapping = serde_norway::from_str("n: -7\nb: false").unwrap();
        let env = data_env(&m.data, &params);
        assert_eq!(env[0].1, "false");
        assert_eq!(env[1].1, "-7");
    }

    #[test]
    fn format_bytes_units() {
        assert_eq!(format_bytes(500), "500B");
        assert_eq!(format_bytes(2048), "2KB");
        assert_eq!(format_bytes(2_097_152), "2MB");
    }

    #[test]
    fn truncate_log_disabled() {
        let dir = TempDir::new().unwrap();
        let log = dir.path().join("test.log");
        fs::write(&log, "a".repeat(5000)).unwrap();
        truncate_log_if_needed(&log, 0).unwrap();
        assert_eq!(fs::metadata(&log).unwrap().len(), 5000);
    }

    #[test]
    fn truncate_log_under_limit() {
        let dir = TempDir::new().unwrap();
        let log = dir.path().join("test.log");
        fs::write(&log, "small log").unwrap();
        truncate_log_if_needed(&log, 1000).unwrap();
        assert_eq!(fs::read_to_string(&log).unwrap(), "small log");
    }

    #[test]
    fn truncate_log_over_limit() {
        let dir = TempDir::new().unwrap();
        let log = dir.path().join("test.log");
        fs::write(&log, "x".repeat(200)).unwrap();
        truncate_log_if_needed(&log, 100).unwrap();
        let result = fs::read_to_string(&log).unwrap();
        assert!(result.starts_with("[log truncated"));
        assert!(result.contains("100B"));
        assert!(result.ends_with(&"x".repeat(100)));
    }
}
