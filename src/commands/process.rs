//! `decree process` (spec sections 4 and 8): validate the machines and pending migrations,
//! mark runs a crash left behind `interrupted` and continue `pending` runs, then repeat
//! until nothing is left: claim the next `inbox/` message and run it through its machine,
//! or start the next migration once the inbox is empty. Never continues an `interrupted`
//! run. Stops at the first run that ends in `failed`, and before a `failed`, `interrupted`
//! or `waiting` migration. Ends by printing every waiting run.
//!
//! A queued message with `to:` is a reply: it is delivered to its waiting run instead,
//! or, failing a check, becomes a failed `invalid_message` run. Each pass also delivers
//! `timeout_s` deadlines that have passed (section 4, Replies).

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::Value;

use crate::commands::check::{md_files, Project};
use crate::config::{self, DECREE_DIR, INBOX_DIR, MIGRATIONS_DIR};
use crate::error::DecreeError;
use crate::interpreter::{
    self, continue_run, read_events, Context, Interpreter, InterpreterError, Outcome, RunInput,
    RunStatus,
};
use crate::machine::FAILED;
use crate::message::{self, Claim, LockState, Message};
use crate::reply::{self, Delivery};
use crate::runtime::{self, EventLog, MESSAGE_FILE};

/// Run `decree process [--dry-run]`.
pub fn run(project_root: &Path, dry_run: bool) -> Result<(), DecreeError> {
    let project = Project::load(project_root)?;
    if dry_run {
        return run_dry(&project);
    }
    let shutdown = Arc::new(AtomicBool::new(false));
    runtime::register_signals(&shutdown)?;
    let mut pipeline = Pipeline::new(project_root, &project, shutdown)?;
    let result = pipeline.recover().and_then(|()| pipeline.drain());
    pipeline.print_waiting()?;
    result.map_err(Stop::into_error)
}

/// Why a pass of the pipeline stopped short.
#[derive(Debug)]
pub(crate) enum Stop {
    /// A run ended in `failed`. `process` stops; `daemon` reports it and goes on.
    Failed(String),
    /// A migration is `failed` or `interrupted`: later migrations wait for `decree retry`.
    Blocked(String),
    /// SIGINT or SIGTERM: the current run is `interrupted`.
    Interrupted,
    /// Anything else: an I/O error, or a run that cannot be stepped.
    Error(DecreeError),
}

