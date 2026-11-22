//! The pipeline `decree daemon` runs (docs/reference/cli.md): validate the machines, mark runs
//! a crash left behind `interrupted` and continue `pending` runs, then repeat until nothing is
//! left: deliver replies and timeouts, and claim the next `inbox/` message and run it through
//! its machine. Never continues an `interrupted` run. A run that ends in `failed` is reported
//! and the pass goes on. `daemon --once` ends by printing every waiting run, then every run
//! that failed, and exits 1 if there was one.
//!
//! `--retry <id> [--state <s>]` first makes one `interrupted` or finished run `pending`
//! again. It appends a `transition` event with `source: "retry"` and mirrors `state`; the
//! pipeline then continues it, re-running root `onentry` and the `onentry` of every ancestor
//! of `<s>` and of `<s>` (docs/reference/runs.md, step 1).
//!
//! A queued message with `to:` is a reply: it is delivered to its waiting run instead,
//! or, failing a check, becomes a failed `invalid_message` run (docs/reference/messages.md,
//! Replies).
//!
//! Before each run it reports plugins that are not installed as their files say, installing
//! nothing (docs/reference/plugins.md), and reads the machines and `.env` files again
//! (`Pipeline::refresh`), so an edit applies to the next run without a restart.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use serde_json::json;

use crate::attention;
use crate::cli::Format;
use crate::commands::check::{self, md_files, Project};
use crate::commands::print_json;
use crate::commands::report::{self, Reporter, RunNotes};
use crate::error::DecreeError;
use crate::events::{
    current_state, first_text, is_transition, is_type, run_attempt, strings, text, Event, EventLog,
};
use crate::filter::{self, MachineFilter, Select};
use crate::interpreter::child::continue_holding;
use crate::interpreter::recover::{self, mirror_state, RunStatus};
use crate::interpreter::{Context, Interpreter, InterpreterError, Outcome, RunInput};
use crate::layout::MESSAGE_FILE;
use crate::layout::{self, ATTENTION_DIR, DECREE_DIR, INBOX_DIR};
use crate::machine::FAILED;
use crate::message::{self, is_valid_id, Claim, Message, RunLock, ARRIVED_AS};
use crate::reply::{self, Delivery};
use crate::runtime::{self};
use crate::slots::SlotGuard;

/// What `--retry` continues: run `id`.
pub struct Retry {
    pub id: String,
    /// The atomic state to continue in, from `--state`.
    pub state: Option<String>,
}

impl Retry {
    /// From `--retry <id>` and `--state`.
    pub fn from_args(retry: Option<String>, state: Option<String>) -> Option<Retry> {
        retry.map(|id| Retry { id, state })
    }
}

/// Run `decree daemon --once [--dry-run | --retry <id> [--state <s>]] [--quiet]
/// [--machine <m> | --exclude-machine <m>]`: a daemon start, which fires the `@reboot` cron
/// files, then works until only a person or a program can move a run on. Unless `quiet`,
/// each run's output is printed as it goes (`commands::report`).
pub fn run(
    project_root: &Path,
    dry_run: bool,
    retry: Option<Retry>,
    format: Format,
    quiet: bool,
    select: &Select,
) -> Result<(), DecreeError> {
    let mut plugin_notes = BTreeSet::new();
    plugin_notices(project_root, &mut plugin_notes, "");
    let project = Project::load(project_root)?;
    if dry_run {
        // The machine filter's variables are read only when no flag beats them; `load_env`
        // also refuses a bad `DECREE_MAX_DEPTH`, which no flag beats.
        let dotenv = if select.is_given() {
            check::max_depth(&project)?;
            Vec::new()
        } else {
            check::load_env(&project)?.decree
        };
        let filter = select.filter(&dotenv, |m| project.machine_ids.contains(m))?;
        return run_dry(&project, format, &filter);
    }
    let shutdown = Arc::new(AtomicBool::new(false));
    runtime::register_signals(&shutdown)?;
    let observer_ctx = context(project_root, &project, Arc::clone(&shutdown));
    let mut pipeline = Pipeline::new(project_root, project, shutdown, "", select)?;
    pipeline.plugin_notes = plugin_notes;
    if let Some(retry) = retry {
        pipeline.retry(&retry)?;
    }
    let result = reporting(&mut pipeline, &observer_ctx, quiet, |pipeline| {
        pipeline.recover()?;
        crate::commands::daemon::fire_cron_jobs(
            project_root,
            pipeline.filter(),
            "cron fired at daemon start",
            |cf| matches!(cf.when, crate::cron::When::DaemonStart),
        );
        pipeline.drain_with_timers()
    });
    print_waiting(&pipeline.ctx, &pipeline.filter)?;
    let failed = pipeline.take_failed();
    print_failed(&failed);
    print_attention(project_root)?;
    result.map_err(Stop::into_error)?;
    match failed.len() {
        0 => Ok(()),
        n => Err(DecreeError::Other(format!(
            "{n} run(s) ended in `{FAILED}`"
        ))),
    }
}

/// Run `body` with a thread printing each run as it goes (to `.decree/process.log` only, if
/// `quiet`); returns once the thread has printed everything.
pub(crate) fn reporting<R>(
    pipeline: &mut Pipeline,
    observer_ctx: &Context,
    quiet: bool,
    body: impl FnOnce(&mut Pipeline) -> R,
) -> R {
    std::thread::scope(|s| {
        let (reporter, rx) = Reporter::new();
        pipeline.report = Some(reporter);
        let observer = s.spawn(move || report::observe(observer_ctx, rx, quiet));
        let result = body(pipeline);
        // Dropping the reporter closes the channel: the thread prints the rest and ends.
        pipeline.report = None;
        let _ = observer.join();
        result
    })
}

