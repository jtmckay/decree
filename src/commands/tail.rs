//! `decree tail [<id>]` (docs/reference/cli.md): follow the live output of runs. Prints each
//! script's log as it is written, behind a header line (`== 0004 implement ==`), moves on to
//! the next script's log as the run proceeds, including into child runs. With an id it stops
//! when that run finishes, waits or is interrupted. Without one it follows the active run,
//! then each run after it, behind a run header (`▶ <id> · <machine>`), until stopped. Reads
//! the log files, events and `.running` only; it never touches a run. `decree process` and
//! `daemon` print the same output through `Follower`.

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::commands::check::Project;
use crate::commands::process::context;
use crate::error::DecreeError;
use crate::events::{current_state, first_text, text, waiting_child};
use crate::interpreter::recover::RunStatus;
use crate::interpreter::Context;
use crate::layout::MESSAGE_FILE;
use crate::message::{is_valid_id, run_ids, Message};
use crate::runtime::Running;

/// How often the run folder is read again.
pub(crate) const POLL: Duration = Duration::from_millis(100);

/// Run `decree tail`.
pub fn run(project_root: &Path, id: Option<&str>) -> Result<(), DecreeError> {
    let project = Project::load(project_root)?;
    let ctx = context(project_root, &project, Arc::new(AtomicBool::new(false)));
    let mut out = io::stdout().lock();
    let result = match id {
        Some(id) if is_valid_id(id) && ctx.runs_dir().join(id).is_dir() => {
            follow(&ctx, Follower::join(&ctx, id)?, &mut out)
        }
        Some(id) => return Err(DecreeError::MessageNotFound(id.to_string())),
        None => follow_all(&ctx, &mut out),
    };
    match result {
        // Whoever reads the output stopped reading: nothing left to do.
        Err(DecreeError::Io(e)) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result,
    }
}

/// Follow the active run, then every run after it, each behind its run header, until
/// stopped. A run that starts and ends between two reads is still printed, from its first
/// log, and so is one claimed while another is followed.
fn follow_all(ctx: &Context, out: &mut impl Write) -> Result<(), DecreeError> {
    // Runs that existed before tail started are followed only while active.
    let mut seen: HashSet<String> = run_ids(&ctx.runs_dir())?.into_iter().collect();
    let mut follower = match active_run(ctx)? {
        Some(id) => Some(Follower::join(ctx, &id)?),
        None => None,
    };
    loop {
        if let Some(f) = follower.take() {
            seen.insert(f.root().to_string());
            writeln!(out, "{}", run_header(ctx, f.root())?)?;
            follow(ctx, f, out)?;
        }
        follower = next_run(ctx, &mut seen)?;
        if follower.is_none() {
            thread::sleep(POLL);
        }
    }
}

/// The next run for `follow_all`: the first new top-level run folder, from its first log,
/// else the active run, from where it is.
fn next_run(ctx: &Context, seen: &mut HashSet<String>) -> Result<Option<Follower>, DecreeError> {
    for id in run_ids(&ctx.runs_dir())? {
        if seen.contains(&id) {
            continue;
        }
        // A claim creates the folder, then renames the message into it.
        let Ok(message) = Message::read(&ctx.runs_dir().join(&id).join(MESSAGE_FILE)) else {
            continue;
        };
        seen.insert(id.clone());
        // A child run is followed from its parent.
        if message.text("trigger") != Some("invoke") {
            return Ok(Some(Follower::from_log(&id, 1)));
        }
    }
    Ok(match active_run(ctx)? {
        Some(id) => Some(Follower::join(ctx, &id)?),
        None => None,
    })
}

/// `▶ <id> · <machine>`, the line `tail` prints as it moves to a run.
pub(crate) fn run_header(ctx: &Context, id: &str) -> Result<String, DecreeError> {
    let (machine, _) = machine_and_state(ctx, id)?;
    Ok(format!("▶ {id} · {}", machine.as_deref().unwrap_or("?")))
}

/// The machine of run `id`, from its events or else its message, and its current state.
pub(crate) fn machine_and_state(
    ctx: &Context,
    id: &str,
) -> Result<(Option<String>, Option<String>), DecreeError> {
    let events = ctx.events(id)?;
    let machine = match events.first().and_then(|e| text(e, "machine")) {
        Some(machine) => Some(machine.to_string()),
        None => Message::read(&ctx.runs_dir().join(id).join(MESSAGE_FILE))
            .ok()
            .and_then(|m| m.machine().map(String::from)),
    };
    Ok((machine, current_state(&events).map(String::from)))
}

