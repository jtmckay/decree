//! The step loop (docs/reference/runs.md): moves one run through one machine with SCXML's exit
//! and entry order, runs each state's invoke, and appends every step to the run's
//! `events.jsonl`. State, status and visits are derived from that log (docs/reference/messages.md, Source
//! of truth).
//!
//! Interpreted here: the whole SCXML subset (docs/reference/machines.md). Transitions on compound states, with
//! events bubbling from the atomic state outward; `type: internal`; final states at any
//! level, a nested one raising `done.state.<parent>`; `person`, which runs its
//! `ask` script, appends `waiting` and stops until a `received` event continues the run;
//! and child runs (docs/reference/runs.md, Sub-machines): a `machine` invoke, and the router machine of
//! a `model` invoke. Replies and timeouts are delivered by `reply`.
//!
//! Each run is stepped under its run lock (docs/reference/messages.md, Run lock). `recover` is what
//! `process` and `daemon` do first: it marks runs a crash left behind `interrupted` and
//! lists the `pending` runs to continue. Only `decree retry` makes an interrupted run
//! `pending` again; `continue_run` then re-runs its `onentry` scripts (docs/reference/runs.md, step 1).

pub(crate) mod child;
pub(crate) mod decide;
pub(crate) mod recover;

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::cond::{self, CondError};
use crate::events::{
    claim_event, current_state, is_transition, read_events, text, visits, Event, EVENTS_FILE,
};
use crate::layout::MESSAGE_FILE;
use crate::layout::{DECREE_DIR, PROCESSED_FILE, RUNS_DIR};
use crate::machine::{event_matches, Invoke, LoadedMachine, FAILED};
use crate::message::{Message, MessageError, RunLock, LOCK_FILE};
use crate::runtime::{
    data_env, Executor, InvokeEvent, Phase, RouterFiles, RunInfo, RuntimeError, ScriptRun,
    ROOT_STATE,
};
use decide::{REPLY_FILE, REQUEST_FILE};
use recover::{mirror_state, repair_mirror};

#[derive(Debug, thiserror::Error)]
pub enum InterpreterError {
    #[error(transparent)]
    Runtime(#[from] RuntimeError),

    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },

    /// `message.md` could not be read or written back with the mirrored `state`.
    #[error(transparent)]
    MessageFile(#[from] MessageError),

    /// `message.md` parses but cannot start or continue a run.
    #[error("{}: {message}", path.display())]
    Message { path: PathBuf, message: String },

    /// The machine breaks a rule `decree check` enforces.
    #[error("machine `{machine}`: {message}")]
    Invalid { machine: String, message: String },

    /// `resume` was called on a run that is not `pending`.
    #[error("cannot continue the run: {0}")]
    NotReceived(String),

    /// Another live process holds the run's lock: the run is `active` (docs/reference/messages.md, Run lock).
    #[error("run `{0}` is active: another process holds its lock")]
    Active(String),

    /// A `check` could not be evaluated.
    #[error("machine `{machine}`: {at}: check: {source}")]
    Check {
        machine: String,
        at: String,
        source: CondError,
    },
}

pub(crate) fn io_err(path: &Path) -> impl FnOnce(io::Error) -> InterpreterError + '_ {
    move |source| InterpreterError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// What the interpreter needs from the claimed message.
#[derive(Debug, Clone, Default)]
pub struct RunInput {
    /// Frontmatter `params`, already validated against the machine's `data`.
    pub params: serde_norway::Mapping,
    /// The message body, for a model's request and for child runs.
    pub message_body: String,
    /// Original inbox or migration filename, recorded on the claim event; `None` for a child
    /// run. For a migration (`trigger: migration`) it is also the `processed.md` ledger line.
    pub file: Option<String>,
    /// Frontmatter `depth`: 0 unless the message was emitted or is a child run.
    pub depth: u32,
}

impl RunInput {
    /// The input of the run of `message`, claimed from `file`.
    pub fn new(message: &Message, file: Option<String>) -> Self {
        RunInput {
            params: message.params(),
            message_body: message.body.clone(),
            file,
            depth: message.depth(),
        }
    }
}

/// Everything stepping a run needs beyond the run itself: the project's machines, and the
/// settings each run's executor and its child runs use (docs/reference/runs.md, Sub-machines).
pub struct Context<'a> {
    /// The directory containing `.decree/`.
    pub project_root: PathBuf,
    /// Every machine, by name: the children a run may start.
    pub machines: &'a BTreeMap<String, LoadedMachine>,
    /// Set on SIGINT or SIGTERM; stops the running script (docs/reference/messages.md, Stopping).
    pub shutdown: Arc<AtomicBool>,
}