/// Why a pass of the pipeline stopped short.
#[derive(Debug)]
pub(crate) enum Stop {
    /// SIGINT or SIGTERM: the current run is `interrupted`.
    Interrupted,
    /// A file in `.decree/attention/` holds the queue: nothing runs until it is gone. Holds
    /// what to print.
    Held(String),
    /// Anything else: an I/O error, or a run that cannot be stepped.
    Error(DecreeError),
}

impl Stop {
    pub(crate) fn into_error(self) -> DecreeError {
        match self {
            Stop::Held(message) => DecreeError::Other(message),
            Stop::Interrupted => DecreeError::Interrupted,
            Stop::Error(e) => e,
        }
    }
}

impl<E: Into<DecreeError>> From<E> for Stop {
    fn from(e: E) -> Self {
        Stop::Error(e.into())
    }
}

/// The steps `daemon` and `daemon --once` share (docs/reference/cli.md): one pipeline. Each
/// step runs to the end of one run before it returns.
pub(crate) struct Pipeline {
    ctx: Context,
    project: Arc<Project>,
    /// Inbox files another process claimed first, or is claiming now.
    lost: HashSet<String>,
    /// The machines this process runs (`--machine`, `--exclude-machine`).
    filter: MachineFilter,
    /// Put before each failed run's report: `daemon --once` prints none, `daemon` its name.
    prefix: &'static str,
    /// Runs that ended in `failed` since the last `take_failed`.
    failed: Vec<String>,
    /// Told when each run starts and stops, unless `--quiet`.
    pub(crate) report: Option<Reporter>,
    /// The last machine or `.env` problem `refresh` printed, so each is printed once.
    machine_error: Option<String>,
    env_error: Option<String>,
    /// The plugin problems last printed (`plugin_notices`), so each is printed once.
    pub(crate) plugin_notes: BTreeSet<String>,
    /// The runs executing in a worker thread now, each holding a slot of its machine.
    in_flight: HashSet<String>,
    workers: Vec<std::thread::JoinHandle<()>>,
    done_tx: Sender<Done>,
    done_rx: Receiver<Done>,
}

/// Print, once each, every plugin `decree install` would install or remove and every
/// problem that would stop one (a conflict, a plugin that fails its schema): `daemon`
/// installs nothing (docs/reference/plugins.md, Installing). `printed` holds the
/// lines already printed, and keeps those still there.
pub(crate) fn plugin_notices(project_root: &Path, printed: &mut BTreeSet<String>, prefix: &str) {
    let now: BTreeSet<String> = match crate::plugin::status(&project_root.join(DECREE_DIR)) {
        Ok(synced) => synced.notices().into_iter().collect(),
        Err(e) => BTreeSet::from([format!("cannot read plugins: {e}")]),
    };
    for note in now.difference(printed) {
        report::notice(format!("{prefix}{note}"));
    }
    *printed = now;
}

impl Pipeline {
    /// Validate every machine, and set up the context runs are stepped in, and the machines `select` runs. Nothing runs if
    /// anything is invalid.
    pub(crate) fn new(
        project_root: &Path,
        project: Project,
        shutdown: Arc<AtomicBool>,
        prefix: &'static str,
        select: &Select,
    ) -> Result<Self, DecreeError> {
        if !project.problems.is_empty() {
            for problem in &project.problems {
                eprintln!("{problem}");
            }
            return Err(DecreeError::Other(format!(
                "{} machine error(s); nothing was processed. Run `decree check`.",
                project.problems.len()
            )));
        }
        let mut ctx = context(project_root, &project, shutdown);
        ctx.env = check::load_env(&project)?;
        let filter = select.filter(&ctx.env.decree, |m| project.machine_ids.contains(m))?;
        let (done_tx, done_rx) = mpsc::channel();
        Ok(Pipeline {
            ctx,
            project: Arc::new(project),
            lost: HashSet::new(),
            filter,
            prefix,
            failed: Vec::new(),
            report: None,
            machine_error: None,
            env_error: None,
            plugin_notes: BTreeSet::new(),
            in_flight: HashSet::new(),
            workers: Vec::new(),
            done_tx,
            done_rx,
        })
    }

    /// `--retry`: make run `id` `pending` again.
    pub(crate) fn retry(&self, retry: &Retry) -> Result<(), DecreeError> {
        retry_run(
            &self.ctx,
            &self.project,
            &self.filter,
            &retry.id,
            retry.state.as_deref(),
        )
    }

    /// The machines this process runs.
    pub(crate) fn filter(&self) -> &MachineFilter {
        &self.filter
    }

    /// Whether this process runs run `id`: its root run's machine is one it runs.
    fn owns(&self, id: &str) -> bool {
        owns(&self.ctx, &self.filter, id)
    }

    /// Read the machines and the `.env` files again, so edits apply to the next run without
    /// a restart. A machine with errors keeps every machine as last loaded, and a malformed
    /// `.env` file the variables last read; each problem is printed once, until it changes.
    /// A run already going keeps its machine until it stops.
    pub(crate) fn refresh(&mut self) {
        plugin_notices(&self.ctx.project_root, &mut self.plugin_notes, self.prefix);
        match self.reload_machines() {
            Ok(changed) => {
                self.machine_error = None;
                if !changed.is_empty() {
                    report::notice(format!(
                        "{}reloaded machines: {}",
                        self.prefix,
                        changed.join(", ")
                    ));
                }
            }
            Err(message) => {
                if self.machine_error.as_ref() != Some(&message) {
                    report::notice(format!(
                        "{message}\n{}runs keep the machines last loaded until this is fixed. \
                         Run `decree check`.",
                        self.prefix
                    ));
                    self.machine_error = Some(message);
                }
            }
        }
        match check::load_env(&self.project) {
            Ok(env) => {
                self.ctx.env = env;
                self.env_error = None;
            }
            Err(e) => {
                let message = e.to_string();
                if self.env_error.as_ref() != Some(&message) {
                    report::notice(format!(
                        "{}{message}; keeping the variables last read",
                        self.prefix
                    ));
                    self.env_error = Some(message);
                }
            }
        }
    }