impl Stop {
    pub(crate) fn into_error(self) -> DecreeError {
        match self {
            Stop::Failed(message) | Stop::Blocked(message) => DecreeError::Other(message),
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

/// The steps `process` and `daemon` share (section 8): one pipeline, so the daemon has
/// no copy of its own. Each step runs to the end of one run before it returns.
pub(crate) struct Pipeline<'a> {
    ctx: Context<'a>,
    project: &'a Project,
    /// Inbox files another process claimed first, or is claiming now.
    lost: HashSet<String>,
}

impl<'a> Pipeline<'a> {
    /// Validate every machine and pending migration (section 4, Migrations, rule 6), and
    /// set up the context runs are stepped in. Nothing runs if anything is invalid.
    pub(crate) fn new(
        project_root: &Path,
        project: &'a Project,
        shutdown: Arc<AtomicBool>,
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
        validate_migrations(project)?;
        let ctx = context(project_root, project, shutdown);
        Ok(Pipeline {
            ctx,
            project,
            lost: HashSet::new(),
        })
    }

    fn shutdown(&self) -> bool {
        self.ctx.shutdown.load(Ordering::Relaxed)
    }

    /// At start: mark runs a crash left behind `interrupted` (never continued), then
    /// continue `pending` runs in `id` order (section 4, Run status).
    pub(crate) fn recover(&mut self) -> Result<(), Stop> {
        let recovery = interpreter::recover(&self.ctx).map_err(other)?;
        for (id, state) in &recovery.crashed {
            eprintln!(
                "run {id} was interrupted in `{state}` (crash); \
                 continue it with `decree retry {id}`"
            );
        }
        self.continue_runs(&recovery.pending)
    }

    /// `process`: repeat until nothing is left: deliver timeouts, continue `pending` runs
    /// (`decree retry` may have made more), claim the next inbox message, and start the
    /// next migration once the inbox is empty. Stops at the first run that ends in `failed`.
    pub(crate) fn drain(&mut self) -> Result<(), Stop> {
        loop {
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

    /// Continue `ids` in order; stops at the first that ends in `failed`.
    fn continue_runs(&self, ids: &[String]) -> Result<(), Stop> {
        for id in ids {
            self.continue_one(id)?;
        }
        Ok(())
    }

    /// Every `pending` run, in `id` order: after `decree retry`, or a delivery another
    /// process made. A run another process holds is `active`, not `pending`.
    pub(crate) fn pending(&self) -> Result<Vec<String>, Stop> {
        let mut pending = Vec::new();
        for id in run_ids(&self.ctx.runs_dir())? {
            let run_dir = self.ctx.runs_dir().join(&id);
            let events = read_events(&run_dir)?;
            let Some(machine) = run_machine(&self.ctx, &events) else {
                continue;
            };
            let alive = matches!(message::lock_state(&run_dir)?, LockState::Live(_));
            if self.ctx.status(machine, &events, alive) == RunStatus::Pending {
                pending.push(id);
            }
        }
        Ok(pending)
    }

    /// Continue `pending` run `id`. A run another process holds now is skipped.
    pub(crate) fn continue_one(&self, id: &str) -> Result<(), Stop> {
        if self.shutdown() {
            return Err(Stop::Interrupted);
        }
        let outcome = match continue_run(&self.ctx, id) {
            Ok(outcome) => outcome,
            Err(InterpreterError::Active(_)) => return Ok(()),
            Err(e) => return Err(other(e).into()),
        };
        match outcome {
            Outcome::Finished(state) if state == FAILED => {
                let run_dir = self.ctx.runs_dir().join(id);
                let migration = read_events(&run_dir)?
                    .first()
                    .is_some_and(|e| e.get("trigger").and_then(Value::as_str) == Some("migration"));
                Err(if migration {
                    blocked(&format!("{id}.md"), id, &format!("ended in `{FAILED}`"))
                } else {
                    Stop::Failed(format!(
                        "run {id} ended in `{FAILED}`; see .decree/runs/{id}/"
                    ))
                })
            }
            Outcome::Interrupted(_) => Err(Stop::Interrupted),
            _ => Ok(()),
        }
    }

    /// Deliver every `timeout_s` deadline that has passed, and continue those runs.
    pub(crate) fn deliver_timeouts(&self) -> Result<(), Stop> {
        let timed_out =
            reply::deliver_timeouts(&self.ctx.runs_dir(), self.ctx.machines, chrono::Utc::now())
                .map_err(other)?;
        self.continue_runs(&timed_out)
    }

    /// Take the next `inbox/` file in filename order: deliver a reply and continue its
    /// run, or claim a message and run it (section 4, Lifecycle). Returns `false` when the
    /// inbox is empty.
    pub(crate) fn next_inbox(&mut self) -> Result<bool, Stop> {
        if self.shutdown() {
            return Err(Stop::Interrupted);
        }
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
                let runs = self.ctx.runs_dir();
                match reply::deliver(&runs, self.ctx.machines, &inbox, &file, to, event)
                    .map_err(other)?
                {
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
        let Some(claim) = message::claim(&decree_dir, &file).map_err(other)? else {
            self.lost.insert(file);
            return Ok(true);
        };
        let id = claim.id.clone();
        match run_claim(&self.ctx, self.project, claim, problem)? {
            Outcome::Finished(state) if state == FAILED => Err(Stop::Failed(format!(
                "run {id} ({file}) ended in `{FAILED}`; see .decree/runs/{id}/"
            ))),
            Outcome::Interrupted(_) => Err(Stop::Interrupted),
            _ => Ok(true),
        }
    }

    /// Start or continue the first pending migration (section 4, Migrations). Returns
    /// whether it finished, so the next may start; `false` when there is none, or it
    /// waits.
    pub(crate) fn next_migration(&self) -> Result<bool, Stop> {
        if self.shutdown() {
            return Err(Stop::Interrupted);
        }
        match self.project.pending_migrations()?.into_iter().next() {
            Some(migration) => step_migration(&self.ctx, self.project, &migration),
            None => Ok(false),
        }
    }

    /// Print every waiting run (section 4, Replies).
    pub(crate) fn print_waiting(&self) -> Result<(), DecreeError> {
        print_waiting(&self.ctx)
    }
}

/// The context runs of `project` are stepped, or their status derived, in.
pub(crate) fn context<'a>(
    project_root: &Path,
    project: &'a Project,
    shutdown: Arc<AtomicBool>,
) -> Context<'a> {
    Context {
        project_root: project_root.to_path_buf(),
        shared_source: project.shared_source.clone(),
        machines: &project.machines,
        default_router: project.config.default_router.clone(),
        max_attempts: project.config.max_attempts,
        max_depth: project.config.max_depth,
        max_log_size: project.config.max_log_size,
        shutdown,
    }
}

/// Run `id`'s status (section 4, Run status) and its events. A run whose machine is not
/// loaded can never be stepped: it is `finished` in `failed` (an invalid message), else
/// `active` while its lock is live, else `interrupted`.
pub(crate) fn run_status(
    ctx: &Context,
    id: &str,
) -> Result<(RunStatus, Vec<serde_json::Map<String, Value>>), DecreeError> {
    let run_dir = ctx.runs_dir().join(id);
    let events = read_events(&run_dir)?;
    let alive = matches!(message::lock_state(&run_dir)?, LockState::Live(_));
    let status = match run_machine(ctx, &events) {
        Some(m) => ctx.status(m, &events, alive),
        None if interpreter::current_state(&events) == Some(FAILED) => RunStatus::Finished,
        None if alive => RunStatus::Active,
        None => RunStatus::Interrupted,
    };
    Ok((status, events))
}

/// The run folders in `runs_dir`, in `id` order.
pub(crate) fn run_ids(runs_dir: &Path) -> std::io::Result<Vec<String>> {
    let mut ids: Vec<String> = match std::fs::read_dir(runs_dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    ids.sort();
    Ok(ids)
}

/// The machine a run's first event names, if it is loaded.
fn run_machine<'m>(
    ctx: &Context<'m>,
    events: &[serde_json::Map<String, Value>],
) -> Option<&'m crate::machine::LoadedMachine> {
    events
        .first()
        .and_then(|e| e.get("machine"))
        .and_then(Value::as_str)
        .and_then(|name| ctx.machines.get(name))
}

/// Section 4, Migrations, rule 6: parse every pending migration (frontmatter, machine,
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
    message.write(&run_dir.join(MESSAGE_FILE)).map_err(other)?;

    let name = message
        .text("machine")
        .or_else(|| message.text("routine"))
        .unwrap_or_default()
        .to_string();
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
/// starts in `failed` (section 4, Lifecycle step 3).
fn start(
    ctx: &Context,
    project: &Project,
    id: &str,
    trigger: &str,
    file: &str,
    message: &Message,
) -> Result<Outcome, DecreeError> {
    let run_dir = ctx.runs_dir().join(id);
    let name = match message::validate(
        message,
        &project.machines,
        &project.machine_ids,
        project.default_machine(),
    ) {
        Ok(name) => name,
        Err(errors) => {
            let reason = errors
                .iter()
                .map(|(line, msg)| format!("line {line}: {msg}"))
                .collect::<Vec<_>>()
                .join("; ");
            let name = message
                .text("machine")
                .or_else(|| message.text("routine"))
                .unwrap_or_default();
            return reject(&run_dir, id, name, trigger, file, &reason, true);
        }
    };
    let machine = &project.machines[&name];
    let params = match message.frontmatter.get("params") {
        Some(serde_norway::Value::Mapping(params)) => params.clone(),
        _ => serde_norway::Mapping::new(),
    };
    let depth = message
        .frontmatter
        .get("depth")
        .and_then(serde_norway::Value::as_u64)
        .map_or(0, |d| u32::try_from(d).unwrap_or(u32::MAX));
    let executor = ctx
        .executor(machine, id, trigger, &params, message.text("parent"))
        .map_err(other)?;
    let input = RunInput {
        params,
        message_body: message.body.clone(),
        file: Some(file.to_string()),
        depth,
    };
    Interpreter::new(ctx, machine, executor, input)
        .and_then(|mut run| run.start())
        .map_err(other)
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
    interpreter::reject(&mut events, run_dir, file, reason, mirror).map_err(other)?;
    eprintln!("{file}: invalid message: {reason}");
    Ok(Outcome::Finished(FAILED.to_string()))
}

/// Section 4, Migrations: start migration `file`, or continue or report the run it has.
/// Returns whether later migrations may start once this one is in `processed.md`; `false`
/// means it waits.
fn step_migration(ctx: &Context, project: &Project, file: &str) -> Result<bool, Stop> {
    let id = file.strip_suffix(".md").unwrap_or(file);
    let run_dir = ctx.runs_dir().join(id);
    let outcome = if run_dir.exists() {
        // Rule 4: the migration has a run already.
        let events = read_events(&run_dir)?;
        let lock = message::lock_state(&run_dir)?;
        let alive = matches!(lock, LockState::Live(_));
        let status = run_machine(ctx, &events)
            .map_or(RunStatus::Interrupted, |m| ctx.status(m, &events, alive));
        match status {
            RunStatus::Pending => match continue_run(ctx, id) {
                Ok(outcome) => outcome,
                Err(InterpreterError::Active(_)) => return Ok(false),
                Err(e) => return Err(other(e).into()),
            },
            RunStatus::Waiting | RunStatus::Active => return Ok(false),
            RunStatus::Finished | RunStatus::Interrupted => {
                let state = interpreter::current_state(&events).unwrap_or("no state");
                return Err(blocked(
                    file,
                    id,
                    &format!("is {} in `{state}`", status.as_str()),
                ));
            }
        }
    } else {
        start_migration(ctx, project, file, id)?
    };
    match outcome {
        Outcome::Finished(state) if state == FAILED => {
            Err(blocked(file, id, &format!("ended in `{FAILED}`")))
        }
        // Rule 5 wrote the ledger line; the loop moves on to the next migration.
        Outcome::Finished(_) => Ok(true),
        Outcome::Interrupted(_) => Err(Stop::Interrupted),
        Outcome::Waiting { .. } | Outcome::Child { .. } => Ok(false),
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
        .join(config::MIGRATIONS_DIR)
        .join(file);
    let mut message = Message::read(&source).map_err(other)?;
    message.set("id", id);
    message.set("trigger", "migration");
    message
        .write(&runs.join(id).join(MESSAGE_FILE))
        .map_err(other)?;
    start(ctx, project, id, "migration", file, &message)
}

/// Rule 4: a migration that blocks every later one, and the command that continues it.
fn blocked(file: &str, id: &str, what: &str) -> Stop {
    Stop::Blocked(format!(
        "migration {file} {what}; later migrations are blocked. \
         Fix the cause, then run `decree retry {id}`."
    ))
}

/// Print every waiting run: its question (the state's `description`), its wait id and
/// options, and a `decree event` command for each option (section 4, Replies).
fn print_waiting(ctx: &Context) -> Result<(), DecreeError> {
    let mut ids: Vec<String> = match std::fs::read_dir(ctx.runs_dir()) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    ids.sort();
    for id in ids {
        let events = read_events(&ctx.runs_dir().join(&id))?;
        let Some(last) = events.last() else { continue };
        let text = |key: &str| last.get(key).and_then(Value::as_str);
        if text("type") != Some("waiting") {
            continue;
        }
        let (Some(state), Some(wait_id)) = (text("state"), text("wait_id")) else {
            continue;
        };
        let question = ctx
            .machines
            .get(text("machine").unwrap_or_default())
            .and_then(|m| m.find(state).map(|s| &m.nodes[s]))
            .and_then(|node| node.description.as_deref())
            .unwrap_or(state);
        let options: Vec<&str> = last
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
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
fn run_dry(project: &Project) -> Result<(), DecreeError> {
    let mut problems = project.problems.clone();
    let lists = [
        (MIGRATIONS_DIR, project.pending_migrations()?),
        (INBOX_DIR, md_files(&project.decree_dir.join(INBOX_DIR))?),
    ];
    for (dir, files) in lists {
        if files.is_empty() {
            println!("{dir}/: nothing queued");
            continue;
        }
        println!("{dir}/:");
        for file in files {
            let path = project.decree_dir.join(dir).join(&file);
            let target = Message::read(&path)
                .map_err(|e| vec![(0, e.to_string())])
                .and_then(|m| {
                    if m.frontmatter.contains_key("to") {
                        return Ok(format!("reply to {}", m.text("to").unwrap_or_default()));
                    }
                    message::validate(
                        &m,
                        &project.machines,
                        &project.machine_ids,
                        project.default_machine(),
                    )
                });
            match target {
                Ok(machine) => println!("  {file:<24} → {machine}"),
                Err(_) => {
                    println!("  {file:<24} → invalid");
                    let rule = if dir == MIGRATIONS_DIR { "M1" } else { "M2" };
                    project.check_file(dir, &file, rule, false, &mut problems)?;
                }
            }
        }
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

fn other(e: impl std::fmt::Display) -> DecreeError {
    DecreeError::Other(e.to_string())
}