/// How a call to the step loop ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The run entered this root-level final state, root `onexit` ran, and `run_finished`
    /// was appended.
    Finished(String),
    /// SIGINT or SIGTERM stopped a script in this state; an `interrupted` event was
    /// appended (docs/reference/messages.md, Stopping).
    Interrupted(String),
    /// The run entered this `person` state, its `ask` script ran, and a `waiting`
    /// event was appended. A reply must name `wait_id` (docs/reference/messages.md, Replies).
    Waiting { state: String, wait_id: String },
    /// The run waits in this `machine` or `model` state for child run `child`, which
    /// stopped with `outcome` before finishing: it waits for a reply itself, or was
    /// interrupted. The run continues when the child finishes (docs/reference/runs.md, Sub-machines).
    Child {
        state: String,
        child: String,
        outcome: Box<Outcome>,
    },
}

/// What a state's invoke led to (steps 2 and 3).
enum Invoked {
    Event(Decision),
    /// The run pauses for a reply.
    Wait(Outcome),
}

/// What happens after a state is entered (step 8).
enum Next {
    Stop(Outcome),
    /// Step again from the entered state, with an event already chosen (an `onentry`
    /// failure, or `done.state.<id>`) or not.
    Step(Option<Decision>),
}

/// The event chosen for the current state (step 3), and where it came from.
#[derive(Debug, Clone)]
struct Decision {
    event: String,
    source: &'static str,
    exit_code: Option<i32>,
    invalid_event: Option<String>,
    /// Why a `machine` invoke started no child: the `transition` event's `error`.
    error: Option<String>,
}

impl Decision {
    fn new(event: &str, source: &'static str, exit_code: Option<i32>) -> Self {
        Decision {
            event: event.to_string(),
            source,
            exit_code,
            invalid_event: None,
            error: None,
        }
    }

    /// `error` from a failed `onentry` script: there was no invoke, so no exit code.
    fn entry_error() -> Self {
        Decision::new("error", "exit_code", None)
    }
}

/// Steps one run. Scripts run through `executor`, which owns the run's event log.
pub struct Interpreter<'a> {
    ctx: &'a Context<'a>,
    machine: &'a LoadedMachine,
    executor: Executor,
    data: BTreeMap<String, cond::Value>,
    message_body: String,
    file: Option<String>,
    depth: u32,
    /// The atomic state the run is in, for the `interrupted` event.
    current: usize,
    /// `seq` of the last `transition` event, which names the wait id of a waiting state.
    entered_seq: u64,
}

impl<'a> Interpreter<'a> {
    pub fn new(
        ctx: &'a Context<'a>,
        machine: &'a LoadedMachine,
        executor: Executor,
        input: RunInput,
    ) -> Result<Self, InterpreterError> {
        Ok(Interpreter {
            ctx,
            machine,
            data: data_values(machine, &input.params)?,
            executor,
            message_body: input.message_body,
            file: input.file,
            depth: input.depth,
            current: 0,
            entered_seq: 0,
        })
    }

    /// Start a new run (step 1) and step it until it finishes, waits or is interrupted.
    /// The run lock is held throughout.
    pub fn start(&mut self) -> Result<Outcome, InterpreterError> {
        let _lock = self.lock()?;
        let result = self.claim_and_run();
        self.interrupt_on_signal(result)
    }