    /// Load the machines again and use them if they changed and have no errors. Returns the
    /// machines added, changed or removed, or the errors.
    fn reload_machines(&mut self) -> Result<Vec<String>, String> {
        // Unchanged files need no new check, unless the last one failed: it may have been a
        // script that is now there.
        if self.machine_error.is_none() && self.project.sources == self.read_sources()? {
            return Ok(Vec::new());
        }
        let project = Project::load(&self.ctx.project_root).map_err(|e| e.to_string())?;
        if !project.problems.is_empty() {
            let mut lines: Vec<String> = project.problems.iter().map(ToString::to_string).collect();
            lines.push(format!("{} machine error(s)", project.problems.len()));
            return Err(lines.join("\n"));
        }
        let (old, new) = (&self.project.sources, &project.sources);
        let changed: BTreeSet<&String> = old
            .keys()
            .chain(new.keys())
            .filter(|id| old.get(*id) != new.get(*id))
            .collect();
        let changed: Vec<String> = changed.into_iter().cloned().collect();
        if !changed.is_empty() {
            self.ctx.machines = Arc::clone(&project.machines);
            self.project = Arc::new(project);
        }
        Ok(changed)
    }

    /// Every machine file's text, by machine id.
    fn read_sources(&self) -> Result<BTreeMap<String, String>, String> {
        let decree_dir = self.ctx.project_root.join(DECREE_DIR);
        let paths = crate::machine::machine_paths(&decree_dir).map_err(|e| e.to_string())?;
        paths
            .into_iter()
            .map(|(id, path)| {
                Ok((
                    id,
                    std::fs::read_to_string(&path).map_err(|e| e.to_string())?,
                ))
            })
            .collect()
    }

    fn shutdown(&self) -> bool {
        self.ctx.shutdown.load(Ordering::Relaxed)
    }

    /// Stop for a signal, or for a file in `.decree/attention/` that holds the queue, before
    /// a run starts or continues.
    fn may_run(&self) -> Result<(), Stop> {
        if self.shutdown() {
            return Err(Stop::Interrupted);
        }
        match attention::held(&self.ctx)? {
            Some(message) => Err(Stop::Held(message)),
            None => Ok(()),
        }
    }

    /// Report run `id`, which ended in `failed`, with its `--retry` command.
    /// Inbox messages are independent, so the pass goes on (docs/reference/messages.md, Lifecycle).
    fn fail(&mut self, id: &str, file: Option<&str>) {
        let file = file.map(|f| format!(" ({f})")).unwrap_or_default();
        report::notice(format!(
            "{}run {id}{file} ended in `{FAILED}`; see .decree/runs/{id}/, \
             then continue it with `decree daemon --once --retry {id}`",
            self.prefix
        ));
        self.failed.push(id.to_string());
    }

    /// The runs that ended in `failed` since the last call, in the order they failed.
    pub(crate) fn take_failed(&mut self) -> Vec<String> {
        std::mem::take(&mut self.failed)
    }

    /// At start: mark runs a crash left behind `interrupted` (never continued). The
    /// `pending` runs it leaves are dispatched by the next pass (docs/reference/messages.md,
    /// Run status).
    pub(crate) fn recover(&mut self) -> Result<(), Stop> {
        let recovery = recover::recover(&self.ctx, |id| self.owns(id))?;
        for (id, state) in &recovery.crashed {
            eprintln!(
                "run {id} was interrupted in `{state}` (crash); \
                 continue it with `decree daemon --once --retry {id}`"
            );
        }
        Ok(())
    }

    /// Repeat until nothing is left and no run is executing: deliver timeouts and replies,
    /// start every `pending` run and inbox message whose machine has a free slot, and wait
    /// for a run to stop. A failed run is reported and the loop goes on. On a stop (a signal,
    /// a held queue, an I/O error), every executing run is waited for first.
    pub(crate) fn drain(&mut self) -> Result<(), Stop> {
        let result = self.drain_until_idle();
        if result.is_err() {
            self.wait_all();
        }
        result
    }

    fn drain_until_idle(&mut self) -> Result<(), Stop> {
        let inbox = self.ctx.project_root.join(DECREE_DIR).join(INBOX_DIR);
        loop {
            self.reap(None);
            if self.dispatch()? {
                continue;
            }
            if self.in_flight.is_empty() {
                return Ok(());
            }
            // Nothing could start: look again when a run stops, a file arrives in inbox/, or
            // a second passes (a deadline, a run another process made pending), not every
            // `WAKE`, since a pass reads every run's events.
            let listing = md_files(&inbox)?;
            let since = std::time::Instant::now();
            loop {
                let before = self.in_flight.len();
                self.reap(Some(WAKE));
                if self.in_flight.len() < before
                    || since.elapsed() >= RESCAN
                    || md_files(&inbox)? != listing
                {
                    break;
                }
                self.may_run()?;
            }
        }
    }

    /// `drain`, and again after each timer a run waits on (a reply wait with `attention:
    /// false` and a `timeout`, docs/reference/cli.md, `daemon --once`), sleeping until the
    /// earliest: returns when nothing can move on without outside input.
    pub(crate) fn drain_with_timers(&mut self) -> Result<(), Stop> {
        loop {
            self.drain()?;
            let Some(at) = reply::next_timer(&self.ctx, |id| self.owns(id))? else {
                return Ok(());
            };
            while chrono::Utc::now() < at {
                self.may_run()?;
                let left = (at - chrono::Utc::now())
                    .to_std()
                    .unwrap_or_default()
                    .min(std::time::Duration::from_millis(100));
                std::thread::sleep(left);
            }
        }
    }

