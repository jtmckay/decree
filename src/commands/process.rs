//! `decree process` (docs/reference/messages.md, docs/reference/cli.md): validate the machines and pending migrations,
//! mark runs a crash left behind `interrupted` and continue `pending` runs, then repeat
//! until nothing is left: claim the next `inbox/` message and run it through its machine,
//! or start the next migration once the inbox is empty. Never continues an `interrupted`
//! run. A run that is not a migration and ends in `failed` is reported and the pass goes on;
//! a migration that ends in `failed` stops it, as does a `failed`, `interrupted` or `waiting`
//! migration before it starts. Ends by printing every waiting run, then every run that failed
//! in the pass, and exits 1 if there was one.
//!
//! `--retry [<id>] [--state <s>]` first makes one `interrupted` or finished run `pending`
//! again: run `<id>`, or the migration that blocks the queue. It appends a `transition`
//! event with `source: "retry"` and mirrors `state`; the pipeline then continues it,
//! re-running root `onentry` and the `onentry` of every ancestor of `<s>` and of `<s>`
//! (docs/reference/runs.md, step 1).
//!
//! A queued message with `to:` is a reply: it is delivered to its waiting run instead,
//! or, failing a check, becomes a failed `invalid_message` run. Each pass also delivers
//! `timeout` deadlines that have passed (docs/reference/messages.md, Replies).
//!
//! Before each run it reads the machines and `.env` files again (`Pipeline::refresh`), so an
//! edit applies to the next run without a restart.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::json;

use crate::cli::Format;
use crate::commands::check::{self, md_files, Project};
use crate::commands::print_json;
use crate::commands::report::{self, Reporter};
use crate::error::DecreeError;
use crate::events::{
    current_state, first_text, is_transition, is_type, run_attempt, strings, text, waiting_child,
    Event, EventLog,
};
use crate::interpreter::child::continue_run;
use crate::interpreter::recover::{self, mirror_state, RunStatus};
use crate::interpreter::{Context, Interpreter, InterpreterError, Outcome, RunInput};
use crate::layout::MESSAGE_FILE;
use crate::layout::{self, DECREE_DIR, INBOX_DIR, MIGRATIONS_DIR};
use crate::machine::FAILED;
use crate::message::{self, is_valid_id, Claim, LockState, Message, RunLock};
use crate::reply::{self, Delivery};
use crate::runtime::{self};

/// What `--retry` continues: run `id`, or with `None` the migration that blocks the queue.
pub struct Retry {
    pub id: Option<String>,
    /// The atomic state to continue in, from `--state`.
    pub state: Option<String>,
}

impl Retry {
    /// From `--retry [<id>]` and `--state`: `--retry` alone names no run, the blocking
    /// migration.
    pub fn from_args(retry: Option<String>, state: Option<String>) -> Option<Retry> {
        retry.map(|id| Retry {
            id: Some(id).filter(|id| !id.is_empty()),
            state,
        })
    }
}

/// Run `decree process [--dry-run | --retry [<id>] [--state <s>]] [--quiet]`. Unless
/// `quiet`, each run's output is printed as it goes (`commands::report`).
pub fn run(
    project_root: &Path,
    dry_run: bool,
    retry: Option<Retry>,
    format: Format,
    quiet: bool,
) -> Result<(), DecreeError> {
    let project = Project::load(project_root)?;
    if dry_run {
        return run_dry(&project, format);
    }
    let shutdown = Arc::new(AtomicBool::new(false));
    runtime::register_signals(&shutdown)?;
    let observer_ctx = context(project_root, &project, Arc::clone(&shutdown));
    let mut pipeline = Pipeline::new(project_root, project, shutdown, "")?;
    if let Some(retry) = retry {
        pipeline.retry(&retry)?;
    }
    let result = reporting(&mut pipeline, &observer_ctx, quiet, |pipeline| {
        pipeline.recover().and_then(|()| pipeline.drain())
    });
    print_waiting(&pipeline.ctx)?;
    if result.is_ok() {
        if let Some(why) = held_back(&pipeline.ctx, &pipeline.project)? {
            println!("Held back: {why}");
        }
    }
    let failed = pipeline.take_failed();
    print_failed(&failed);
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
    /// A migration is `failed` or `interrupted`: later migrations wait for `decree process --retry`.
    Blocked(String),
    /// SIGINT or SIGTERM: the current run is `interrupted`.
    Interrupted,
    /// `.decree/HOLD` exists: nothing runs until it is gone. Holds what to print.
    Held(String),
    /// Anything else: an I/O error, or a run that cannot be stepped.
    Error(DecreeError),
}

