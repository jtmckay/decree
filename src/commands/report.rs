//! What `decree daemon` prints as runs go, unless `--quiet` (docs/reference/cli.md,
//! Output): each run's script output as `decree tail` prints it, behind the run's header; on
//! a terminal, a status line below it (`▶ <id> · <machine> · <state> · 3m 12s · 4/31`); and
//! one line as each run stops (`✓ <id> done in 7m 31s`). The pipeline tells a thread when
//! each run starts and stops, and the thread follows the run's files, so every run is
//! printed whole and in order, however short. Notices go through the same thread, so they
//! land after the output of the run they are about. All of it but the status line also goes
//! to `.decree/process.log`, with `--quiet` too.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::commands::tail::{machine_and_state, Follower, POLL};
use crate::console::{self, Console};
use crate::interpreter::{Context, Outcome};
use crate::machine::FAILED;

/// What the pipeline tells the thread.
pub(crate) enum Note {
    /// Run `id` starts or continues, its first new script log numbered `first_log`. It is
    /// run `position` of this pass, of `total` so far.
    Start {
        id: String,
        first_log: u32,
        position: usize,
        total: usize,
    },
    /// Run `id` stopped with this outcome, or with an error, `None`.
    End {
        id: String,
        outcome: Option<Outcome>,
    },
    /// A line for stdout.
    Line(String),
    /// A line for stderr.
    Notice(String),
}

/// Where `line` and `notice` send while a `Reporter` lives.
static SINK: Mutex<Option<Sender<Note>>> = Mutex::new(None);

/// Print `text` on stdout: through the reporting thread while there is one, so it lands
/// after the output before it and never over the status line.
pub(crate) fn line(text: String) {
    if let Err(Note::Line(text)) = send(Note::Line(text)) {
        println!("{text}");
    }
}

/// Print `text` on stderr, as `line` does.
pub(crate) fn notice(text: String) {
    if let Err(Note::Notice(text)) = send(Note::Notice(text)) {
        eprintln!("{text}");
    }
}

/// Send `note` to the reporting thread, or hand it back when there is none.
fn send(note: Note) -> Result<(), Note> {
    let sink = SINK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match sink.as_ref() {
        Some(tx) => tx.send(note).map_err(|e| e.0),
        None => Err(note),
    }
}

/// The pipeline's end of the channel.
pub(crate) struct Reporter {
    tx: Sender<Note>,
    /// Runs started this pass.
    started: Arc<AtomicUsize>,
}

/// What a worker thread tells the reporting thread about its run: a clone of the
/// pipeline's end, which can move to another thread.
#[derive(Clone)]
pub(crate) struct RunNotes {
    tx: Sender<Note>,
    started: Arc<AtomicUsize>,
}

impl Reporter {
    pub(crate) fn new() -> (Reporter, Receiver<Note>) {
        let (tx, rx) = mpsc::channel();
        *SINK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(tx.clone());
        let reporter = Reporter {
            tx,
            started: Arc::new(AtomicUsize::new(0)),
        };
        (reporter, rx)
    }

    /// A handle for a worker thread.
    pub(crate) fn notes(&self) -> RunNotes {
        RunNotes {
            tx: self.tx.clone(),
            started: Arc::clone(&self.started),
        }
    }
}

impl RunNotes {
    /// Run `id`, in `run_dir`, starts or continues, with `queued` messages
    /// left after it.
    pub(crate) fn start(&self, id: &str, run_dir: &Path, queued: usize) {
        let position = self.started.fetch_add(1, Ordering::Relaxed) + 1;
        let first_log = crate::runtime::next_log_number(run_dir).unwrap_or(1);
        self.send(Note::Start {
            id: id.to_string(),
            first_log,
            position,
            total: position + queued,
        });
    }

    pub(crate) fn end(&self, id: &str, outcome: Option<&Outcome>) {
        self.send(Note::End {
            id: id.to_string(),
            outcome: outcome.cloned(),
        });
    }

    fn send(&self, note: Note) {
        let _ = self.tx.send(note);
    }
}