    /// One pass of the daemon: what can start starts, and the runs that stopped are
    /// collected; it never waits for a run. Returns whether anything moved.
    pub(crate) fn step(&mut self) -> Result<bool, Stop> {
        self.reap(None);
        self.dispatch()
    }

    /// Wait up to `wait` for a run to stop (the daemon's sleep between passes), collecting
    /// each that did. Returns whether one did.
    pub(crate) fn sleep(&mut self, wait: std::time::Duration) -> Result<bool, Stop> {
        if self.in_flight.is_empty() {
            std::thread::sleep(wait);
            return Ok(false);
        }
        let before = self.in_flight.len();
        self.reap(Some(wait));
        Ok(self.in_flight.len() < before)
    }

    /// Every `pending` run this process runs, in `id` order: after `--retry`, a delivered
    /// reply or timeout, a finished child, or a delivery another process made. A run another
    /// process holds is `active`, not `pending`.
    pub(crate) fn pending(&self) -> Result<Vec<String>, Stop> {
        let mut pending = Vec::new();
        for id in message::run_ids(&self.ctx.runs_dir())? {
            if self.ctx.run_finished(&id)?.is_none()
                && self.ctx.status_of(&id)?.0 == RunStatus::Pending
                && self.owns(&id)
            {
                pending.push(id);
            }
        }
        Ok(pending)
    }

    /// Start what can start (docs/reference/machines.md, Concurrency): deliver passed
    /// deadlines (a reply queued before one wins), then each `pending` run, then each inbox
    /// file in filename order, a reply delivered and a message claimed, each run in a worker
    /// thread holding a slot of its machine. A machine with no free slot keeps its messages
    /// queued, in order, and its runs pending; the others go on. Returns whether anything
    /// moved.
    fn dispatch(&mut self) -> Result<bool, Stop> {
        self.may_run()?;
        self.refresh();
        let mut progressed = false;
        // Deadlines: replies queued before them first, then the timeouts. Both only append
        // an event; the runs they make pending start below.
        let now = chrono::Utc::now();
        progressed |= self.deliver_early_replies(now)?;
        progressed |= !reply::deliver_timeouts(&self.ctx, now, |id| self.owns(id))?.is_empty();
        progressed |= self.start_pending()?;
        let decree_dir = self.ctx.project_root.join(DECREE_DIR);
        let inbox = decree_dir.join(INBOX_DIR);
        message::arrive(&decree_dir)?;
        // A machine (or group) found full keeps the rest of its messages queued, in order.
        let mut full: HashSet<String> = HashSet::new();
        // A run executing, about to (pending, its machine at its limit, in any process), or
        // answered in this pass, gets its next reply once it waits again: replies to one run
        // go in the order they were queued, each to the wait the run is in by then, as when
        // one run went at a time.
        let mut answered: HashSet<String> = HashSet::new();
        for file in md_files(&inbox)? {
            self.may_run()?;
            if self.lost.contains(&file) || !message::is_queued(&decree_dir, &file) {
                continue;
            }
            let machine = inbox_machine(&self.ctx.runs_dir(), &inbox, &file);
            if !self.filter.is_all() && !self.filter.runs(machine.as_deref()) {
                continue;
            }
            let Ok(message) = Message::read(&inbox.join(&file)) else {
                progressed |= self.claim_inline(file, None)?;
                continue;
            };
            if message.frontmatter.contains_key("to") {
                let run = message
                    .text("to")
                    .and_then(|to| reply::target_run(&self.ctx.runs_dir(), to));
                if let Some(run) = run {
                    if self.executing_here(&run) || self.not_yet_waiting(&run) || !answered.insert(run) {
                        continue;
                    }
                }
                progressed |= self.take_reply(file, &message)?;
                continue;
            }
            let Some(name) = message
                .machine()
                .filter(|m| self.ctx.machines.contains_key(*m))
                .map(String::from)
            else {
                // No machine to run it: claimed and failed with `invalid_message` at once.
                progressed |= self.claim_inline(file, None)?;
                continue;
            };
            // Full is per slot key: a group found full keeps every member's messages queued.
            let key = crate::interpreter::child::slot_of(&self.ctx, &name).0;
            if full.contains(&key) {
                continue;
            }
            let Some(slot) = self.try_slot(&name)? else {
                full.insert(key);
                continue;
            };
            let Some(claim) = message::claim(&decree_dir, &file)? else {
                self.lost.insert(file);
                continue;
            };
            let arrived = message.text(ARRIVED_AS).map(String::from).unwrap_or(file);
            // The run is locked here, before its worker starts, so from its claim on it is
            // `active` to every reader (`decree status`, the `order` plugin), never a run with
            // no events and no lock, which reads as interrupted. The worker steps it holding
            // this lock.
            let lock = RunLock::acquire(&claim.run_dir)
                .map_err(|e| DecreeError::Other(format!("{}: {e}", claim.run_dir.display())))?;
            self.spawn(
                claim.id.clone(),
                Some(arrived),
                slot,
                Job::Claim(Box::new(claim), lock),
            );
            progressed = true;
        }
        Ok(progressed)
    }

    /// Start each `pending` run not executing here whose machine has a free slot.
    fn start_pending(&mut self) -> Result<bool, Stop> {
        let mut progressed = false;
        for id in self.pending()? {
            if self.in_flight.contains(&id) {
                continue;
            }
            let Some(machine) = run_machine(&self.ctx, &id) else {
                continue;
            };
            if let Some(slot) = self.try_slot(&machine)? {
                self.spawn(id, None, slot, Job::Continue);
                progressed = true;
            }
        }
        Ok(progressed)
    }

    /// Whether run `id` executes in this process now: dispatched here, or a child run a
    /// worker here steps (its lock holds this process's id).
    fn executing_here(&self, id: &str) -> bool {
        self.in_flight.contains(id)
            || matches!(
                message::lock_state(&self.ctx.runs_dir().join(id)),
                Ok(message::LockState::Live(pid)) if pid == std::process::id()
            )
    }