    /// Continue a `pending` run (step 1), under its run lock. After `decree retry` (the
    /// last event is a `transition` with `source: "retry"`), root `onentry` and the
    /// `onentry` of every ancestor of the state and of the state itself run again, then
    /// its invoke. Otherwise the last event is `received`, or `waiting` for a child run
    /// that has finished: take the event's transition at step 4, with `source: "person"`
    /// after a `decision` event for a reply, `source: "timeout"`, or `source: "machine"` for
    /// a child's final state; a finished router run is validated as docs/reference/runs.md, Choose:
    /// model says. Nothing is re-run then, because the run only paused.
    fn resume(&mut self) -> Result<Outcome, InterpreterError> {
        let _lock = self.lock()?;
        let result = self.continue_pending();
        self.interrupt_on_signal(result)
    }

    /// Take the run lock, which is deleted when the returned guard drops: when the run
    /// finishes, waits, is interrupted by a signal, or stepping fails.
    fn lock(&self) -> Result<RunLock, InterpreterError> {
        let info = self.executor.info();
        RunLock::acquire(&info.run_dir)
            .map_err(io_err(&info.run_dir.join(LOCK_FILE)))?
            .ok_or_else(|| InterpreterError::Active(info.run_id.clone()))
    }

    /// Turns a script stopped by SIGINT or SIGTERM into an `interrupted` event.
    fn interrupt_on_signal(
        &mut self,
        result: Result<Outcome, InterpreterError>,
    ) -> Result<Outcome, InterpreterError> {
        match result {
            Err(InterpreterError::Runtime(RuntimeError::Interrupted { script })) => {
                let state = self.machine.nodes[self.current].id.clone();
                self.append(
                    "interrupted",
                    json!({ "state": state, "cause": "signal", "script": script }),
                )?;
                Ok(Outcome::Interrupted(state))
            }
            other => other,
        }
    }

    fn claim_and_run(&mut self) -> Result<Outcome, InterpreterError> {
        let m = self.machine;
        let initial = m.root().initial.as_deref().unwrap_or_default();
        let s = m
            .find(initial)
            .filter(|&i| m.nodes[i].parent == Some(0))
            .and_then(|i| m.enter(i))
            .ok_or_else(|| self.invalid(format!("initial `{initial}` is not a root state")))?;
        self.entered_seq = self.append(
            "transition",
            json!({
                "from": null,
                "event": "claimed",
                "to": m.nodes[s].id,
                "source": "claim",
                "exit_code": null,
                "file": self.file,
            }),
        )?;
        self.current = s;
        self.mirror(s)?;
        self.enter_from_root(s)
    }

    /// Step 1's entry: run root `onentry`, then the `onentry` of each state from the root
    /// down to atomic state `s`, outermost first, then step from `s`.
    fn enter_from_root(&mut self, s: usize) -> Result<Outcome, InterpreterError> {
        let mut entering = vec![0];
        entering.extend(path_below(self.machine, 0, s));
        let entry_failed = self.run_entry(&entering)?;
        match self.after_entry(s, entry_failed)? {
            Next::Stop(outcome) => Ok(outcome),
            Next::Step(pending) => self.step_from(s, pending),
        }
    }