impl Stop {
    pub(crate) fn into_error(self) -> DecreeError {
        match self {
            Stop::Blocked(message) | Stop::Held(message) => DecreeError::Other(message),
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

/// The steps `process` and `daemon` share (docs/reference/cli.md): one pipeline, so the daemon has
/// no copy of its own. Each step runs to the end of one run before it returns.
pub(crate) struct Pipeline {
    ctx: Context,
    project: Project,
    /// Inbox files another process claimed first, or is claiming now.
    lost: HashSet<String>,
    /// Put before each failed run's report: `process` prints none, `daemon` its name.
    prefix: &'static str,
    /// Runs that are not migrations and ended in `failed` since the last `take_failed`.
    failed: Vec<String>,
    /// Told when each run starts and stops, unless `--quiet`.
    pub(crate) report: Option<Reporter>,
    /// The last machine or `.env` problem `refresh` printed, so each is printed once.
    machine_error: Option<String>,
    env_error: Option<String>,
}

impl Pipeline {
    /// Validate every machine and pending migration (docs/reference/messages.md, Migrations, rule 6), and
    /// set up the context runs are stepped in. Nothing runs if anything is invalid.
    pub(crate) fn new(
        project_root: &Path,
        project: Project,
        shutdown: Arc<AtomicBool>,
        prefix: &'static str,
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
        validate_migrations(&project)?;
        let mut ctx = context(project_root, &project, shutdown);
        ctx.env = check::load_env(&project)?;
        Ok(Pipeline {
            ctx,
            project,
            lost: HashSet::new(),
            prefix,
            failed: Vec::new(),
            report: None,
            machine_error: None,
            env_error: None,
        })
    }

    /// `--retry`: make run `id`, or the migration that blocks the queue, `pending` again.
    pub(crate) fn retry(&self, retry: &Retry) -> Result<(), DecreeError> {
        retry_run(
            &self.ctx,
            &self.project,
            retry.id.as_deref(),
            retry.state.as_deref(),
        )
    }

    /// Read the machines and the `.env` files again, so edits apply to the next run without
    /// a restart. A machine with errors keeps every machine as last loaded, and a malformed
    /// `.env` file the variables last read; each problem is printed once, until it changes.
    /// A run already going keeps its machine until it stops.
    pub(crate) fn refresh(&mut self) {
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
            self.project = project;
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

    /// Stop for a signal, or for `.decree/HOLD`, before a run starts or continues.
    fn may_run(&self) -> Result<(), Stop> {
        if self.shutdown() {
            return Err(Stop::Interrupted);
        }
        match held(&self.ctx.project_root) {
            Some(message) => Err(Stop::Held(message)),
            None => Ok(()),
        }
    }

    /// Report run `id`, not a migration, that ended in `failed`, with its `--retry` command.
    /// Inbox messages are independent, so the pass goes on (docs/reference/messages.md, Lifecycle).
    fn fail(&mut self, id: &str, file: Option<&str>) {
        let file = file.map(|f| format!(" ({f})")).unwrap_or_default();
        report::notice(format!(
            "{}run {id}{file} ended in `{FAILED}`; see .decree/runs/{id}/, \
             then continue it with `decree process --retry {id}`",
            self.prefix
        ));
        self.failed.push(id.to_string());
    }

    /// The runs that ended in `failed` since the last call, in the order they failed.
    pub(crate) fn take_failed(&mut self) -> Vec<String> {
        std::mem::take(&mut self.failed)
    }

    /// At start: mark runs a crash left behind `interrupted` (never continued), then
    /// continue `pending` runs in `id` order (docs/reference/messages.md, Run status).
    pub(crate) fn recover(&mut self) -> Result<(), Stop> {
        let recovery = recover::recover(&self.ctx)?;
        for (id, state) in &recovery.crashed {
            eprintln!(
                "run {id} was interrupted in `{state}` (crash); \
                 continue it with `decree process --retry {id}`"
            );
        }
        self.continue_runs(&recovery.pending)
    }

    /// `process`: repeat until nothing is left: deliver timeouts, continue `pending` runs
    /// (`decree process --retry` may have made more), claim the next inbox message, and start the
    /// next migration once the inbox is empty. A failed inbox run is reported and the loop goes
    /// on; a failed migration stops it.
    pub(crate) fn drain(&mut self) -> Result<(), Stop> {
        loop {
            self.refresh();
            self.deliver_timeouts()?;
            let pending = self.pending()?;
            self.continue_runs(&pending)?;
            if self.next_inbox()? {
                continue;
            }
            if !self.next_migration()? {
                return Ok(());
            }
        }
    }

    /// Continue `ids` in order; stops at the first migration that ends in `failed`.
    fn continue_runs(&mut self, ids: &[String]) -> Result<(), Stop> {
        for id in ids {
            self.continue_one(id)?;
        }
        Ok(())
    }

    /// Every `pending` run, in `id` order: after `decree process --retry`, or a delivery another
    /// process made. A run another process holds is `active`, not `pending`.
    pub(crate) fn pending(&self) -> Result<Vec<String>, Stop> {
        let mut pending = Vec::new();
        for id in message::run_ids(&self.ctx.runs_dir())? {
            if self.ctx.run_finished(&id)?.is_none()
                && self.ctx.status_of(&id)?.0 == RunStatus::Pending
            {
                pending.push(id);
            }
        }
        Ok(pending)
    }

    /// Continue `pending` run `id`. A run another process holds now is skipped. A migration
    /// that ends in `failed` blocks the queue; any other run is reported and the pass goes on.
    pub(crate) fn continue_one(&mut self, id: &str) -> Result<(), Stop> {
        self.may_run()?;
        let outcome = match reported(self.report.as_ref(), &self.ctx, &self.project, id, || {
            continue_run(&self.ctx, id)
        }) {
            Ok(outcome) => outcome,
            Err(InterpreterError::Active(_)) => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        match outcome {
            Outcome::Finished(state) if state == FAILED => {
                let events = self.ctx.events(id)?;
                if first_text(&events, "trigger") == Some("migration") {
                    return Err(blocked(
                        &format!("{id}.md"),
                        &format!("ended in `{FAILED}`"),
                    ));
                }
                self.fail(id, None);
                Ok(())
            }
            ref outcome if interrupted(outcome) => Err(Stop::Interrupted),
            _ => Ok(()),
        }
    }

    /// Deliver every `timeout` deadline that has passed, and continue those runs.
    pub(crate) fn deliver_timeouts(&mut self) -> Result<(), Stop> {
        self.may_run()?;
        let timed_out = reply::deliver_timeouts(&self.ctx, chrono::Utc::now())?;
        self.continue_runs(&timed_out)
    }

    /// Take the next `inbox/` file in filename order: deliver a reply and continue its
    /// run, or claim a message and run it (docs/reference/messages.md, Lifecycle). Returns `false` when the
    /// inbox is empty.
    pub(crate) fn next_inbox(&mut self) -> Result<bool, Stop> {
        self.may_run()?;
        self.refresh();
        let decree_dir = self.ctx.project_root.join(DECREE_DIR);
        let inbox = decree_dir.join(INBOX_DIR);
        let next = md_files(&inbox)?
            .into_iter()
            .find(|f| !self.lost.contains(f));
        let Some(file) = next else {
            return Ok(false);
        };
        let mut problem = None;
        if let Ok(m) = Message::read(&inbox.join(&file)) {
            if m.frontmatter.contains_key("to") {
                let (to, event) = (m.text("to"), m.text("event"));
                match reply::deliver(&self.ctx, &inbox, &file, to, event)? {
                    Delivery::Delivered(run_id) => {
                        self.continue_one(&run_id)?;
                        return Ok(true);
                    }
                    Delivery::Lost => {
                        self.lost.insert(file);
                        return Ok(true);
                    }
                    Delivery::Rejected(reason) => problem = Some(reason),
                }
            }
        }
        let Some(claim) = message::claim(&decree_dir, &file)? else {
            self.lost.insert(file);
            return Ok(true);
        };
        let id = claim.id.clone();
        let report = self.report.as_ref();
        match reported(report, &self.ctx, &self.project, &id, || {
            run_claim(&self.ctx, &self.project, claim, problem)
        })? {
            Outcome::Finished(state) if state == FAILED => {
                self.fail(&id, Some(&file));
                Ok(true)
            }
            ref outcome if interrupted(outcome) => Err(Stop::Interrupted),
            _ => Ok(true),
        }
    }

    /// Start or continue the first pending migration (docs/reference/messages.md, Migrations). Returns
    /// whether it finished, so the next may start; `false` when there is none, or it
    /// waits.
    pub(crate) fn next_migration(&mut self) -> Result<bool, Stop> {
        self.may_run()?;
        self.refresh();
        match self.project.pending_migrations()?.into_iter().next() {
            Some(migration) => {
                step_migration(&self.ctx, &self.project, &migration, self.report.as_ref())
            }
            None => Ok(false),
        }
    }
}

/// Run `run`, the steps of run `id`, telling `report` when it starts and stops.
fn reported<E>(
    report: Option<&Reporter>,
    ctx: &Context,
    project: &Project,
    id: &str,
    run: impl FnOnce() -> Result<Outcome, E>,
) -> Result<Outcome, E> {
    let Some(report) = report else {
        return run();
    };
    report.start(id, &ctx.runs_dir().join(id), queued(ctx, project, id));
    let outcome = run();
    report.end(id, outcome.as_ref().ok());
    outcome
}

/// How many inbox messages and pending migrations wait, besides run `id`.
fn queued(ctx: &Context, project: &Project, id: &str) -> usize {
    let inbox = md_files(&ctx.project_root.join(DECREE_DIR).join(INBOX_DIR)).map_or(0, |f| f.len());
    let migrations = project.pending_migrations().unwrap_or_default();
    let others = migrations
        .iter()
        .filter(|m| m.strip_suffix(".md").unwrap_or(m) != id)
        .count();
    inbox + others
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
    }
}

/// Print every run that ended in `failed` in this pass, with the command that continues it.
fn print_failed(failed: &[String]) {
    if failed.is_empty() {
        return;
    }
    eprintln!("{} run(s) ended in `{FAILED}` in this pass:", failed.len());
    for id in failed {
        eprintln!("  run {id}: decree process --retry {id}");
    }
}

/// docs/reference/messages.md, Migrations, rule 6: parse every pending migration (frontmatter, machine,
/// `params` against the machine's `data`) before any runs. Prints every error.
fn validate_migrations(project: &Project) -> Result<(), DecreeError> {
    let mut invalid = 0;
    for name in project.pending_migrations()? {
        let mut problems = Vec::new();
        project.check_file(MIGRATIONS_DIR, &name, "M1", false, &mut problems)?;
        if !problems.is_empty() {
            invalid += 1;
            for problem in problems {
                eprintln!("{problem}");
            }
        }
    }
    match invalid {
        0 => Ok(()),
        n => Err(DecreeError::Other(format!(
            "{n} migration(s) are invalid; nothing was processed."
        ))),
    }
}

/// Lifecycle steps 2 to 4 for a claimed inbox message: set `id` and `trigger` if missing,
/// validate, and start the run, or fail it with `invalid_message`. A reply (`to:`) never
/// starts a run: it was not delivered, for `reply_problem`.
fn run_claim(
    ctx: &Context,
    project: &Project,
    claim: Claim,
    reply_problem: Option<String>,
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
        Err(e) => return reject(&run_dir, &id, "", "inbox", &file, &e.to_string(), false),
    };
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
        return reject(&run_dir, &id, &name, &trigger, &file, &problem, true);
    }
    if message.frontmatter.contains_key("to") {
        let reason = reply_problem.map_or_else(
            || "a reply (`to:`) was not delivered".to_string(),
            |p| format!("reply not delivered: {p}"),
        );
        return reject(&run_dir, &id, &name, &trigger, &file, &reason, true);
    }
    start(ctx, project, &id, &trigger, &file, &message)
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
) -> Result<Outcome, DecreeError> {
    let run_dir = ctx.runs_dir().join(id);
    let name = match message::validate(message, &project.machines, &project.machine_ids) {
        Ok(name) => name,
        Err(errors) => {
            let reason = errors
                .iter()
                .map(|(line, msg)| format!("line {line}: {msg}"))
                .collect::<Vec<_>>()
                .join("; ");
            let name = message.machine().unwrap_or_default();
            return reject(&run_dir, id, name, trigger, file, &reason, true);
        }
    };
    // `Pipeline::new` refuses a project with a machine that does not load.
    let machine = project
        .machines
        .get(&name)
        .ok_or_else(|| DecreeError::Other(format!("machine `{name}` does not load")))?;
    let input = RunInput::new(message, Some(file.to_string()));
    let executor = ctx.executor(machine, id, trigger, &input.params, message.text("parent"))?;
    Ok(Interpreter::new(ctx, machine, executor, input)?.start()?)
}

/// Fail a run at its claim with `invalid_message` and `reason`: one `transition` event to
/// `failed`, and `state: failed` mirrored unless the frontmatter did not parse.
fn reject(
    run_dir: &Path,
    id: &str,
    machine: &str,
    trigger: &str,
    file: &str,
    reason: &str,
    mirror: bool,
) -> Result<Outcome, DecreeError> {
    let mut events = EventLog::open(run_dir, id, machine, trigger)?;
    recover::reject(&mut events, run_dir, file, reason, mirror)?;
    report::notice(format!("{file}: invalid message: {reason}"));
    Ok(Outcome::Finished(FAILED.to_string()))
}

/// docs/reference/messages.md, Migrations: start migration `file`, or continue or report the run it has.
/// Returns whether later migrations may start once this one is in `processed.md`; `false`
/// means it waits.
fn step_migration(
    ctx: &Context,
    project: &Project,
    file: &str,
    report: Option<&Reporter>,
) -> Result<bool, Stop> {
    let id = file.strip_suffix(".md").unwrap_or(file);
    let run_dir = ctx.runs_dir().join(id);
    let outcome = if run_dir.exists() {
        // Rule 4: the migration has a run already.
        let events = ctx.events(id)?;
        let alive = matches!(message::lock_state(&run_dir)?, LockState::Live(_));
        let status = ctx
            .run_machine(&events)
            .map_or(RunStatus::Interrupted, |m| ctx.status(m, &events, alive));
        match status {
            RunStatus::Pending => {
                match reported(report, ctx, project, id, || continue_run(ctx, id)) {
                    Ok(outcome) => outcome,
                    Err(InterpreterError::Active(_)) => return Ok(false),
                    Err(e) => return Err(e.into()),
                }
            }
            RunStatus::Waiting => {
                // A child that was interrupted never continues its parent by itself.
                return match stalled_child(ctx, &events)? {
                    Some((child, state)) => Err(blocked(
                        file,
                        &format!("waits for child run {child}, interrupted in `{state}`"),
                    )),
                    None => Ok(false),
                };
            }
            RunStatus::Active => return Ok(false),
            RunStatus::Finished | RunStatus::Interrupted => {
                let state = current_state(&events).unwrap_or("no state");
                return Err(blocked(
                    file,
                    &format!("is {} in `{state}`", status.as_str()),
                ));
            }
        }
    } else {
        reported(report, ctx, project, id, || {
            start_migration(ctx, project, file, id)
        })?
    };
    match outcome {
        Outcome::Finished(state) if state == FAILED => {
            Err(blocked(file, &format!("ended in `{FAILED}`")))
        }
        // Rule 5 wrote the ledger line; the loop moves on to the next migration.
        Outcome::Finished(_) => Ok(true),
        ref outcome if interrupted(outcome) => Err(Stop::Interrupted),
        // It waits for a reply, or for a child run.
        _ => Ok(false),
    }
}

/// Whether a signal stopped the run, or the child run it waits for, child after child.
fn interrupted(outcome: &Outcome) -> bool {
    match outcome {
        Outcome::Interrupted(_) => true,
        Outcome::Child { outcome, .. } => interrupted(outcome),
        _ => false,
    }
}

/// Rule 1: create `runs/<id>/` and copy the migration to `runs/<id>/message.md`, adding
/// `id` and `trigger: migration`, then start the run.
fn start_migration(
    ctx: &Context,
    project: &Project,
    file: &str,
    id: &str,
) -> Result<Outcome, DecreeError> {
    let runs = ctx.runs_dir();
    std::fs::create_dir_all(&runs)?;
    std::fs::create_dir(runs.join(id))?;
    let source = ctx
        .project_root
        .join(DECREE_DIR)
        .join(layout::MIGRATIONS_DIR)
        .join(file);
    let mut message = Message::read(&source)?;
    message.set("id", id);
    message.set("trigger", "migration");
    message.write(&runs.join(id).join(MESSAGE_FILE))?;
    start(ctx, project, id, "migration", file, &message)
}

/// Rule 4: a migration that blocks every later one, and the command that continues it.
fn blocked(file: &str, what: &str) -> Stop {
    Stop::Blocked(format!(
        "migration {file} {what}; later migrations are blocked. \
         Fix the cause, then run `decree process --retry`."
    ))
}

/// The migration that blocks the queue, and the run `decree process --retry` continues for it.
pub(crate) struct Blocker {
    /// The migration's file name.
    pub(crate) file: String,
    /// The migration's run, or the interrupted child run it waits for.
    pub(crate) run: String,
    /// The state `run` is in.
    pub(crate) state: String,
    pub(crate) status: RunStatus,
}

impl Blocker {
    /// Why the migration blocks: `is interrupted in `work``, or `waits for child run <id>,
    /// interrupted in `work``.
    pub(crate) fn what(&self) -> String {
        let (status, state) = (self.status.as_str(), &self.state);
        if self.file.strip_suffix(".md") == Some(self.run.as_str()) {
            format!("is {status} in `{state}`")
        } else {
            format!("waits for child run {}, {status} in `{state}`", self.run)
        }
    }
}

/// docs/reference/messages.md, Migrations, rule 4: the migration that blocks the queue, the
/// earliest pending one whose run is `failed` or `interrupted`, or `waiting` for a child run
/// that is `interrupted`. A finished run of a pending migration blocks too, as `process`
/// reports it, so `--retry` can continue it.
pub(crate) fn blocking_migration(
    ctx: &Context,
    project: &Project,
) -> Result<Option<Blocker>, DecreeError> {
    for file in project.pending_migrations()? {
        let id = file.strip_suffix(".md").unwrap_or(&file).to_string();
        if !ctx.runs_dir().join(&id).is_dir() {
            continue;
        }
        let (status, events) = ctx.status_of(&id)?;
        let (run, state, status) = match status {
            RunStatus::Finished | RunStatus::Interrupted => {
                let state = current_state(&events).unwrap_or("no state").to_string();
                (id, state, status)
            }
            RunStatus::Waiting => match stalled_child(ctx, &events)? {
                Some((child, state)) => (child, state, RunStatus::Interrupted),
                None => continue,
            },
            RunStatus::Active | RunStatus::Pending => continue,
        };
        return Ok(Some(Blocker {
            file,
            run,
            state,
            status,
        }));
    }
    Ok(None)
}

/// Why the first pending migration has not finished, when nothing blocks it
/// (`blocking_migration`): it, or the child run it waits for, waits for a reply or is
/// active in another process, or it waits for `inbox/` to empty. `None` when no migration
/// is pending, or the pass would start or continue it.
pub(crate) fn held_back(ctx: &Context, project: &Project) -> Result<Option<String>, DecreeError> {
    let pending = project.pending_migrations()?;
    let Some(file) = pending.first() else {
        return Ok(None);
    };
    let behind = match pending.len() - 1 {
        0 => String::new(),
        n => format!("; {n} later migration(s) wait behind it"),
    };
    let id = file.strip_suffix(".md").unwrap_or(file);
    if !ctx.runs_dir().join(id).is_dir() {
        let inbox = md_files(&ctx.project_root.join(DECREE_DIR).join(INBOX_DIR))?.len();
        return Ok((inbox > 0).then(|| {
            format!("migration {file} starts once the {inbox} file(s) in inbox/ have run{behind}")
        }));
    }
    let mut run = id.to_string();
    let mut seen = HashSet::new();
    let reason = loop {
        let (status, events) = ctx.status_of(&run)?;
        match (status, waiting_child(&events)) {
            (RunStatus::Waiting, Some(child)) if seen.insert(child.to_string()) => {
                run = child.to_string();
            }
            (RunStatus::Waiting, _) => {
                let Some(last) = events.last() else {
                    return Ok(None);
                };
                let wait_id = text(last, "wait_id").unwrap_or_default();
                break format!(
                    "waits for a reply: `decree event {wait_id} <{}>`",
                    strings(last, "options").join("|")
                );
            }
            (RunStatus::Active, _) => {
                let pid = match message::lock_state(&ctx.runs_dir().join(&run))? {
                    LockState::Live(pid) => format!(" (pid {pid})"),
                    _ => String::new(),
                };
                break format!("is active in another process{pid}");
            }
            // Blocked, or the pass continues it.
            _ => return Ok(None),
        }
    };
    Ok(Some(if run == id {
        format!("migration {file} {reason}{behind}")
    } else {
        format!("migration {file} waits for child run {run}, which {reason}{behind}")
    }))
}

/// The `interrupted` run, and its state, at the end of the child runs a run with `events`
/// waits for, child after child; `None` if the last one is not `interrupted`.
fn stalled_child(ctx: &Context, events: &[Event]) -> Result<Option<(String, String)>, DecreeError> {
    let mut seen = HashSet::new();
    let mut next = waiting_child(events).map(String::from);
    while let Some(id) = next {
        if !seen.insert(id.clone()) || !ctx.runs_dir().join(&id).is_dir() {
            break;
        }
        let (status, events) = ctx.status_of(&id)?;
        match status {
            RunStatus::Interrupted => {
                let state = current_state(&events).unwrap_or("no state").to_string();
                return Ok(Some((id, state)));
            }
            RunStatus::Waiting => next = waiting_child(&events).map(String::from),
            _ => break,
        }
    }
    Ok(None)
}

/// `--retry` (docs/reference/cli.md): make an `interrupted` or finished run `pending` again,
/// run `id` or else the blocking migration, in `state` or by default the state it was in
/// (for a finished run, the state it last left). The pipeline then continues it.
fn retry_run(
    ctx: &Context,
    project: &Project,
    id: Option<&str>,
    state: Option<&str>,
) -> Result<(), DecreeError> {
    let id = match id {
        Some(id) => id.to_string(),
        None => {
            blocking_migration(ctx, project)?
                .ok_or_else(|| {
                    DecreeError::Other(
                        "nothing to retry: no migration is failed or interrupted".to_string(),
                    )
                })?
                .run
        }
    };
    let id = id.as_str();
    let run_dir = ctx.runs_dir().join(id);
    if !is_valid_id(id) || !run_dir.is_dir() {
        return Err(DecreeError::MessageNotFound(id.to_string()));
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
                "run {id} is waiting for a reply, not interrupted; answer it with `decree event`"
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
    release_hold(&ctx.project_root, id)?;
    Ok(())
}

/// What to print while `.decree/HOLD` exists, or `None` (docs/reference/cli.md, Hold).
pub(crate) fn held(project_root: &Path) -> Option<String> {
    let path = project_root.join(DECREE_DIR).join(layout::HOLD_FILE);
    let text = std::fs::read_to_string(&path).ok().or_else(|| {
        // A HOLD that cannot be read (a folder, no permission) holds all the same.
        path.exists().then(String::new)
    })?;
    Some(format!(
        "nothing runs while .decree/HOLD exists; delete it, or retry the run it names:\n{}",
        text.trim_end()
    ))
}

/// Remove `.decree/HOLD` if its first line is run `id`, which `--retry` continues.
fn release_hold(project_root: &Path, id: &str) -> Result<(), DecreeError> {
    let path = project_root.join(DECREE_DIR).join(layout::HOLD_FILE);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    if text.lines().next().map(str::trim) == Some(id) {
        std::fs::remove_file(&path)?;
        println!("removed .decree/HOLD, which run {id} wrote");
    }
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

/// Print every waiting run: its question (the state's `description`), its wait id and
/// options, and a `decree event` command for each option (docs/reference/messages.md, Replies).
fn print_waiting(ctx: &Context) -> Result<(), DecreeError> {
    for id in message::run_ids(&ctx.runs_dir())? {
        if ctx.run_finished(&id)?.is_some() {
            continue;
        }
        let events = ctx.events(&id)?;
        let Some(last) = events.last().filter(|e| is_type(e, "waiting")) else {
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
            println!("  decree event {wait_id} {option}");
        }
    }
    Ok(())
}

/// `decree process --dry-run`: list what would run, and run nothing. Pending migrations
/// in order with their machine, then queued inbox messages; invalid ones with their errors.
fn run_dry(project: &Project, format: Format) -> Result<(), DecreeError> {
    let mut problems = project.problems.clone();
    let mut document = serde_json::Map::new();
    let lists = [
        (MIGRATIONS_DIR, project.pending_migrations()?),
        (INBOX_DIR, md_files(&project.decree_dir.join(INBOX_DIR))?),
    ];
    for (dir, files) in lists {
        let text = format == Format::Text;
        if text && files.is_empty() {
            println!("{dir}/: nothing queued");
            continue;
        }
        if text {
            println!("{dir}/:");
        }
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
                    let rule = if dir == MIGRATIONS_DIR { "M1" } else { "M2" };
                    project.check_file(dir, &file, rule, false, &mut problems)?;
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