/// Follow `follower` until its run finishes, waits or is interrupted.
fn follow(ctx: &Context, mut follower: Follower, out: &mut impl Write) -> Result<(), DecreeError> {
    while follower.step(ctx, out)? {
        thread::sleep(POLL);
    }
    Ok(())
}

/// The active run to follow: the first in `id` order that is not a child run (a child
/// is followed from its parent), else the first child.
fn active_run(ctx: &Context) -> Result<Option<String>, DecreeError> {
    let mut child = None;
    for id in run_ids(&ctx.runs_dir())? {
        if ctx.run_finished(&id)?.is_some() {
            continue;
        }
        let (status, events) = ctx.status_of(&id)?;
        if status != RunStatus::Active {
            continue;
        }
        if first_text(&events, "trigger") != Some("invoke") {
            return Ok(Some(id));
        }
        child.get_or_insert(id);
    }
    Ok(child)
}

/// Where tail is in one run's logs.
struct Cursor {
    run: String,
    /// The log being printed, and how many of its bytes are printed.
    log: Option<(u32, String, u64)>,
    /// The lowest log number not yet started.
    next: u32,
}

impl Cursor {
    /// Start at the log of the script running now, else at the next log to appear.
    fn join(ctx: &Context, run: &str) -> Result<Cursor, DecreeError> {
        let run_dir = ctx.runs_dir().join(run);
        let running = Running::read(&run_dir)?.and_then(|r| log_number(&r.log));
        let next = match running {
            Some(n) => n,
            None => logs(&run_dir)?.last().map_or(1, |(n, _)| n + 1),
        };
        Ok(Cursor::from_log(run, next))
    }

    /// Start at log `next`.
    fn from_log(run: &str, next: u32) -> Cursor {
        Cursor {
            run: run.to_string(),
            log: None,
            next,
        }
    }

    /// Print what was written to this run's logs since the last call: the rest of the
    /// current log, then every later log in order, each behind its header.
    fn pump(&mut self, ctx: &Context, out: &mut impl Write) -> Result<(), DecreeError> {
        let run_dir = ctx.runs_dir().join(&self.run);
        for (n, name) in logs(&run_dir)? {
            if let Some((current, file, offset)) = &mut self.log {
                if n == *current {
                    *offset = copy_from(&run_dir.join(&*file), *offset, out)?;
                    continue;
                }
            }
            if n < self.next {
                continue;
            }
            writeln!(out, "== {}", header(&name))?;
            let offset = copy_from(&run_dir.join(&name), 0, out)?;
            self.log = Some((n, name, offset));
            self.next = n + 1;
        }
        out.flush()?;
        Ok(())
    }
}

/// One run's output as it is written: its logs, and those of the child runs it waits for.
pub(crate) struct Follower {
    /// The run, then each child run it waits for, innermost last.
    stack: Vec<Cursor>,
}

impl Follower {
    /// Follow run `id` from the script running now, joining the child runs it waits for.
    pub(crate) fn join(ctx: &Context, id: &str) -> Result<Follower, DecreeError> {
        let mut stack = vec![Cursor::join(ctx, id)?];
        while let Some(child) = child_of(ctx, &stack[stack.len() - 1].run)? {
            stack.push(Cursor::join(ctx, &child)?);
        }
        Ok(Follower { stack })
    }

    /// Follow run `id` from log `next`, as it starts or continues.
    pub(crate) fn from_log(id: &str, next: u32) -> Follower {
        Follower {
            stack: vec![Cursor::from_log(id, next)],
        }
    }

    /// The run followed.
    pub(crate) fn root(&self) -> &str {
        &self.stack[0].run
    }

    /// The innermost run followed now: the run, or the child run it waits for.
    pub(crate) fn current(&self) -> &str {
        &self.stack[self.stack.len() - 1].run
    }

    /// Print what was written since the last call, moving into a child run as it starts
    /// and back as it finishes. Returns whether the run goes on.
    pub(crate) fn step(
        &mut self,
        ctx: &Context,
        out: &mut impl Write,
    ) -> Result<bool, DecreeError> {
        loop {
            // Observe first, then print: what the run did before it stopped is all printed.
            let top = self.current().to_string();
            let child = child_of(ctx, &top)?;
            let top_finished = ctx.status_of(&top)?.0 == RunStatus::Finished;
            let live = is_live(ctx, self.root())?;
            let depth = self.stack.len();
            self.stack[depth - 1].pump(ctx, out)?;
            if let Some(child) = child.filter(|c| !self.stack.iter().any(|cur| cur.run == *c)) {
                // A child that started while tail was following: print it from its first log.
                self.stack.push(Cursor::from_log(&child, 1));
                continue;
            }
            if depth > 1 && top_finished {
                self.stack.pop();
                continue;
            }
            return Ok(depth > 1 || live);
        }
    }