    fn continue_pending(&mut self) -> Result<Outcome, InterpreterError> {
        let m = self.machine;
        let events = self.read_events()?;
        let not_received = |message: &str| InterpreterError::NotReceived(message.to_string());
        let last = events
            .last()
            .ok_or_else(|| not_received("it has no events"))?;
        let last_type = text(last, "type");
        let child = text(last, "child").map(String::from);
        let s = current_state(&events)
            .and_then(|id| m.find(id))
            .ok_or_else(|| not_received("its current state is not in the machine"))?;
        let invoke = m.nodes[s].invoke.as_ref();
        self.entered_seq = events
            .iter()
            .rev()
            .find(|e| is_transition(e))
            .and_then(|e| e.get("seq"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        self.current = s;
        repair_mirror(&self.executor.info().run_dir, &events)?;

        // After `decree retry`: enter the state again, from the root.
        if last_type == Some("transition") && text(last, "source") == Some("retry") {
            return self.enter_from_root(s);
        }

        // Waiting for a child run that has finished: take its result now.
        if last_type == Some("waiting") {
            let child = child.ok_or_else(|| not_received("its last event is not `received`"))?;
            let finished = self.ctx.final_state(&child)?.ok_or_else(|| {
                InterpreterError::NotReceived(format!("child run `{child}` has not finished"))
            })?;
            let decision = match invoke {
                Some(Invoke::Machine(_)) => self.machine_finished(&child, &finished)?,
                Some(Invoke::Model(c)) => {
                    let duration_ms = self.child_duration(&child)?;
                    self.model_finished(s, c, &child, &finished, duration_ms)?
                }
                _ => return Err(not_received("its current state does not start child runs")),
            };
            return self.step_from(s, Some(decision));
        }
        if last_type != Some("received") {
            return Err(not_received("its last event is not `received`"));
        }
        let event = text(last, "event")
            .ok_or_else(|| not_received("the `received` event has no `event`"))?
            .to_string();
        if child.is_some() {
            if !matches!(invoke, Some(Invoke::Machine(_))) {
                return Err(not_received("its current state is not a `machine` state"));
            }
            return self.step_from(s, Some(Decision::new(&event, "machine", None)));
        }
        if !matches!(invoke, Some(Invoke::Person(_))) {
            return Err(not_received("its current state is not a `person` state"));
        }
        let decision = self.person_received(s, &events, &event)?;
        self.step_from(s, Some(decision))
    }

    /// Steps 2–8, from state `s`. `pending` is an event already chosen for `s` (an
    /// `onentry` failure, `done.state.<id>` or a received event), which skips the invoke.
    fn step_from(
        &mut self,
        mut s: usize,
        mut pending: Option<Decision>,
    ) -> Result<Outcome, InterpreterError> {
        let m = self.machine;
        loop {
            // 2–3. Invoke, pick the event.
            let mut decision = match pending.take() {
                Some(d) => d,
                None => match self.invoke(s)? {
                    Invoked::Event(d) => d,
                    Invoked::Wait(outcome) => return Ok(outcome),
                },
            };
            // 4. Find the target.
            let (source, target, internal) = self.select(s, &mut decision)?;
            let domain = m.transition_domain(source, target, internal);
            let t = m.enter(target).ok_or_else(|| {
                self.invalid(format!("`{}` has no valid initial", m.nodes[target].id))
            })?;
            // 5. Exit.
            let exit_failures = self.run_exit(s, domain)?;
            // 6. Record.
            let mut fields = json!({
                "from": m.nodes[s].id,
                "event": decision.event,
                "to": m.nodes[t].id,
                "source": decision.source,
                "exit_code": decision.exit_code,
            });
            if let Some(invalid) = &decision.invalid_event {
                fields["invalid_event"] = json!(invalid);
            }
            if let Some(error) = &decision.error {
                fields["error"] = json!(error);
            }
            if !exit_failures.is_empty() {
                fields["exit_failures"] = json!(exit_failures);
            }
            self.entered_seq = self.append("transition", fields)?;
            self.current = t;
            self.mirror(t)?;
            // 7. Enter.
            if let Some(line) = self.ledger_line(t) {
                self.ledger_add(&line)?;
            }
            let entry_failed = self.run_entry(&path_below(m, domain, t))?;
            // 8. Finish or loop.
            match self.after_entry(t, entry_failed)? {
                Next::Stop(outcome) => return Ok(outcome),
                Next::Step(next) => pending = next,
            }
            s = t;
        }
    }

    /// Step 8, once `t` and the states above it have been entered. `entry_failed` is true
    /// if an `onentry` script of any of them (the root included) exited non-zero.
    fn after_entry(&mut self, t: usize, entry_failed: bool) -> Result<Next, InterpreterError> {
        let m = self.machine;
        let node = &m.nodes[t];
        // A root-level final state ends the run.
        if m.is_root_final(t) {
            return self.finish(t, entry_failed).map(Next::Stop);
        }
        // An `onentry` failure is `error`, selected from the atomic state entered like any
        // other event (SCXML `error.execution`), before `done.state.<id>` or the invoke.
        if entry_failed {
            return Ok(Next::Step(Some(Decision::entry_error())));
        }
        // A nested final state raises `done.state.<parent>`, handled before anything else.
        if let Some(p) = node.parent.filter(|_| node.is_final) {
            let event = format!("done.state.{}", m.nodes[p].id);
            return Ok(Next::Step(Some(Decision::new(&event, "internal", None))));
        }
        Ok(Next::Step(None))
    }

    /// Steps 2 and 3 for atomic state `s`: run its function (docs/reference/machines.md, Invoke) and take
    /// its event. A state with no invoke produces `done`.
    fn invoke(&mut self, s: usize) -> Result<Invoked, InterpreterError> {
        let m = self.machine;
        let node = &m.nodes[s];
        match &node.invoke {
            None => Ok(Invoked::Event(Decision::new("done", "exit_code", None))),
            Some(Invoke::Script(script)) => {
                let visits = self.visits_of(s)?;
                let out = self.executor.run_invoke(m, s, script, visits)?;
                let exit_code = out.execution.exit_code;
                Ok(Invoked::Event(match out.event {
                    InvokeEvent::ExitCode(e) => Decision::new(e, "exit_code", exit_code),
                    InvokeEvent::Stdout(e) => Decision::new(&e, "stdout", exit_code),
                    InvokeEvent::Invalid(e) => Decision {
                        invalid_event: Some(e),
                        ..Decision::new("error", "stdout", exit_code)
                    },
                }))
            }
            Some(Invoke::Check(check)) => self.check(s, check).map(Invoked::Event),
            Some(Invoke::Model(c)) => self.ask_model(s, c),
            Some(Invoke::Person(c)) => self.ask(s, c),
            Some(Invoke::Machine(invoke)) => self.invoke_machine(s, invoke),
        }
    }

    /// Step 4: the state declaring the transition for the event, its target, and whether
    /// it is `type: internal`. The state's own transitions come first, then each
    /// ancestor's. An `error` that matches nothing targets `failed`; any other event that
    /// matches nothing becomes `error`.
    fn select(
        &self,
        s: usize,
        decision: &mut Decision,
    ) -> Result<(usize, usize, bool), InterpreterError> {
        let m = self.machine;
        for n in m.chain(s) {
            let edge = m.nodes[n]
                .transitions
                .iter()
                .find(|e| event_matches(&e.event, &decision.event));
            if let Some(edge) = edge {
                let target = m.find(&edge.target).ok_or_else(|| {
                    self.invalid(format!(
                        "{}: target `{}` does not exist",
                        m.state_path(n),
                        edge.target
                    ))
                })?;
                return Ok((n, target, edge.internal));
            }
        }
        if decision.event != "error" {
            decision.event = "error".to_string();
            return self.select(s, decision);
        }
        let failed = m
            .failed_state()
            .ok_or_else(|| self.invalid(format!("no root-level final state `{FAILED}`")))?;
        Ok((s, failed, false))
    }

    /// Step 8 for root-level final state `t`, after its `onentry` scripts ran. If one of
    /// them failed (and `t` is not `failed`), the run moves to `failed` first.
    fn finish(&mut self, t: usize, entry_failed: bool) -> Result<Outcome, InterpreterError> {
        let m = self.machine;
        let mut last = t;
        if entry_failed && m.nodes[t].id != FAILED {
            let failed = m
                .failed_state()
                .ok_or_else(|| self.invalid(format!("no root-level final state `{FAILED}`")))?;
            self.append(
                "transition",
                json!({
                    "from": m.nodes[t].id,
                    "event": "error",
                    "to": FAILED,
                    "source": "exit_code",
                    "exit_code": null,
                }),
            )?;
            self.current = failed;
            self.mirror(failed)?;
            if let Some(line) = self.ledger_line(t) {
                self.ledger_remove(&line)?;
            }
            // A failing `onentry` script on `failed` itself is only logged.
            self.run_entry(&[failed])?;
            last = failed;
        }
        self.run_exit_scripts(0)?;
        let duration_ms = self
            .claimed_at()?
            .map_or(0, |at| (Utc::now() - at).num_milliseconds().max(0) as u64);
        self.append(
            "run_finished",
            json!({ "state": m.nodes[last].id, "duration_ms": duration_ms }),
        )?;
        Ok(Outcome::Finished(m.nodes[last].id.clone()))
    }

    /// Run the `onentry` scripts of `states`, outermost first (index 0 is the root). A
    /// script that exits non-zero skips the rest of its own state's scripts only, as SCXML
    /// stops the failing `<onentry>` block; the other states still run theirs. Returns
    /// whether any script failed.
    fn run_entry(&mut self, states: &[usize]) -> Result<bool, InterpreterError> {
        let m = self.machine;
        let mut failed = false;
        for &n in states {
            let (name, visits) = self.script_state(n)?;
            let max_attempts = m.max_attempts(n);
            let events = m.accepted_events(n);
            for script in &m.nodes[n].onentry {
                let execution = self.executor.run_script(&ScriptRun {
                    visits,
                    max_attempts,
                    events: &events,
                    ..ScriptRun::new(script, name, Phase::OnEntry)
                })?;
                if !execution.succeeded() {
                    failed = true;
                    break;
                }
            }
        }
        Ok(failed)
    }

    /// Step 5: the `onexit` scripts of `s` and its ancestors, innermost first, stopping
    /// below `domain`. Returns the names of those that exited non-zero.
    fn run_exit(&mut self, s: usize, domain: usize) -> Result<Vec<String>, InterpreterError> {
        let m = self.machine;
        let mut failures = Vec::new();
        for n in m.chain(s).take_while(|&n| n != domain) {
            failures.extend(self.run_exit_scripts(n)?);
        }
        Ok(failures)
    }

    /// The `onexit` scripts of state `n` (0 is the root). Every one runs; returns the
    /// names of those that exited non-zero.
    fn run_exit_scripts(&mut self, n: usize) -> Result<Vec<String>, InterpreterError> {
        let m = self.machine;
        let (name, visits) = self.script_state(n)?;
        let max_attempts = m.max_attempts(n);
        let events = m.accepted_events(n);
        let mut failures = Vec::new();
        for script in &m.nodes[n].onexit {
            let execution = self.executor.run_script(&ScriptRun {
                visits,
                max_attempts,
                events: &events,
                ..ScriptRun::new(script, name, Phase::OnExit)
            })?;
            if !execution.succeeded() {
                failures.push(script.clone());
            }
        }
        Ok(failures)
    }

    /// `DECREE_STATE` and `DECREE_VISITS` for scripts of state `n`.
    fn script_state(&self, n: usize) -> Result<(&'a str, u32), InterpreterError> {
        if n == 0 {
            return Ok((ROOT_STATE, 0));
        }
        Ok((self.machine.nodes[n].id.as_str(), self.visits_of(n)?))
    }

    fn visits_of(&self, n: usize) -> Result<u32, InterpreterError> {
        let events = self.read_events()?;
        Ok(visits(&events)
            .get(&self.machine.nodes[n].id)
            .copied()
            .unwrap_or(0))
    }

    fn read_events(&self) -> Result<Vec<Event>, InterpreterError> {
        let run_dir = &self.executor.info().run_dir;
        read_events(run_dir).map_err(io_err(&run_dir.join(EVENTS_FILE)))
    }

    /// When the claim event was written.
    fn claimed_at(&self) -> Result<Option<DateTime<Utc>>, InterpreterError> {
        Ok(claim_event(&self.read_events()?)
            .and_then(|e| text(e, "ts"))
            .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
            .map(|ts| ts.with_timezone(&Utc)))
    }

    /// Append an event and return its `seq`.
    fn append(&mut self, kind: &str, fields: Value) -> Result<u64, InterpreterError> {
        let path = self.executor.info().run_dir.join(EVENTS_FILE);
        self.executor
            .events()
            .append(kind, fields)
            .map_err(io_err(&path))
    }

    /// Mirror `state` into the run's `message.md`.
    fn mirror(&self, state: usize) -> Result<(), InterpreterError> {
        let path = self.executor.info().run_dir.join(MESSAGE_FILE);
        mirror_state(&path, &self.machine.nodes[state].id)
    }

    /// The `processed.md` line to write before entering `t`: a migration entering a final
    /// state other than `failed` (docs/reference/messages.md, Migrations, rule 5). Only a root-level final
    /// state finishes the run, so a nested one writes nothing.
    fn ledger_line(&self, t: usize) -> Option<String> {
        let finishes = self.machine.is_root_final(t) && self.machine.nodes[t].id != FAILED;
        let migration = self.executor.info().trigger == "migration";
        self.file.clone().filter(|_| migration && finishes)
    }

    fn ledger_path(&self) -> PathBuf {
        self.executor
            .info()
            .project_root
            .join(DECREE_DIR)
            .join(PROCESSED_FILE)
    }

    fn ledger_add(&self, line: &str) -> Result<(), InterpreterError> {
        let path = self.ledger_path();
        let mut text = read_or_empty(&path)?;
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(line);
        text.push('\n');
        write_replace(&path, text.as_bytes())
    }

    fn ledger_remove(&self, line: &str) -> Result<(), InterpreterError> {
        let path = self.ledger_path();
        let text: String = read_or_empty(&path)?
            .lines()
            .filter(|l| l.trim() != line)
            .map(|l| format!("{l}\n"))
            .collect();
        write_replace(&path, text.as_bytes())
    }

    fn invalid(&self, message: String) -> InterpreterError {
        InterpreterError::Invalid {
            machine: self.machine.id.clone(),
            message,
        }
    }
}

impl Context<'_> {
    /// `.decree/runs/`.
    pub fn runs_dir(&self) -> PathBuf {
        self.project_root.join(DECREE_DIR).join(RUNS_DIR)
    }

    /// The executor for run `run_id` of machine `m`, whose folder exists. A run whose
    /// folder holds `request.json` is a router run: its scripts get `DECREE_REQUEST` and
    /// `DECREE_REPLY`.
    pub fn executor(
        &self,
        m: &LoadedMachine,
        run_id: &str,
        trigger: &str,
        params: &serde_norway::Mapping,
        parent: Option<&str>,
    ) -> Result<Executor, InterpreterError> {
        let run_dir = self.runs_dir().join(run_id);
        let router = run_dir.join(REQUEST_FILE).is_file().then(|| RouterFiles {
            request: run_dir.join(REQUEST_FILE),
            reply: run_dir.join(REPLY_FILE),
        });
        let info = RunInfo {
            project_root: self.project_root.clone(),
            run_dir,
            run_id: run_id.to_string(),
            machine: m.id.clone(),
            trigger: trigger.to_string(),
            data: data_env(&m.data, params),
            parent: parent.map(String::from),
            router,
        };
        Ok(Executor::open(info, Arc::clone(&self.shutdown))?)
    }
}

/// The run's `data`: each `params` value, else the default (docs/reference/machines.md, Keys).
fn data_values(
    m: &LoadedMachine,
    params: &serde_norway::Mapping,
) -> Result<BTreeMap<String, cond::Value>, InterpreterError> {
    let mut data = BTreeMap::new();
    for (name, spec) in &m.data {
        let value = params.get(name.as_str()).unwrap_or(&spec.default);
        let value = match (value, value.as_i64()) {
            (serde_norway::Value::String(s), _) => cond::Value::Str(s.clone()),
            (serde_norway::Value::Bool(b), _) => cond::Value::Bool(*b),
            (_, Some(n)) => cond::Value::Int(n),
            _ => {
                return Err(InterpreterError::Invalid {
                    machine: m.id.clone(),
                    message: format!("data `{name}` is not a string, int or bool"),
                })
            }
        };
        data.insert(name.clone(), value);
    }
    Ok(data)
}

/// The states from just below `domain` down to `t`, outermost first: the states a
/// transition with that domain enters.
fn path_below(m: &LoadedMachine, domain: usize, t: usize) -> Vec<usize> {
    let mut path: Vec<usize> = m.chain(t).take_while(|&n| n != domain).collect();
    path.reverse();
    path
}

fn read_or_empty(path: &Path) -> Result<String, InterpreterError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(io_err(path)(e)),
    }
}

/// Write `.<name>.tmp` in the same directory, then rename it over `path`.
fn write_replace(path: &Path, bytes: &[u8]) -> Result<(), InterpreterError> {
    crate::message::write_replace(path, bytes)
        .map_err(|(path, source)| InterpreterError::Io { path, source })
}

#[cfg(test)]
mod tests;
