//! `decree tail [<id>]` (docs/reference/cli.md): follow the live output of a run, by default the
//! `active` one. Prints each script's log as it is written, behind a header line
//! (`== 0004 implement/implement ==`), moves on to the next script's log as the run
//! proceeds, including into child runs, and stops when the run finishes, waits or is
//! interrupted. Reads the log files, events and `.running` only; it never touches the run.

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
use crate::events::{first_text, waiting_child};
use crate::interpreter::recover::RunStatus;
use crate::interpreter::Context;
use crate::message::{is_valid_id, run_ids};
use crate::runtime::Running;

/// How often the run folder is read again.
const POLL: Duration = Duration::from_millis(100);

/// Run `decree tail`.
pub fn run(project_root: &Path, id: Option<&str>) -> Result<(), DecreeError> {
    let project = Project::load(project_root)?;
    let ctx = context(project_root, &project, Arc::new(AtomicBool::new(false)));
    let id = match id {
        Some(id) if is_valid_id(id) && ctx.runs_dir().join(id).is_dir() => id.to_string(),
        Some(id) => return Err(DecreeError::MessageNotFound(id.to_string())),
        None => {
            active_run(&ctx)?.ok_or_else(|| DecreeError::Other("no run is active".to_string()))?
        }
    };
    let mut out = io::stdout().lock();
    match follow(&ctx, &id, &mut out) {
        // Whoever reads the output stopped reading: nothing left to do.
        Err(DecreeError::Io(e)) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result,
    }
}

/// The `active` run to follow: the first in `id` order that is not a child run (a child
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
        Ok(Cursor {
            run: run.to_string(),
            log: None,
            next,
        })
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

/// Follow run `id` until it finishes, waits or is interrupted.
fn follow(ctx: &Context, id: &str, out: &mut impl Write) -> Result<(), DecreeError> {
    let mut stack = vec![Cursor::join(ctx, id)?];
    // Join the child runs the run is waiting for now.
    while let Some(child) = child_of(ctx, &stack[stack.len() - 1].run)? {
        stack.push(Cursor::join(ctx, &child)?);
    }
    loop {
        // Observe first, then print: what the run did before it stopped is all printed.
        let top = &stack[stack.len() - 1].run;
        let child = child_of(ctx, top)?;
        let top_finished = ctx.status_of(top)?.0 == RunStatus::Finished;
        let live = is_live(ctx, id)?;
        let depth = stack.len();
        stack[depth - 1].pump(ctx, out)?;
        if let Some(child) = child.filter(|c| !stack.iter().any(|cur| cur.run == *c)) {
            // A child that started while tail was following: print it from its first log.
            stack.push(Cursor {
                run: child,
                log: None,
                next: 1,
            });
            continue;
        }
        if depth > 1 && top_finished {
            stack.pop();
            continue;
        }
        if depth == 1 && !live {
            return Ok(());
        }
        thread::sleep(POLL);
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

/// The `NNNN-<state>-<script>.log` files in `run_dir`, by number.
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
    if number.len() < 4 || !number.bytes().all(|b| b.is_ascii_digit()) || !rest.contains('-') {
        return None;
    }
    number.parse().ok()
}

/// `0004 implement/implement ==` for `0004-implement-implement.log`. State and script
/// names cannot contain `-` (docs/reference/observability.md).
fn header(name: &str) -> String {
    let stem = name.strip_suffix(".log").unwrap_or(name);
    let mut parts = stem.splitn(3, '-');
    let (n, state, script) = (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    );
    format!("{n} {state}/{script} ==")
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
        for name in [
            "routine.log",
            "04-a-b.log",
            "0004-a.log",
            "0004-a-b.txt",
            "x004-a-b.log",
        ] {
            assert_eq!(log_number(name), None, "{name}");
        }
        assert_eq!(
            header("0004-implement-implement.log"),
            "0004 implement/implement =="
        );
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