    /// Print the rest of every run followed, innermost first, once the run has stopped.
    pub(crate) fn finish(
        &mut self,
        ctx: &Context,
        out: &mut impl Write,
    ) -> Result<(), DecreeError> {
        while let Some(mut cursor) = self.stack.pop() {
            cursor.pump(ctx, out)?;
            if self.stack.is_empty() {
                self.stack.push(cursor);
                break;
            }
        }
        Ok(())
    }
}

/// Whether run `id` goes on: `active` or `pending`, or waiting for a child run that goes on
/// itself. A finished child leaves its parent `pending` until it is continued.
fn is_live(ctx: &Context, id: &str) -> Result<bool, DecreeError> {
    Ok(match ctx.status_of(id)?.0 {
        RunStatus::Active | RunStatus::Pending => true,
        RunStatus::Waiting => match child_of(ctx, id)? {
            Some(child) => is_live(ctx, &child)?,
            None => false,
        },
        RunStatus::Finished | RunStatus::Interrupted => false,
    })
}

/// The child run `run` waits for, if its last event is a `waiting` that names one.
fn child_of(ctx: &Context, run: &str) -> Result<Option<String>, DecreeError> {
    let events = ctx.events(run)?;
    let child =
        waiting_child(&events).filter(|c| is_valid_id(c) && ctx.runs_dir().join(c).is_dir());
    Ok(child.map(String::from))
}

/// The `NNNN-<state>-<script>.log` and `NNNN-<state>.log` files in `run_dir`, by number.
fn logs(run_dir: &Path) -> io::Result<Vec<(u32, String)>> {
    let mut logs: Vec<(u32, String)> = std::fs::read_dir(run_dir)?
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|name| log_number(&name).map(|n| (n, name)))
        .collect();
    logs.sort();
    Ok(logs)
}

/// The `NNNN` of a script log's filename: 4 digits, more past 9999.
fn log_number(name: &str) -> Option<u32> {
    let (number, rest) = name.strip_suffix(".log")?.split_once('-')?;
    if number.len() < 4 || !number.bytes().all(|b| b.is_ascii_digit()) || rest.is_empty() {
        return None;
    }
    number.parse().ok()
}

/// `0004 gate ==` for `0004-gate.log`, `0012 _root/setup ==` for `0012-_root-setup.log`.
/// State and script names cannot contain `-` (docs/reference/observability.md).
fn header(name: &str) -> String {
    let stem = name.strip_suffix(".log").unwrap_or(name);
    match stem.splitn(3, '-').collect::<Vec<_>>()[..] {
        [n, state, script] => format!("{n} {state}/{script} =="),
        [n, state] => format!("{n} {state} =="),
        _ => format!("{stem} =="),
    }
}

/// Copy `path` from byte `offset` to its end into `out`; returns the new offset. A log
/// truncated to its 2 MiB cap after it was printed is skipped to its end.
fn copy_from(path: &Path, offset: u64, out: &mut impl Write) -> io::Result<u64> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(offset),
        Err(e) => return Err(e),
    };
    let len = file.metadata()?.len();
    if len < offset {
        return Ok(len);
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut buf = Vec::new();
    file.take(len - offset).read_to_end(&mut buf)?;
    out.write_all(&buf)?;
    Ok(offset + buf.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_number_and_header() {
        assert_eq!(log_number("0004-implement-implement.log"), Some(4));
        assert_eq!(log_number("0012-_root-setup.log"), Some(12));
        assert_eq!(log_number("10000-work-step.log"), Some(10000));
        for name in ["run.log", "04-a-b.log", "0004-a-b.txt", "x004-a-b.log"] {
            assert_eq!(log_number(name), None, "{name}");
        }
        assert_eq!(log_number("0004-gate.log"), Some(4));
        assert_eq!(header("0004-gate.log"), "0004 gate ==");
        assert_eq!(header("0012-_root-setup.log"), "0012 _root/setup ==");
    }

    #[test]
    fn test_copy_from_prints_only_new_bytes() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("0001-s-x.log");
        std::fs::write(&path, "one\n").unwrap();
        let mut out = Vec::new();
        let offset = copy_from(&path, 0, &mut out).unwrap();
        std::fs::write(&path, "one\ntwo\n").unwrap();
        let offset = copy_from(&path, offset, &mut out).unwrap();
        assert_eq!(out, b"one\ntwo\n");
        assert_eq!(offset, 8);
        // Truncated below what was printed: skip to the end, print nothing.
        std::fs::write(&path, "x\n").unwrap();
        assert_eq!(copy_from(&path, offset, &mut out).unwrap(), 2);
        assert_eq!(out, b"one\ntwo\n");
    }
}