    /// Whether run `id` is `active` or `pending`: it will wait again, or finish, and a reply
    /// to it is delivered then (or rejected, once it finished).
    fn not_yet_waiting(&self, id: &str) -> bool {
        matches!(
            self.ctx.status_of(id).map(|(status, _)| status),
            Ok(RunStatus::Active | RunStatus::Pending)
        )
    }

    /// A free slot of machine `name`.
    fn try_slot(&self, name: &str) -> Result<Option<SlotGuard>, Stop> {
        let (key, n) = crate::interpreter::child::slot_of(&self.ctx, name);
        let dir = self.ctx.project_root.join(DECREE_DIR);
        self.ctx
            .slots
            .try_take(&key, n)
            .map_err(|e| Stop::Error(DecreeError::Other(format!("{}: {e}", dir.display()))))
    }

    /// Run `job`, run `id`, in a worker thread holding `slot`; it reports back when the
    /// run stops.
    fn spawn(&mut self, id: String, file: Option<String>, slot: SlotGuard, job: Job) {
        self.in_flight.insert(id.clone());
        let ctx = self.ctx.clone();
        let project = Arc::clone(&self.project);
        let notes = self.report.as_ref().map(Reporter::notes);
        let tx = self.done_tx.clone();
        let handle = std::thread::spawn(move || {
            let notes = notes.as_ref();
            // A panic in the run is that run's error: the worker still reports back, so the
            // run leaves `in_flight` and the daemon goes on.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match job {
                Job::Continue => {
                    match reported(notes, &ctx, &id, || continue_holding(&ctx, &id, Some(slot))) {
                        Ok(outcome) => Ok(Some(outcome)),
                        // Another process holds it now.
                        Err(InterpreterError::Active(_)) => Ok(None),
                        Err(e) => Err(Stop::from(e)),
                    }
                }
                Job::Claim(claim, lock) => {
                    let _slot = slot;
                    reported(notes, &ctx, &id, || run_claim(&ctx, &project, *claim, None, lock))
                        .map(Some)
                        .map_err(Stop::from)
                }
            }))
            .unwrap_or_else(|panic| {
                let what = panic
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "a panic".to_string());
                Err(Stop::Error(DecreeError::Other(format!("it panicked: {what}"))))
            });
            let _ = tx.send(Done { id, file, result });
        });
        self.workers.push(handle);
    }

    /// Collect the runs that stopped: each one ready now, or, with `wait`, the first to stop
    /// within it and those ready after it. A run that ended in `failed`, or stopped with an
    /// error, is reported, and the others go on.
    fn reap(&mut self, wait: Option<std::time::Duration>) {
        let first = match wait {
            Some(wait) if !self.in_flight.is_empty() => self.done_rx.recv_timeout(wait).ok(),
            _ => None,
        };
        let mut done: Vec<Done> = first.into_iter().collect();
        done.extend(self.done_rx.try_iter());
        for Done { id, file, result } in done {
            self.in_flight.remove(&id);
            match result {
                Ok(Some(Outcome::Finished(state))) if state == FAILED => {
                    self.fail(&id, file.as_deref());
                }
                Ok(_) | Err(Stop::Interrupted) => {}
                // One run's error is that run's: it is reported, and the others go on.
                Err(stop) => {
                    let e = stop.into_error();
                    report::notice(format!(
                        "{}run {id} stopped with an error: {e}; see .decree/runs/{id}/",
                        self.prefix
                    ));
                    self.failed.push(id.clone());
                }
            }
        }
        self.workers.retain(|h| !h.is_finished());
    }

    /// Wait for every executing run to stop, collecting each: on a signal, each running
    /// script is stopped and its run `interrupted`.
    pub(crate) fn wait_all(&mut self) {
        while !self.in_flight.is_empty() {
            self.reap(Some(WAKE));
        }
        for handle in self.workers.drain(..) {
            let _ = handle.join();
        }
    }

    /// For each run waiting past its deadline at `now`, take the queued replies to its wait
    /// that were queued at or before the deadline, in filename order (the order they were
    /// queued), until one is delivered. One that fails a check is rejected as any reply is.
    fn deliver_early_replies(&mut self, now: chrono::DateTime<chrono::Utc>) -> Result<bool, Stop> {
        let overdue = reply::overdue(&self.ctx, now, |id| self.owns(id))?;
        if overdue.is_empty() {
            return Ok(false);
        }
        let decree_dir = self.ctx.project_root.join(DECREE_DIR);
        let inbox = decree_dir.join(INBOX_DIR);
        // A file written straight into `inbox/` gets its id, the time that counts, now.
        message::arrive(&decree_dir)?;
        // One scan for every wait; a reply that arrives later is delivered on the next pass.
        let replies = reply::queued(&inbox)
            .map_err(|e| DecreeError::Other(format!("{}: {e}", inbox.display())))?;
        let mut progressed = false;
        for wait in &overdue {
            let answers = replies.iter().filter(|(file, to)| {
                file.strip_suffix(".md")
                    .is_some_and(|id| wait.answered_by(to, id))
            });
            for (file, _) in answers {
                self.may_run()?;
                // Taken already (for another wait, or by another process) or lost: skip it.
                if self.lost.contains(file) || !message::is_queued(&decree_dir, file) {
                    continue;
                }
                let Ok(message) = Message::read(&inbox.join(file)) else {
                    continue;
                };
                progressed = true;
                if self.take_reply(file.clone(), &message)? {
                    break;
                }
            }
        }
        Ok(progressed)
    }

    /// Deliver queued reply `inbox/<file>` to its waiting run, which is then `pending` and
    /// starts like any other; one that fails a check becomes a failed `invalid_message` run.
    /// Returns whether it was delivered.
    fn take_reply(&mut self, file: String, message: &Message) -> Result<bool, Stop> {
        let inbox = self.ctx.project_root.join(DECREE_DIR).join(INBOX_DIR);
        let (to, event) = (message.text("to"), message.text("event"));
        match reply::deliver(&self.ctx, &inbox, &file, to, event)? {
            Delivery::Delivered(_) => Ok(true),
            Delivery::Lost => {
                self.lost.insert(file);
                Ok(false)
            }
            Delivery::Rejected(reason) => {
                self.claim_inline(file, Some(reason))?;
                Ok(false)
            }
        }
    }

    /// Claim `inbox/<file>` and fail it here, with no worker: a message that does not parse
    /// or names no machine, or a reply that failed a check (`reply_problem`). Nothing of a
    /// machine runs. Returns whether it was claimed.
    fn claim_inline(&mut self, file: String, reply_problem: Option<String>) -> Result<bool, Stop> {
        let decree_dir = self.ctx.project_root.join(DECREE_DIR);
        let arrived_as = Message::read(&decree_dir.join(INBOX_DIR).join(&file))
            .ok()
            .and_then(|m| m.text(ARRIVED_AS).map(String::from));
        let Some(claim) = message::claim(&decree_dir, &file)? else {
            self.lost.insert(file);
            return Ok(false);
        };
        let id = claim.id.clone();
        let notes = self.report.as_ref().map(Reporter::notes);
        let outcome = reported(notes.as_ref(), &self.ctx, &id, || {
            run_claim(&self.ctx, &self.project, claim, reply_problem, None)
        })?;
        if matches!(&outcome, Outcome::Finished(state) if state == FAILED) {
            self.fail(&id, Some(arrived_as.as_deref().unwrap_or(&file)));
        }
        Ok(true)
    }
}

