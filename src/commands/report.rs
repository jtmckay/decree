//! What `decree process` and `daemon` print as runs go, unless `--quiet` (docs/reference/cli.md,
//! Output): each run's script output as `decree tail` prints it, behind the run's header; on
//! a terminal, a status line below it (`▶ <id> · <machine> · <state> · 3m 12s · 4/31`); and
//! one line as each run stops (`✓ <id> done in 7m 31s`). The pipeline tells a thread when
//! each run starts and stops, and the thread follows the run's files, so every run is
//! printed whole and in order, however short. Notices go through the same thread, so they
//! land after the output of the run they are about.

use std::cell::Cell;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
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
    started: Cell<usize>,
}

impl Reporter {
    pub(crate) fn new() -> (Reporter, Receiver<Note>) {
        let (tx, rx) = mpsc::channel();
        *SINK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(tx.clone());
        let reporter = Reporter {
            tx,
            started: Cell::new(0),
        };
        (reporter, rx)
    }

    /// Run `id`, in `run_dir`, starts or continues, with `queued` messages and migrations
    /// left after it.
    pub(crate) fn start(&self, id: &str, run_dir: &Path, queued: usize) {
        let position = self.started.get() + 1;
        self.started.set(position);
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
    started: Instant,
    position: usize,
    total: usize,
    /// Whether its header is printed: once its machine is known.
    header: bool,
}

/// The thread: print what `rx` announces until the pipeline drops its end. Output errors
/// (a closed terminal) are ignored: the runs go on either way.
pub(crate) fn observe(ctx: &Context, rx: Receiver<Note>) {
    let mut console = Console::new();
    let mut current: Option<Current> = None;
    loop {
        let note = match rx.recv_timeout(POLL) {
            Ok(note) => Some(note),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                if let Some(run) = current.take() {
                    finish(ctx, &mut console, run, None);
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
                if let Some(run) = current.take() {
                    finish(ctx, &mut console, run, None);
                }
                current = Some(Current {
                    follower: Follower::from_log(&id, first_log),
                    started: Instant::now(),
                    position,
                    total,
                    header: false,
                });
            }
            Some(Note::End { id, outcome }) => {
                if let Some(run) = current.take_if(|run| run.follower.root() == id) {
                    finish(ctx, &mut console, run, outcome.as_ref());
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
        if let Some(run) = &mut current {
            print_header(ctx, &mut console, run, false);
            if run.header {
                let _ = run.follower.step(ctx, &mut console);
            }
            let _ = console.set_status(Some(status_line(ctx, run)));
        }
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
        let id = run.follower.root();
        let _ = console.line(&format!("▶ {id} · {}", machine.as_deref().unwrap_or("?")));
        run.header = true;
    }
}

/// `▶ <run> · <machine> · <state> · <elapsed> · <position>/<total>`: the innermost run's
/// machine and state, so a child run shows where the work is.
fn status_line(ctx: &Context, run: &Current) -> String {
    let (machine, state) = machine_and_state(ctx, run.follower.current()).unwrap_or_default();
    format!(
        "▶ {} · {} · {} · {} · {}/{}",
        run.follower.root(),
        machine.as_deref().unwrap_or("?"),
        state.as_deref().unwrap_or("starting"),
        console::duration(run.started.elapsed()),
        run.position,
        run.total
    )
}

/// Print the rest of the run's output, clear the status line, and print how it stopped.
fn finish(ctx: &Context, console: &mut Console, mut run: Current, outcome: Option<&Outcome>) {
    print_header(ctx, console, &mut run, true);
    let _ = run.follower.finish(ctx, console);
    let _ = console.set_status(None);
    let id = run.follower.root();
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
    let _ = console.line(&line);
}
