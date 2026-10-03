//! `decree process` (spec sections 4 and 8): validate the machines and pending migrations,
//! mark runs a crash left behind `interrupted` and continue `pending` runs, then repeat
//! until nothing is left: claim the next `inbox/` message and run it through its machine,
//! or start the next migration once the inbox is empty. Never continues an `interrupted`
//! run. Stops at the first run that ends in `failed`, and before a `failed`, `interrupted`
//! or `waiting` migration. Ends by printing every waiting run.
//!
//! Replies (`to:`) are delivered from ticket M4.3 on.

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
use crate::runtime::{self, EventLog, MESSAGE_FILE};

/// Run `decree process [--dry-run]`.
pub fn run(project_root: &Path, dry_run: bool) -> Result<(), DecreeError> {
    let project = Project::load(project_root)?;
    if dry_run {
        return run_dry(&project);
    }
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

    let shutdown = Arc::new(AtomicBool::new(false));
    runtime::register_signals(&shutdown)?;
    let ctx = Context {
        project_root: project_root.to_path_buf(),
        shared_source: project.shared_source.clone(),
        machines: &project.machines,
        default_router: project.config.default_router.clone(),
        max_attempts: project.config.max_attempts,
        max_depth: project.config.max_depth,
        max_log_size: project.config.max_log_size,
        shutdown,
    };
    let recovery = interpreter::recover(&ctx).map_err(other)?;
    for (id, state) in &recovery.crashed {
        eprintln!(
            "run {id} was interrupted in `{state}` (crash); \
             continue it with `decree retry {id}`"
        );
    }
    let result = continue_pending(&ctx, &recovery.pending).and_then(|()| drain(&ctx, &project));
    print_waiting(&ctx)?;
    result
}

/// Continue the `pending` runs, in `id` order, before reading `inbox/` (section 4, Run
/// status). A run another process holds now is skipped.
fn continue_pending(ctx: &Context, ids: &[String]) -> Result<(), DecreeError> {
    for id in ids {
        if ctx.shutdown.load(Ordering::Relaxed) {
            return Err(DecreeError::Interrupted);
        }
        let outcome = match continue_run(ctx, id) {
            Ok(outcome) => outcome,
            Err(InterpreterError::Active(_)) => continue,
            Err(e) => return Err(other(e)),
        };
        match outcome {
            Outcome::Finished(state) if state == FAILED => {
                let run_dir = ctx.runs_dir().join(id);
                let migration = read_events(&run_dir)?
                    .first()
                    .is_some_and(|e| e.get("trigger").and_then(Value::as_str) == Some("migration"));
                return Err(if migration {
                    blocked(&format!("{id}.md"), id, &format!("ended in `{FAILED}`"))
                } else {
                    DecreeError::Other(format!(
                        "run {id} ended in `{FAILED}`; see .decree/runs/{id}/"
                    ))
                });
            }
            Outcome::Interrupted(_) => return Err(DecreeError::Interrupted),
            _ => {}
        }
    }
    Ok(())
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

/// Run inbox messages in filename order, and the next migration whenever the inbox is
/// empty (section 4, Lifecycle and Migrations).
fn drain(ctx: &Context, project: &Project) -> Result<(), DecreeError> {
    let decree_dir = ctx.project_root.join(DECREE_DIR);
    // Files another process claimed first, or is claiming now.
    let mut lost = HashSet::new();
    loop {
        if ctx.shutdown.load(Ordering::Relaxed) {
            return Err(DecreeError::Interrupted);
        }
        let next = md_files(&decree_dir.join(INBOX_DIR))?
            .into_iter()
            .find(|f| !lost.contains(f));
        if let Some(file) = next {
            match message::claim(&decree_dir, &file).map_err(other)? {
                None => {
                    lost.insert(file);
                }
                Some(claim) => {
                    let id = claim.id.clone();
                    match run_claim(ctx, project, claim)? {
                        Outcome::Finished(state) if state == FAILED => {
                            return Err(DecreeError::Other(format!(
                                "run {id} ({file}) ended in `{FAILED}`; see .decree/runs/{id}/"
                            )))
                        }
                        Outcome::Interrupted(_) => return Err(DecreeError::Interrupted),
                        _ => {}
                    }
                }
            }
            continue;
        }
        let Some(migration) = project.pending_migrations()?.into_iter().next() else {
            return Ok(());
        };
        if !step_migration(ctx, project, &migration)? {
            return Ok(());
        }
    }
}

/// Lifecycle steps 2 to 4 for a claimed inbox message: set `id` and `trigger` if missing,
/// validate, and start the run, or fail it with `invalid_message`.
fn run_claim(ctx: &Context, project: &Project, claim: Claim) -> Result<Outcome, DecreeError> {
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
        let reason =
            "a reply (`to:`) cannot start a run, and reply delivery is not implemented yet";
        return reject(&run_dir, &id, &name, &trigger, &file, reason, true);
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
fn step_migration(ctx: &Context, project: &Project, file: &str) -> Result<bool, DecreeError> {
    let id = file.strip_suffix(".md").unwrap_or(file);
    let run_dir = ctx.runs_dir().join(id);
    let outcome = if run_dir.exists() {
        // Rule 4: the migration has a run already.
        let events = read_events(&run_dir)?;
        let machine = events
            .first()
            .and_then(|e| e.get("machine"))
            .and_then(Value::as_str)
            .and_then(|name| project.machines.get(name));
        let lock = message::lock_state(&run_dir)?;
        let alive = matches!(lock, LockState::Live(_));
        let status = machine.map_or(RunStatus::Interrupted, |m| ctx.status(m, &events, alive));
        match status {
            RunStatus::Pending => match continue_run(ctx, id) {
                Ok(outcome) => outcome,
                Err(InterpreterError::Active(_)) => return Ok(false),
                Err(e) => return Err(other(e)),
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
        Outcome::Interrupted(_) => Err(DecreeError::Interrupted),
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
fn blocked(file: &str, id: &str, what: &str) -> DecreeError {
    DecreeError::Other(format!(
        "migration {file} {what}; later migrations are blocked. \
         Fix the cause, then run `decree retry {id}`."
    ))
}

/// Print every waiting run: its question (the state's `description`), and a
/// `decree event` command for each accepted event (section 4, Replies).
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
        println!("Waiting: run {id} in `{state}`: {question}");
        for option in last
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
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