/// How long the dispatcher waits for a run to stop before it looks at the queue again.
const WAKE: std::time::Duration = std::time::Duration::from_millis(100);

/// How long `daemon --once` waits, with runs executing and nothing else to start, before it
/// reads every run again: for deadlines and for runs another process made pending.
const RESCAN: std::time::Duration = std::time::Duration::from_secs(1);

/// What a worker thread does with its run.
enum Job {
    /// Continue the `pending` run.
    Continue,
    /// Run the claimed inbox message, holding its run lock when the dispatcher took it.
    Claim(Box<Claim>, Option<RunLock>),
}

/// A worker's report: run `id` (claimed from `file`, if it was) stopped with `result`;
/// `Ok(None)` when another process held it.
struct Done {
    id: String,
    file: Option<String>,
    result: Result<Option<Outcome>, Stop>,
}

/// The machine run `id` runs, from its `message.md`.
fn run_machine(ctx: &Context, id: &str) -> Option<String> {
    Message::read(&ctx.runs_dir().join(id).join(MESSAGE_FILE))
        .ok()?
        .machine()
        .map(String::from)
}

/// Whether a process that runs `filter` runs run `id`: its root run's machine is one it runs.
fn owns(ctx: &Context, filter: &MachineFilter, id: &str) -> bool {
    filter.is_all() || filter.runs(filter::owner(&ctx.runs_dir(), id).as_deref())
}

/// The machine `inbox/<file>` is for: a reply's, that of the run it answers; a message's, its
/// `machine`. `None` when it cannot be read.
fn inbox_machine(runs_dir: &Path, inbox: &Path, file: &str) -> Option<String> {
    let message = Message::read(&inbox.join(file)).ok()?;
    if message.frontmatter.contains_key("to") {
        let run = reply::target_run(runs_dir, message.text("to")?)?;
        return filter::owner(runs_dir, &run);
    }
    message.machine().map(String::from)
}

/// Run `run`, the steps of run `id`, telling `report` when it starts and stops.
fn reported<E>(
    report: Option<&RunNotes>,
    ctx: &Context,
    id: &str,
    run: impl FnOnce() -> Result<Outcome, E>,
) -> Result<Outcome, E> {
    let Some(report) = report else {
        return run();
    };
    report.start(id, &ctx.runs_dir().join(id), queued(ctx));
    let outcome = run();
    report.end(id, outcome.as_ref().ok());
    outcome
}

/// How many inbox messages wait.
fn queued(ctx: &Context) -> usize {
    md_files(&ctx.project_root.join(DECREE_DIR).join(INBOX_DIR)).map_or(0, |f| f.len())
}

/// The context runs of `project` are stepped, or their status derived, in.
pub(crate) fn context(
    project_root: &Path,
    project: &Project,
    shutdown: Arc<AtomicBool>,
) -> Context {
    Context {
        project_root: project_root.to_path_buf(),
        machines: Arc::clone(&project.machines),
        shutdown,
        env: Default::default(),
        slots: crate::slots::Slots::new(project_root),
    }
}

/// Print every run that ended in `failed` in this pass, with the command that continues it.
fn print_failed(failed: &[String]) {
    if failed.is_empty() {
        return;
    }
    eprintln!("{} run(s) ended in `{FAILED}` in this pass:", failed.len());
    for id in failed {
        eprintln!("  run {id}: decree daemon --once --retry {id}");
    }
}

/// `<n> run(s) need attention: .decree/attention/`, when the folder has files.
fn print_attention(project_root: &Path) -> Result<(), DecreeError> {
    let n = attention::list(project_root)?.len();
    if n > 0 {
        eprintln!("{n} run(s) need attention: {DECREE_DIR}/{ATTENTION_DIR}/");
    }
    Ok(())
}