impl Drop for Reporter {
    /// Close the channel, so the thread prints what is left and ends.
    fn drop(&mut self) {
        *SINK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}

/// The run being printed.
struct Current {
    follower: Follower,
    /// How the run is shown, `<id>`, known once
    /// its header is printed.
    label: String,
    started: Instant,
    position: usize,
    total: usize,
    /// Whether its header is printed: once its machine is known.
    header: bool,
    /// Its output after the last newline, held while other runs print beside it.
    partial: Vec<u8>,
    /// Whether it has printed beside another run: from then on, to its end, each of its
    /// lines is behind its prefix.
    prefixed: bool,
}

/// Where a run's output goes while other runs execute beside it: each whole line, behind
/// `[<the run id's last six characters>] `, so the lines of runs that execute at once
/// (docs/reference/machines.md, Concurrency) can be told apart.
struct Prefixed<'a> {
    console: &'a mut Console,
    prefix: String,
    partial: &'a mut Vec<u8>,
}

impl std::io::Write for Prefixed<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.partial.extend_from_slice(buf);
        while let Some(end) = self.partial.iter().position(|&b| b == b'\n') {
            let mut line = self.prefix.clone().into_bytes();
            line.extend(self.partial.drain(..=end));
            self.console.write_out(&line)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if !self.partial.is_empty() {
            self.partial.push(b'\n');
            self.write(&[])?;
        }
        Ok(())
    }
}

/// `[<last six characters of id>] `.
fn prefix(id: &str) -> String {
    format!("[{}] ", &id[id.len().saturating_sub(6)..])
}

impl Current {
    /// Print what the run wrote since the last call: straight to `console` while it runs
    /// alone, else line by line behind its prefix.
    fn step(&mut self, ctx: &Context, console: &mut Console, alone: bool) {
        self.prefixed |= !alone;
        if !self.prefixed {
            let mut held = Prefixed {
                prefix: prefix(self.follower.root()),
                console: &mut *console,
                partial: &mut self.partial,
            };
            let _ = std::io::Write::flush(&mut held);
            let _ = self.follower.step(ctx, console);
        } else {
            let mut out = Prefixed {
                prefix: prefix(self.follower.root()),
                console,
                partial: &mut self.partial,
            };
            let _ = self.follower.step(ctx, &mut out);
        }
    }
}

/// The thread: print what `rx` announces until the pipeline drops its end. Output errors
/// (a closed terminal) are ignored: the runs go on either way.
pub(crate) fn observe(ctx: &Context, rx: Receiver<Note>, quiet: bool) {
    let log_path = ctx
        .project_root
        .join(crate::layout::DECREE_DIR)
        .join(crate::layout::PROCESS_LOG_FILE);
    let trim = || {
        let _ = crate::runtime::truncate_log_if_needed(&log_path, crate::runtime::MAX_LOG_SIZE);
    };
    trim();
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok();
    let mut console = Console::logged(log, quiet);
    // The runs executing now, in the order they started.
    let mut runs: Vec<Current> = Vec::new();
    loop {
        let note = match rx.recv_timeout(POLL) {
            Ok(note) => Some(note),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                let alone = runs.len() == 1;
                for run in runs.drain(..) {
                    finish(ctx, &mut console, run, None, alone);
                }
                return;
            }
        };
        match note {
            Some(Note::Start {
                id,
                first_log,
                position,
                total,
            }) => {
                // The same run started again (another process let it go): follow it anew.
                if let Some(i) = runs.iter().position(|r| r.follower.root() == id) {
                    let alone = runs.len() == 1;
                    finish(ctx, &mut console, runs.remove(i), None, alone);
                }
                runs.push(Current {
                    label: id.clone(),
                    follower: Follower::from_log(&id, first_log),
                    started: Instant::now(),
                    position,
                    total,
                    header: false,
                    partial: Vec::new(),
                    prefixed: false,
                });
            }
            Some(Note::End { id, outcome }) => {
                if let Some(i) = runs.iter().position(|r| r.follower.root() == id) {
                    let alone = runs.len() == 1;
                    finish(ctx, &mut console, runs.remove(i), outcome.as_ref(), alone);
                    trim();
                }
            }
            Some(Note::Line(text)) => {
                let _ = console.line(&text);
            }
            Some(Note::Notice(text)) => {
                let _ = console.notice(&text);
            }
            None => {}
        }
        let alone = runs.len() == 1;
        for run in &mut runs {
            print_header(ctx, &mut console, run, false);
            if run.header {
                run.step(ctx, &mut console, alone);
            }
        }
        let status = match runs.as_slice() {
            [] => None,
            [run] => Some(status_line(ctx, run)),
            many => Some(status_lines(ctx, many)),
        };
        let _ = console.set_status(status);
    }
}