/// Lifecycle steps 2 to 4 for a claimed inbox message: set `id` and `trigger` if missing,
/// validate, and start the run, or fail it with `invalid_message`. A reply (`to:`) never
/// starts a run: it was not delivered, for `reply_problem`.
fn run_claim(
    ctx: &Context,
    project: &Project,
    claim: Claim,
    reply_problem: Option<String>,
    lock: Option<RunLock>,
) -> Result<Outcome, DecreeError> {
    let Claim {
        id,
        run_dir,
        file,
        message,
        id_problem,
    } = claim;
    let mut message = match message {
        Ok(message) => message,
        // The frontmatter did not parse: leave message.md unchanged.
        Err(e) => return reject(ctx, &id, "", "inbox", &file, &e.to_string(), false),
    };
    // The name the person or tool gave the file, before it got its id.
    let file = message.text(ARRIVED_AS).map_or(file, String::from);
    let trigger = message.text("trigger").unwrap_or("inbox").to_string();
    if message.frontmatter.get("id").is_none() || id_problem.is_some() {
        message.set("id", id.as_str());
    }
    if !message.frontmatter.contains_key("trigger") {
        message.set("trigger", trigger.as_str());
    }
    message.write(&run_dir.join(MESSAGE_FILE))?;

    let name = message.machine().unwrap_or_default().to_string();
    if let Some(problem) = id_problem {
        return reject(ctx, &id, &name, &trigger, &file, &problem, true);
    }
    if message.frontmatter.contains_key("to") {
        let reason = reply_problem.map_or_else(
            || "a reply (`to:`) was not delivered".to_string(),
            |p| format!("reply not delivered: {p}"),
        );
        return reject(ctx, &id, &name, &trigger, &file, &reason, true);
    }
    start(ctx, project, &id, &trigger, &file, &message, lock)
}

/// Validate `message`, now `runs/<id>/message.md`, and start its run; an invalid message
/// starts in `failed` (docs/reference/messages.md, Lifecycle step 3).
fn start(
    ctx: &Context,
    project: &Project,
    id: &str,
    trigger: &str,
    file: &str,
    message: &Message,
    lock: Option<RunLock>,
) -> Result<Outcome, DecreeError> {
    let name = match message::validate(message, &project.machines, &project.machine_ids) {
        Ok(name) => name,
        Err(errors) => {
            let reason = errors
                .iter()
                .map(|(line, msg)| format!("line {line}: {msg}"))
                .collect::<Vec<_>>()
                .join("; ");
            let name = message.machine().unwrap_or_default();
            return reject(ctx, id, name, trigger, file, &reason, true);
        }
    };
    // `Pipeline::new` refuses a project with a machine that does not load.
    let machine = project
        .machines
        .get(&name)
        .ok_or_else(|| DecreeError::Other(format!("machine `{name}` does not load")))?;
    let input = RunInput::new(message, Some(file.to_string()));
    let executor = ctx.executor(machine, id, trigger, &input.params, message.text("parent"))?;
    Ok(Interpreter::new(ctx, machine, executor, input)?.start_holding(lock)?)
}

/// Fail a run at its claim with `invalid_message` and `reason`: one `transition` event to
/// `failed`, and `state: failed` mirrored unless the frontmatter did not parse.
fn reject(
    ctx: &Context,
    id: &str,
    machine: &str,
    trigger: &str,
    file: &str,
    reason: &str,
    mirror: bool,
) -> Result<Outcome, DecreeError> {
    let run_dir = ctx.runs_dir().join(id);
    let mut events = EventLog::open(&run_dir, id, machine, trigger)?;
    recover::reject(&mut events, &run_dir, file, reason, mirror)?;
    attention::record(ctx, id)?;
    report::notice(format!("{file}: invalid message: {reason}"));
    Ok(Outcome::Finished(FAILED.to_string()))
}

/// `--retry` (docs/reference/cli.md): make an `interrupted` or finished run `pending` again,
/// in `state` or by default the state it was in (for a finished run, the state it last
/// left). The pipeline then continues it.
fn retry_run(
    ctx: &Context,
    project: &Project,
    filter: &MachineFilter,
    id: &str,
    state: Option<&str>,
) -> Result<(), DecreeError> {
    let run_dir = ctx.runs_dir().join(id);
    if !is_valid_id(id) || !run_dir.is_dir() {
        return Err(DecreeError::MessageNotFound(id.to_string()));
    }
    if !owns(ctx, filter, id) {
        let machine = filter::owner(&ctx.runs_dir(), id).unwrap_or_default();
        return Err(DecreeError::Other(format!(
            "run {id} is for machine `{machine}`, which this process does not run"
        )));
    }
    // Hold the lock while writing, so no other process steps the run meanwhile.
    let Some(_lock) = RunLock::acquire(&run_dir)? else {
        return Err(refuse_retry(id, RunStatus::Active));
    };
    let events = ctx.events(id)?;
    let machine_name = first_text(&events, "machine").unwrap_or_default();
    let Some(machine) = project.machines.get(machine_name) else {
        return Err(DecreeError::Other(format!(
            "run {id}: machine `{machine_name}` is not loaded; it cannot be continued"
        )));
    };
    // This process holds the lock, so no other is stepping the run.
    let status = ctx.status(machine, &events, false);
    match status {
        RunStatus::Interrupted | RunStatus::Finished => {}
        RunStatus::Waiting => {
            return Err(DecreeError::Other(format!(
                "run {id} is waiting for a reply, not interrupted; answer it with `decree reply`"
            )))
        }
        RunStatus::Pending | RunStatus::Active => return Err(refuse_retry(id, status)),
    }

    let from = current_state(&events);
    let target = match (state, status) {
        (Some(s), _) => s.to_string(),
        (None, RunStatus::Finished) => last_from(&events).ok_or_else(|| {
            DecreeError::Other(format!(
                "run {id} never left a state; name one with --state"
            ))
        })?,
        (None, _) => from
            .ok_or_else(|| {
                DecreeError::Other(format!("run {id} has no state; name one with --state"))
            })?
            .to_string(),
    };
    let atomic = machine.find(&target).is_some_and(|n| {
        let node = &machine.nodes[n];
        node.children.is_empty() && !node.is_final
    });
    if !atomic {
        return Err(DecreeError::Other(format!(
            "`{target}` is not an atomic state of machine `{machine_name}`"
        )));
    }

    let trigger = first_text(&events, "trigger").unwrap_or_default();
    let mut log = EventLog::open(&run_dir, id, machine_name, trigger)?;
    log.append(
        "transition",
        json!({
            "from": from,
            "event": "retry",
            "to": target,
            "source": "retry",
            "exit_code": null,
            "run_attempt": run_attempt(&events) + 1,
        }),
    )?;
    mirror_state(&run_dir.join(MESSAGE_FILE), &target)?;
    println!("run {id} continues in `{target}`");
    attention::release(ctx, id)?;
    attention::record(ctx, id)?;
    Ok(())
}

/// The `from` of the last `transition` event that has one.
fn last_from(events: &[Event]) -> Option<String> {
    events
        .iter()
        .rev()
        .filter(|e| is_transition(e))
        .find_map(|e| text(e, "from"))
        .map(String::from)
}

fn refuse_retry(id: &str, status: RunStatus) -> DecreeError {
    DecreeError::Other(format!(
        "run {id} is {}; only an interrupted or finished run can be retried",
        status.as_str()
    ))
}

/// Print every waiting run of the machines `filter` runs: its question (the state's
/// `description`), its wait id and options, and a `decree reply` command for each option
/// (docs/reference/messages.md, Replies).
fn print_waiting(ctx: &Context, filter: &MachineFilter) -> Result<(), DecreeError> {
    for id in message::run_ids(&ctx.runs_dir())? {
        if ctx.run_finished(&id)?.is_some() || !owns(ctx, filter, &id) {
            continue;
        }
        let events = ctx.events(&id)?;
        let Some(last) = events
            .last()
            .filter(|e| is_type(e, "waiting") && crate::attention::wants_attention(e))
        else {
            continue;
        };
        let (Some(state), Some(wait_id)) = (text(last, "state"), text(last, "wait_id")) else {
            continue;
        };
        let question = ctx
            .machines
            .get(text(last, "machine").unwrap_or_default())
            .and_then(|m| m.find(state).map(|s| &m.nodes[s]))
            .and_then(|node| node.description.as_deref())
            .unwrap_or(state);
        let options = strings(last, "options");
        println!("Waiting: run {id} in `{state}`: {question}");
        println!("  wait id {wait_id}, options: {}", options.join(", "));
        for option in options {
            println!("  decree reply {wait_id} {option}");
        }
    }
    Ok(())
}

/// `decree daemon --once --dry-run`: list the queued inbox messages and run nothing; invalid
/// ones with their errors. With a machine filter, only those for its machines.
fn run_dry(project: &Project, format: Format, filter: &MachineFilter) -> Result<(), DecreeError> {
    let mut problems = project.problems.clone();
    let mut document = serde_json::Map::new();
    let inbox_dir = project.decree_dir.join(INBOX_DIR);
    let runs_dir = project.decree_dir.join(layout::RUNS_DIR);
    let mut inbox = md_files(&inbox_dir)?;
    if !filter.is_all() {
        inbox.retain(|f| filter.runs(inbox_machine(&runs_dir, &inbox_dir, f).as_deref()));
    }
    let (dir, files) = (INBOX_DIR, inbox);
    let text = format == Format::Text;
    if text && files.is_empty() {
        println!("{dir}/: nothing queued");
    } else if text {
        println!("{dir}/:");
    }
    {
        let mut items = Vec::new();
        for file in files {
            let path = project.decree_dir.join(dir).join(&file);
            let target = Message::read(&path)
                .map_err(|e| vec![(0, e.to_string())])
                .and_then(|m| {
                    if m.frontmatter.contains_key("to") {
                        return Ok(Target::Reply(m.text("to").unwrap_or_default().to_string()));
                    }
                    message::validate(&m, &project.machines, &project.machine_ids)
                        .map(Target::Machine)
                });
            let item = match &target {
                Ok(Target::Machine(machine)) => {
                    serde_json::json!({ "file": file, "valid": true, "machine": machine })
                }
                Ok(Target::Reply(to)) => {
                    serde_json::json!({ "file": file, "valid": true, "to": to })
                }
                Err(_) => serde_json::json!({ "file": file, "valid": false }),
            };
            match target {
                Ok(Target::Machine(machine)) if text => println!("  {file:<24} → {machine}"),
                Ok(Target::Reply(to)) if text => println!("  {file:<24} → reply to {to}"),
                Ok(_) => {}
                Err(_) => {
                    if text {
                        println!("  {file:<24} → invalid");
                    }
                    project.check_file(dir, &file, "M2", None, &mut problems)?;
                }
            }
            items.push(item);
        }
        document.insert(dir.to_string(), items.into());
    }
    if format == Format::Json {
        print_json(&document.into())?;
    }
    for problem in &problems {
        eprintln!("{problem}");
    }
    match problems.len() {
        0 => Ok(()),
        n => Err(DecreeError::Other(format!(
            "{n} error(s); nothing would run"
        ))),
    }
}

/// What a queued message would do: start a run of a machine, or reply to a waiting run.
enum Target {
    Machine(String),
    Reply(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_last_from_skips_transitions_without_from() {
        let events: Vec<_> = [
            json!({"type": "transition", "from": null, "to": "a"}),
            json!({"type": "transition", "from": "a", "to": "b"}),
            json!({"type": "script", "state": "b"}),
            json!({"type": "transition", "from": "b", "to": "failed"}),
            json!({"type": "run_finished", "state": "failed"}),
        ]
        .into_iter()
        .map(|v| v.as_object().cloned().unwrap())
        .collect();
        assert_eq!(last_from(&events).as_deref(), Some("b"));
        assert_eq!(last_from(&events[..1]), None);
    }
}