/// Print the run's header once its machine is known, or at once when `now`.
fn print_header(ctx: &Context, console: &mut Console, run: &mut Current, now: bool) {
    if run.header {
        return;
    }
    let machine = machine_and_state(ctx, run.follower.root())
        .ok()
        .and_then(|(machine, _)| machine);
    if machine.is_some() || now {
        run.label = run.follower.root().to_string();
        let _ = console.run_line(&format!(
            "▶ {} · {}",
            run.label,
            machine.as_deref().unwrap_or("?")
        ));
        run.header = true;
    }
}

/// `▶ <run> · <machine> · <state> · <elapsed> · <position>/<total>`: the innermost run's
/// machine and state, so a child run shows where the work is.
fn status_line(ctx: &Context, run: &Current) -> String {
    let (machine, state) = machine_and_state(ctx, run.follower.current()).unwrap_or_default();
    format!(
        "▶ {} · {} · {} · {} · {}/{}",
        run.label,
        machine.as_deref().unwrap_or("?"),
        state.as_deref().unwrap_or("starting"),
        console::duration(run.started.elapsed()),
        run.position,
        run.total
    )
}

/// `▶ <n> running · <machine> <state> <elapsed> · …`: each run executing, its innermost
/// machine and state.
fn status_lines(ctx: &Context, runs: &[Current]) -> String {
    let each: Vec<String> = runs
        .iter()
        .map(|run| {
            let (machine, state) =
                machine_and_state(ctx, run.follower.current()).unwrap_or_default();
            format!(
                "{} {} {}",
                machine.as_deref().unwrap_or("?"),
                state.as_deref().unwrap_or("starting"),
                console::duration(run.started.elapsed())
            )
        })
        .collect();
    format!("▶ {} running · {}", runs.len(), each.join(" · "))
}

/// Print the rest of the run's output (behind its prefix unless it ran `alone`), clear the
/// status line, and print how it stopped.
fn finish(
    ctx: &Context,
    console: &mut Console,
    mut run: Current,
    outcome: Option<&Outcome>,
    alone: bool,
) {
    print_header(ctx, console, &mut run, true);
    if alone && !run.prefixed {
        let mut out = Prefixed {
            prefix: prefix(run.follower.root()),
            console: &mut *console,
            partial: &mut run.partial,
        };
        let _ = std::io::Write::flush(&mut out);
        let _ = run.follower.finish(ctx, console);
    } else {
        let mut out = Prefixed {
            prefix: prefix(run.follower.root()),
            console: &mut *console,
            partial: &mut run.partial,
        };
        let _ = run.follower.finish(ctx, &mut out);
        let _ = std::io::Write::flush(&mut out);
    }
    let _ = console.set_status(None);
    let id = &run.label;
    let took = console::duration(run.started.elapsed());
    let line = match outcome {
        Some(Outcome::Finished(state)) if state == FAILED => format!("✗ {id} {FAILED} in {took}"),
        Some(Outcome::Finished(state)) => format!("✓ {id} {state} in {took}"),
        Some(Outcome::Waiting { state, .. }) | Some(Outcome::Child { state, .. }) => {
            format!("⏸ {id} waiting in {state} after {took}")
        }
        Some(Outcome::Interrupted(state)) => format!("■ {id} interrupted in {state} after {took}"),
        None => return,
    };
    let _ = console.run_line(&line);
}
