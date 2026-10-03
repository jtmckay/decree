//! The step loop (spec section 7): moves one run through one machine with SCXML's exit
//! and entry order, runs each state's invoke, and appends every step to the run's
//! `events.jsonl`. State, status and visits are derived from that log (section 4, Source
//! of truth).
//!
//! Interpreted here: the whole section 5 subset. Transitions on compound states, with
//! events bubbling from the atomic state outward; `type: internal`; final states at any
//! level, a nested one raising `done.state.<parent>`; `choose: person`, which runs its
//! `ask` script, appends `waiting` and stops until a `received` event continues the run;
//! and child runs (section 7, Sub-machines): a `machine` invoke, and the router machine of
//! a `choose: model` invoke. Delivering replies is ticket M4.3; `decree retry` and the run
//! lock are ticket M4.2.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};

use crate::cond::{self, CondError};
use crate::config::{DECREE_DIR, INBOX_DIR, PROCESSED_FILE, RUNS_DIR};
use crate::machine::{
    event_matches, CheckInvoke, ChooseInvoke, ChooseKind, Invoke, LoadedMachine, MachineInvoke,
    FAILED,
};
use crate::runtime::{
    data_env, timestamp, EventLog, Executor, InvokeEvent, Phase, RouterFiles, RunInfo,
    RuntimeError, ScriptRun, EVENTS_FILE, MESSAGE_FILE, RECEIVED_DIR, ROOT_STATE,
};

/// The JSON file, in the run folder, mapping each option of the `choose: person` state the
/// run waits in to its description: what `DECREE_CHOICES` names (section 6).
pub const CHOICES_FILE: &str = "choices.json";

/// The request a `choose: model` invoke writes in its router run's folder (section 7).
pub const REQUEST_FILE: &str = "request.json";

/// Where a router machine writes its reply, in its run folder (section 7).
pub const REPLY_FILE: &str = "reply.json";

#[derive(Debug, thiserror::Error)]
pub enum InterpreterError {
    #[error(transparent)]
    Runtime(#[from] RuntimeError),

    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },

    /// `message.md` could not be read or written back with the mirrored `state`.
    #[error("{}: {message}", path.display())]
    Message { path: PathBuf, message: String },

    /// The machine breaks a rule `decree check` enforces.
    #[error("machine `{machine}`: {message}")]
    Invalid { machine: String, message: String },

    /// `resume` was called on a run that is not waiting for its `received` event.
    #[error("cannot continue the run: {0}")]
    NotReceived(String),

    /// A `check` could not be evaluated.
    #[error("machine `{machine}`: {at}: check: {source}")]
    Check {
        machine: String,
        at: String,
        source: CondError,
    },

    /// The state invokes a type this interpreter does not run yet.
    #[error("machine `{machine}`: {at}: running `{kind}` invokes is not implemented yet")]
    NotSupported {
        machine: String,
        at: String,
        kind: &'static str,
    },
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> InterpreterError + '_ {
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

/// Everything stepping a run needs beyond the run itself: the project's machines, and the
/// settings each run's executor and its child runs use (section 7, Sub-machines).
pub struct Context<'a> {
    /// The directory containing `.decree/`.
    pub project_root: PathBuf,
    pub shared_source: Option<PathBuf>,
    /// Every machine, by name: the children a run may start.
    pub machines: &'a BTreeMap<String, LoadedMachine>,
    /// Config `default_router`.
    pub default_router: Option<String>,
    pub max_attempts: u32,
    pub max_depth: u32,
    pub max_log_size: u64,
    /// Set on SIGINT or SIGTERM; stops the running script (section 4, Stopping).
    pub shutdown: Arc<AtomicBool>,
}

/// How a call to the step loop ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The run entered this root-level final state, root `onexit` ran, and `run_finished`
    /// was appended.
    Finished(String),
    /// SIGINT or SIGTERM stopped a script in this state; an `interrupted` event was
    /// appended (section 4, Stopping).
    Interrupted(String),
    /// The run entered this `choose: person` state, its `ask` script ran, and a `waiting`
    /// event was appended. A reply must name `wait_id` (section 4, Replies).
    Waiting { state: String, wait_id: String },
    /// The run waits in this `machine` or `choose: model` state for child run `child`, which
    /// stopped with `outcome` before finishing: it waits for a reply itself, or was
    /// interrupted. The run continues when the child finishes (section 7, Sub-machines).
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

/// A child run this run started, and how stepping it ended.
struct Child {
    id: String,
    outcome: Outcome,
    /// Wall time of stepping the child.
    duration_ms: u64,
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
    /// A root `onentry` script failed: the target is `failed`, whatever the state handles.
    to_failed: bool,
}

impl Decision {
    fn new(event: &str, source: &'static str, exit_code: Option<i32>) -> Self {
        Decision {
            event: event.to_string(),
            source,
            exit_code,
            invalid_event: None,
            error: None,
            to_failed: false,
        }
    }

    /// `error` from a failed `onentry` script: there was no invoke, so no exit code.
    fn entry_error(root: bool) -> Self {
        Decision {
            to_failed: root,
            ..Decision::new("error", "exit_code", None)
        }
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
    pub fn start(&mut self) -> Result<Outcome, InterpreterError> {
        let result = self.claim_and_run();
        self.interrupt_on_signal(result)
    }

    /// Continue a `pending` run (step 1): its last event is `received`, or `waiting` for a
    /// child run that has finished. Take the event's transition at step 4: with
    /// `source: "person"` after a `decision` event for a reply, `source: "timeout"`, or
    /// `source: "machine"` for a child's final state; a finished router run is validated as
    /// section 7, Choose: model says. Nothing is re-run, because the run only paused.
    pub fn resume(&mut self) -> Result<Outcome, InterpreterError> {
        let result = self.continue_received();
        self.interrupt_on_signal(result)
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

        let mut entering = vec![0];
        entering.extend(path_below(m, 0, s));
        let failed_at = self.run_entry(&entering)?;
        match self.after_entry(s, failed_at)? {
            Next::Stop(outcome) => Ok(outcome),
            Next::Step(pending) => self.step_from(s, pending),
        }
    }

    fn continue_received(&mut self) -> Result<Outcome, InterpreterError> {
        let m = self.machine;
        let events = self.read_events()?;
        let not_received = |message: &str| InterpreterError::NotReceived(message.to_string());
        let last = events
            .last()
            .ok_or_else(|| not_received("it has no events"))?;
        let last_type = last.get("type").and_then(Value::as_str);
        let child = last.get("child").and_then(Value::as_str).map(String::from);
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

        // Waiting for a child run that has finished: take its result now.
        if last_type == Some("waiting") {
            let child = child.ok_or_else(|| not_received("its last event is not `received`"))?;
            let finished = self.child_final(&child)?.ok_or_else(|| {
                InterpreterError::NotReceived(format!("child run `{child}` has not finished"))
            })?;
            let decision = match invoke {
                Some(Invoke::Machine(_)) => self.machine_finished(&child, &finished)?,
                Some(Invoke::Choose(c)) if c.choose == ChooseKind::Model => {
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
        let event = last
            .get("event")
            .and_then(Value::as_str)
            .ok_or_else(|| not_received("the `received` event has no `event`"))?
            .to_string();
        if child.is_some() {
            if !matches!(invoke, Some(Invoke::Machine(_))) {
                return Err(not_received("its current state is not a `machine` state"));
            }
            return self.step_from(s, Some(Decision::new(&event, "machine", None)));
        }
        if invoke.and_then(|i| i.choose(ChooseKind::Person)).is_none() {
            return Err(not_received(
                "its current state is not a `choose: person` state",
            ));
        }
        let timed_out = last.get("timed_out").and_then(Value::as_bool) == Some(true);
        let reply = last.get("file").and_then(Value::as_str).map(String::from);
        // `DECREE_RECEIVED`: the last reply, not a timeout, which has no file.
        let run_dir = self.executor.info().run_dir.clone();
        self.executor.received = events
            .iter()
            .rev()
            .filter(|e| e.get("type").and_then(Value::as_str) == Some("received"))
            .find_map(|e| e.get("file").and_then(Value::as_str))
            .map(|file| run_dir.join(RECEIVED_DIR).join(file));
        let decision = if timed_out {
            Decision::new(&event, "timeout", None)
        } else {
            let mut fields = json!({
                "state": m.nodes[s].id,
                "kind": "person",
                "event": event,
                "options": option_names(m, s),
            });
            if let Some(reply) = reply {
                fields["reply"] = json!(reply);
            }
            self.append("decision", fields)?;
            Decision::new(&event, "person", None)
        };
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
            let domain = transition_domain(m, source, target, internal);
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
            let failed_at = self.run_entry(&path_below(m, domain, t))?;
            // 8. Finish or loop.
            match self.after_entry(t, failed_at)? {
                Next::Stop(outcome) => return Ok(outcome),
                Next::Step(next) => pending = next,
            }
            s = t;
        }
    }

    /// Step 8, once `t` and the states above it have been entered. `failed_at` is the state
    /// whose `onentry` script failed, if one did.
    fn after_entry(
        &mut self,
        t: usize,
        failed_at: Option<usize>,
    ) -> Result<Next, InterpreterError> {
        let m = self.machine;
        let node = &m.nodes[t];
        // A root-level final state ends the run.
        if is_root_final(m, t) {
            return self.finish(t, failed_at.is_some()).map(Next::Stop);
        }
        // An `onentry` failure is `error`, resolved on the state being entered, before
        // `done.state.<id>` or the invoke.
        if let Some(n) = failed_at {
            return Ok(Next::Step(Some(Decision::entry_error(n == 0))));
        }
        // A nested final state raises `done.state.<parent>`, handled before anything else.
        if let Some(p) = node.parent.filter(|_| node.is_final) {
            let event = format!("done.state.{}", m.nodes[p].id);
            return Ok(Next::Step(Some(Decision::new(&event, "internal", None))));
        }
        Ok(Next::Step(None))
    }

    /// Step 2 for a `choose: person` state (section 4, Replies): write its options to
    /// `choices.json`, run its `ask` script with the wait id, then append `waiting` and stop.
    /// An `ask` script that exits non-zero gives `error` instead, since nobody was told.
    fn ask(&mut self, s: usize, choose: &ChooseInvoke) -> Result<Invoked, InterpreterError> {
        let m = self.machine;
        let node = &m.nodes[s];
        let ask = choose
            .ask
            .as_deref()
            .ok_or_else(|| self.invalid(format!("`{}` has no `ask` script", node.id)))?;
        let options: BTreeMap<&str, &str> = m
            .options(s)
            .map(|e| {
                (
                    e.event.as_str(),
                    e.description.as_deref().unwrap_or_default(),
                )
            })
            .collect();
        let choices = self.executor.info().run_dir.join(CHOICES_FILE);
        let text = serde_json::to_string_pretty(&options).unwrap_or_default() + "\n";
        write_replace(&choices, text.as_bytes())?;

        let wait_id = self.wait_id();
        let visits = self.visits_of(s)?;
        let events = m.accepted_events(s);
        let execution = self.executor.run_script(&ScriptRun {
            visits,
            events: &events,
            wait_id: &wait_id,
            question: choose.question.as_deref().unwrap_or_default(),
            choices: &choices,
            ..ScriptRun::new(ask, &node.id, Phase::Invoke)
        })?;
        if !execution.succeeded() {
            return Ok(Invoked::Event(Decision::new(
                "error",
                "exit_code",
                execution.exit_code,
            )));
        }

        let timeout_at = choose.timeout_s.map(|secs| {
            let secs = i64::try_from(secs).unwrap_or(i64::MAX);
            let deadline = chrono::TimeDelta::try_seconds(secs)
                .and_then(|d| Utc::now().checked_add_signed(d))
                .unwrap_or(DateTime::<Utc>::MAX_UTC);
            timestamp(deadline)
        });
        self.append(
            "waiting",
            json!({
                "state": node.id,
                "wait_id": wait_id,
                "options": option_names(m, s),
                "timeout_at": timeout_at,
            }),
        )?;
        Ok(Invoked::Wait(Outcome::Waiting {
            state: node.id.clone(),
            wait_id,
        }))
    }

    /// `<run id>.w<seq>`, where `seq` is that of the `transition` event that entered the
    /// current state.
    fn wait_id(&self) -> String {
        format!("{}.w{}", self.executor.info().run_id, self.entered_seq)
    }

    /// Steps 2 and 3 for atomic state `s`: run its function (section 5, Invoke) and take
    /// its event. A state with no invoke produces `done`.
    fn invoke(&mut self, s: usize) -> Result<Invoked, InterpreterError> {
        let m = self.machine;
        let node = &m.nodes[s];
        match &node.invoke {
            None => Ok(Invoked::Event(Decision::new("done", "exit_code", None))),
            Some(Invoke::Script(_)) => {
                let visits = self.visits_of(s)?;
                let Some(out) = self.executor.run_invoke(m, s, visits)? else {
                    unreachable!("a script invoke runs a script");
                };
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
            Some(Invoke::Choose(c)) if c.choose == ChooseKind::Person => self.ask(s, c),
            Some(Invoke::Choose(c)) => self.choose_model(s, c),
            Some(Invoke::Machine(invoke)) => self.invoke_machine(s, invoke),
        }
    }

    /// Step 2 for a `machine` invoke (section 7, Sub-machines): run the machine as a child
    /// run. Its final state is the event, `failed` as `error`.
    fn invoke_machine(
        &mut self,
        s: usize,
        invoke: &MachineInvoke,
    ) -> Result<Invoked, InterpreterError> {
        let child = match self.run_child(s, &invoke.machine, &invoke.params, None)? {
            Ok(child) => child,
            Err(reason) => {
                return Ok(Invoked::Event(Decision {
                    error: Some(reason),
                    ..Decision::new("error", "machine", None)
                }))
            }
        };
        match child.outcome {
            Outcome::Finished(state) => {
                self.machine_finished(&child.id, &state).map(Invoked::Event)
            }
            outcome => Ok(self.wait_for_child(s, child.id, outcome)),
        }
    }

    /// Step 2 for a `choose: model` invoke (section 7, Choose: model): write the request,
    /// run the router machine as a child run, and validate its reply.
    fn choose_model(
        &mut self,
        s: usize,
        _choose: &ChooseInvoke,
    ) -> Result<Invoked, InterpreterError> {
        Err(self.not_supported(s, "choose: model"))
    }

    /// A router run finished in `state`: validate its reply and append the `decision` event.
    fn model_finished(
        &mut self,
        s: usize,
        _choose: &ChooseInvoke,
        _child: &str,
        _state: &str,
        _duration_ms: u64,
    ) -> Result<Decision, InterpreterError> {
        Err(self.not_supported(s, "choose: model"))
    }

    /// The parent's side of a child that stopped before finishing: the run stays `waiting`.
    fn wait_for_child(&self, s: usize, child: String, outcome: Outcome) -> Invoked {
        Invoked::Wait(Outcome::Child {
            state: self.machine.nodes[s].id.clone(),
            child,
            outcome: Box::new(outcome),
        })
    }

    /// A `machine` invoke's child reached root final state `state`: append the `received`
    /// event, whose event is that state (`failed` as `error`).
    fn machine_finished(&mut self, child: &str, state: &str) -> Result<Decision, InterpreterError> {
        let event = if state == FAILED { "error" } else { state };
        self.append("received", json!({ "event": event, "child": child }))?;
        Ok(Decision::new(event, "machine", None))
    }

    /// Start machine `name` as a child run of state `s` and step it until it finishes, waits
    /// or is interrupted (section 7, Sub-machines). The child's `message.md` holds `machine`,
    /// `id`, `parent`, `depth`, `trigger: invoke`, any `params` and this run's body; a
    /// `request` is written to its `request.json`, which makes it a router run. Appends the
    /// `waiting` event naming the child first. `Err` holds why no child started: its
    /// `depth` would exceed `max_depth`.
    fn run_child(
        &mut self,
        s: usize,
        name: &str,
        params: &serde_norway::Mapping,
        request: Option<&Value>,
    ) -> Result<Result<Child, String>, InterpreterError> {
        let ctx = self.ctx;
        let machine = ctx
            .machines
            .get(name)
            .ok_or_else(|| self.invalid(format!("machine `{name}` does not exist")))?;
        let depth = self.depth + 1;
        if depth > ctx.max_depth {
            return Ok(Err(format!("max_depth {} reached", ctx.max_depth)));
        }
        let (id, run_dir) = create_run_dir(&ctx.project_root.join(DECREE_DIR))?;
        let parent = self.executor.info().run_id.clone();
        let mut frontmatter = serde_norway::Mapping::new();
        frontmatter.insert("machine".into(), name.into());
        frontmatter.insert("id".into(), id.as_str().into());
        frontmatter.insert("parent".into(), parent.as_str().into());
        frontmatter.insert("depth".into(), depth.into());
        frontmatter.insert("trigger".into(), "invoke".into());
        if !params.is_empty() {
            frontmatter.insert("params".into(), params.clone().into());
        }
        let message_path = run_dir.join(MESSAGE_FILE);
        let yaml =
            serde_norway::to_string(&frontmatter).map_err(|e| InterpreterError::Message {
                path: message_path.clone(),
                message: e.to_string(),
            })?;
        let message = format!("---\n{yaml}---\n{}", self.message_body);
        write_replace(&message_path, message.as_bytes())?;
        if let Some(request) = request {
            let path = run_dir.join(REQUEST_FILE);
            let text = serde_json::to_string_pretty(request).unwrap_or_default() + "\n";
            write_replace(&path, text.as_bytes())?;
        }

        self.append(
            "waiting",
            json!({ "state": self.machine.nodes[s].id, "child": id }),
        )?;
        let executor = ctx.executor(machine, &id, "invoke", params, Some(&parent))?;
        let input = RunInput {
            params: params.clone(),
            message_body: self.message_body.clone(),
            file: None,
            depth,
        };
        let started = Instant::now();
        let outcome = Interpreter::new(ctx, machine, executor, input)?.start()?;
        Ok(Ok(Child {
            id,
            outcome,
            duration_ms: started.elapsed().as_millis() as u64,
        }))
    }

    /// The root final state child run `child` reached, or `None` if it has not finished.
    fn child_final(&self, child: &str) -> Result<Option<String>, InterpreterError> {
        let dir = self.ctx.runs_dir().join(child);
        let events = read_events(&dir).map_err(io_err(&dir.join(EVENTS_FILE)))?;
        let machine = events
            .first()
            .and_then(|e| e.get("machine"))
            .and_then(Value::as_str)
            .and_then(|name| self.ctx.machines.get(name));
        let Some(machine) = machine else {
            return Ok(None);
        };
        Ok((run_status(machine, &events, false) == RunStatus::Finished)
            .then(|| current_state(&events).map(String::from))
            .flatten())
    }

    /// How long finished child run `child` took: its `run_finished` event's `duration_ms`.
    fn child_duration(&self, child: &str) -> Result<u64, InterpreterError> {
        let dir = self.ctx.runs_dir().join(child);
        let events = read_events(&dir).map_err(io_err(&dir.join(EVENTS_FILE)))?;
        Ok(events
            .iter()
            .rev()
            .find(|e| e.get("type").and_then(Value::as_str) == Some("run_finished"))
            .and_then(|e| e.get("duration_ms"))
            .and_then(Value::as_u64)
            .unwrap_or(0))
    }

    /// Section 7, Check: evaluate the condition against the input, `data` and visits,
    /// append a `decision` event, and produce `yes` or `no`. No script runs.
    fn check(&mut self, s: usize, check: &CheckInvoke) -> Result<Decision, InterpreterError> {
        let m = self.machine;
        let events = self.read_events()?;
        let input = match check.check.shape() {
            Ok((cond::Subject::Matches(_), _)) => {
                self.input_text(&events, check.input.as_deref())?
            }
            _ => String::new(),
        };
        let result = check
            .check
            .eval(&cond::Facts {
                data: &self.data,
                visits: &visits(&events),
                confidence: &confidences(&events),
                input: &input,
            })
            .map_err(|source| InterpreterError::Check {
                machine: m.id.clone(),
                at: m.state_path(s),
                source,
            })?;
        let event = if result { "yes" } else { "no" };
        self.append(
            "decision",
            json!({
                "state": m.nodes[s].id,
                "kind": "check",
                "event": event,
                "condition": check.check,
            }),
        )?;
        Ok(Decision::new(event, "check", None))
    }

    /// Section 5, Input: the log of the latest invoke script of state `input`, or, without
    /// `input`, of the most recent invoke script in the run. Empty if none has run.
    fn input_text(
        &self,
        events: &[Map<String, Value>],
        input: Option<&str>,
    ) -> Result<String, InterpreterError> {
        let log = events
            .iter()
            .rev()
            .filter(|e| e.get("type").and_then(Value::as_str) == Some("script"))
            .filter(|e| e.get("phase").and_then(Value::as_str) == Some(Phase::Invoke.as_str()))
            .find(|e| input.is_none_or(|i| e.get("state").and_then(Value::as_str) == Some(i)))
            .and_then(|e| e.get("log").and_then(Value::as_str));
        let Some(log) = log else {
            return Ok(String::new());
        };
        let path = self.executor.info().run_dir.join(log);
        let bytes = fs::read(&path).map_err(io_err(&path))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
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
        if !decision.to_failed {
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

    /// Run the `onentry` scripts of `states`, in order (index 0 is the root). Stops at the
    /// first that exits non-zero and returns the state it belongs to.
    fn run_entry(&mut self, states: &[usize]) -> Result<Option<usize>, InterpreterError> {
        let m = self.machine;
        for &n in states {
            let (name, visits) = self.script_state(n)?;
            let max_attempts = self.executor.max_attempts(m, n);
            let events = m.accepted_events(n);
            for script in &m.nodes[n].onentry {
                let execution = self.executor.run_script(&ScriptRun {
                    visits,
                    max_attempts,
                    events: &events,
                    ..ScriptRun::new(script, name, Phase::OnEntry)
                })?;
                if !execution.succeeded() {
                    return Ok(Some(n));
                }
            }
        }
        Ok(None)
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
        let max_attempts = self.executor.max_attempts(m, n);
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

    fn read_events(&self) -> Result<Vec<Map<String, Value>>, InterpreterError> {
        let run_dir = &self.executor.info().run_dir;
        read_events(run_dir).map_err(io_err(&run_dir.join(EVENTS_FILE)))
    }

    /// When the claim event was written.
    fn claimed_at(&self) -> Result<Option<DateTime<Utc>>, InterpreterError> {
        Ok(self
            .read_events()?
            .iter()
            .find(|e| e["type"] == "transition" && e["source"] == "claim")
            .and_then(|e| e["ts"].as_str())
            .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
            .map(|ts| ts.with_timezone(&Utc)))
    }

    /// Append an event and return its `seq`.
    fn append(&mut self, kind: &str, fields: Value) -> Result<u64, InterpreterError> {
        let path = self.executor.info().run_dir.join(EVENTS_FILE);
        let Value::Object(fields) = fields else {
            unreachable!("event fields are a JSON object");
        };
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
    /// state other than `failed` (section 4, Migrations, rule 5). Only a root-level final
    /// state finishes the run, so a nested one writes nothing.
    fn ledger_line(&self, t: usize) -> Option<String> {
        let finishes = is_root_final(self.machine, t) && self.machine.nodes[t].id != FAILED;
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

    fn not_supported(&self, s: usize, kind: &'static str) -> InterpreterError {
        InterpreterError::NotSupported {
            machine: self.machine.id.clone(),
            at: self.machine.state_path(s),
            kind,
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
            shared_source: self.shared_source.clone(),
            run_dir,
            run_id: run_id.to_string(),
            machine: m.id.clone(),
            trigger: trigger.to_string(),
            data: data_env(&m.data, params),
            max_attempts: self.max_attempts,
            max_log_size: self.max_log_size,
            parent: parent.map(String::from),
            router,
        };
        Ok(Executor::open(info, Arc::clone(&self.shutdown))?)
    }

    /// A run's status (section 4, Run status), where a run left `waiting` for a child that
    /// has already finished is `pending` (section 7, Sub-machines).
    pub fn status(
        &self,
        m: &LoadedMachine,
        events: &[Map<String, Value>],
        lock_alive: bool,
    ) -> RunStatus {
        let status = run_status(m, events, lock_alive);
        let child = events
            .last()
            .filter(|e| e.get("type").and_then(Value::as_str) == Some("waiting"))
            .and_then(|e| e.get("child"))
            .and_then(Value::as_str);
        let Some(child) = child.filter(|_| status == RunStatus::Waiting) else {
            return status;
        };
        let events = read_events(&self.runs_dir().join(child)).unwrap_or_default();
        let finished = events
            .first()
            .and_then(|e| e.get("machine"))
            .and_then(Value::as_str)
            .and_then(|name| self.machines.get(name))
            .is_some_and(|cm| run_status(cm, &events, false) == RunStatus::Finished);
        if finished {
            RunStatus::Pending
        } else {
            status
        }
    }
}

/// Continue `pending` run `run_id` from its folder (section 7, step 1): its last event is
/// `received`, or `waiting` for a child run that has finished. A child run that finishes
/// continues its parent, if the parent waits for it, and so on up: the result is that of
/// the last run continued.
pub fn continue_run(ctx: &Context, run_id: &str) -> Result<Outcome, InterpreterError> {
    let run_dir = ctx.runs_dir().join(run_id);
    let message_path = run_dir.join(MESSAGE_FILE);
    let bytes = fs::read(&message_path).map_err(io_err(&message_path))?;
    let (frontmatter, body) = split_message(&message_path, &bytes)?;
    let text = |key: &str| {
        frontmatter
            .get(key)
            .and_then(|v| v.as_str())
            .map(String::from)
    };
    let name =
        text("machine")
            .or_else(|| text("routine"))
            .ok_or_else(|| InterpreterError::Message {
                path: message_path.clone(),
                message: "frontmatter names no `machine`".to_string(),
            })?;
    let machine = ctx
        .machines
        .get(&name)
        .ok_or_else(|| InterpreterError::Invalid {
            machine: name.clone(),
            message: "no such machine".to_string(),
        })?;
    let trigger = text("trigger").unwrap_or_else(|| "inbox".to_string());
    let parent = text("parent");
    let params = match frontmatter.get("params") {
        Some(serde_norway::Value::Mapping(params)) => params.clone(),
        _ => serde_norway::Mapping::new(),
    };
    let depth = frontmatter
        .get("depth")
        .and_then(|v| v.as_u64())
        .map_or(0, |d| u32::try_from(d).unwrap_or(u32::MAX));
    let events = read_events(&run_dir).map_err(io_err(&run_dir.join(EVENTS_FILE)))?;
    let file = events
        .iter()
        .find(|e| is_transition(e) && e.get("source").and_then(Value::as_str) == Some("claim"))
        .and_then(|e| e.get("file"))
        .and_then(Value::as_str)
        .map(String::from);

    let executor = ctx.executor(machine, run_id, &trigger, &params, parent.as_deref())?;
    let input = RunInput {
        params,
        message_body: body,
        file,
        depth,
    };
    let outcome = Interpreter::new(ctx, machine, executor, input)?.resume()?;
    let Some(parent) = parent.filter(|_| trigger == "invoke") else {
        return Ok(outcome);
    };
    if !matches!(outcome, Outcome::Finished(_)) {
        return Ok(outcome);
    }
    let parent_dir = ctx.runs_dir().join(&parent);
    let parent_events = read_events(&parent_dir).map_err(io_err(&parent_dir.join(EVENTS_FILE)))?;
    let waits_for_this = parent_events.last().is_some_and(|e| {
        e.get("type").and_then(Value::as_str) == Some("waiting")
            && e.get("child").and_then(Value::as_str) == Some(run_id)
    });
    if waits_for_this {
        continue_run(ctx, &parent)
    } else {
        Ok(outcome)
    }
}

/// Create `runs/<id>/` for a new run (section 4, Frontmatter keys): `id` is the UTC time,
/// `YYYYMMDDTHHMMSSZ`, then `-` and 6 lowercase hex chars, the low 24 bits of (sub-second
/// nanoseconds XOR process id). While that id exists in `inbox/` or `runs/`, add 1.
pub fn create_run_dir(decree_dir: &Path) -> Result<(String, PathBuf), InterpreterError> {
    let now = Utc::now();
    let stamp = now.format("%Y%m%dT%H%M%SZ");
    let mut low = (now.timestamp_subsec_nanos() ^ std::process::id()) & 0xff_ffff;
    let runs = decree_dir.join(RUNS_DIR);
    fs::create_dir_all(&runs).map_err(io_err(&runs))?;
    for _ in 0..=0xff_ffff {
        let id = format!("{stamp}-{low:06x}");
        let dir = runs.join(&id);
        let queued = decree_dir.join(INBOX_DIR).join(format!("{id}.md")).exists();
        if !queued {
            match fs::create_dir(&dir) {
                Ok(()) => return Ok((id, dir)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(io_err(&dir)(e)),
            }
        }
        low = (low + 1) & 0xff_ffff;
    }
    Err(io_err(&runs)(io::Error::other(format!(
        "every id for {stamp} is taken"
    ))))
}

/// A final state whose parent is the root: entering it ends the run.
fn is_root_final(m: &LoadedMachine, n: usize) -> bool {
    m.nodes[n].is_final && m.nodes[n].parent == Some(0)
}

/// The options of `choose` state `n`, in name order (section 5, Choices).
fn option_names(m: &LoadedMachine, n: usize) -> Vec<String> {
    m.options(n).map(|e| e.event.clone()).collect()
}

/// The run's `data`: each `params` value, else the default (section 5, Keys).
fn data_values(
    m: &LoadedMachine,
    params: &serde_norway::Mapping,
) -> Result<BTreeMap<String, cond::Value>, InterpreterError> {
    let mut data = BTreeMap::new();
    for (name, spec) in &m.data {
        let value = params.get(name.as_str()).unwrap_or(&spec.default);
        let value = match value {
            serde_norway::Value::String(s) => cond::Value::Str(s.clone()),
            serde_norway::Value::Bool(b) => cond::Value::Bool(*b),
            serde_norway::Value::Number(n) if n.as_i64().is_some() => {
                cond::Value::Int(n.as_i64().unwrap_or_default())
            }
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

/// The transition domain (section 5, Rules): the deepest compound state, or the root, that
/// is a proper ancestor of both the state declaring the transition and its target. For a
/// `type: internal` transition from a compound state to one of its descendants, it is the
/// source itself, so the source is neither exited nor re-entered.
fn transition_domain(m: &LoadedMachine, source: usize, target: usize, internal: bool) -> usize {
    if internal && m.is_compound(source) && m.chain(target).skip(1).any(|a| a == source) {
        return source;
    }
    m.chain(source)
        .skip(1)
        .find(|&a| m.chain(target).skip(1).any(|b| b == a))
        .unwrap_or(0)
}

/// The states from just below `domain` down to `t`, outermost first: the states a
/// transition with that domain enters.
fn path_below(m: &LoadedMachine, domain: usize, t: usize) -> Vec<usize> {
    let mut path: Vec<usize> = m.chain(t).take_while(|&n| n != domain).collect();
    path.reverse();
    path
}

/// Every event in `runs/<id>/events.jsonl`, in order. A missing file holds no events.
pub fn read_events(run_dir: &Path) -> io::Result<Vec<Map<String, Value>>> {
    let text = match fs::read_to_string(run_dir.join(EVENTS_FILE)) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| match serde_json::from_str(line) {
            Ok(Value::Object(event)) => Ok(event),
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "event is not a JSON object",
            )),
            Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
        })
        .collect()
}

fn is_transition(event: &Map<String, Value>) -> bool {
    event.get("type").and_then(Value::as_str) == Some("transition")
}

/// `visits.<state>` (section 7, Visits): the `transition` events whose `to` is that state,
/// except `source: "attempt"`.
pub fn visits(events: &[Map<String, Value>]) -> BTreeMap<String, u32> {
    let mut visits = BTreeMap::new();
    for event in events.iter().filter(|e| is_transition(e)) {
        if event.get("source").and_then(Value::as_str) == Some("attempt") {
            continue;
        }
        if let Some(to) = event.get("to").and_then(Value::as_str) {
            *visits.entry(to.to_string()).or_insert(0) += 1;
        }
    }
    visits
}

/// Each state's confidence from its latest `decision` event, 0 when that event has none
/// (section 5, Conditions). A state with no `decision` event is missing.
pub fn confidences(events: &[Map<String, Value>]) -> BTreeMap<String, f64> {
    let mut confidences = BTreeMap::new();
    for event in events
        .iter()
        .filter(|e| e.get("type").and_then(Value::as_str) == Some("decision"))
    {
        if let Some(state) = event.get("state").and_then(Value::as_str) {
            let confidence = event
                .get("confidence")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            confidences.insert(state.to_string(), confidence);
        }
    }
    confidences
}

/// The run's state: the `to` of the last `transition` event (section 4, Source of truth).
pub fn current_state(events: &[Map<String, Value>]) -> Option<&str> {
    events
        .iter()
        .rev()
        .find(|e| is_transition(e))
        .and_then(|e| e.get("to"))
        .and_then(Value::as_str)
}

/// Section 4, Run status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Finished,
    Active,
    Waiting,
    Pending,
    Interrupted,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Finished => "finished",
            RunStatus::Active => "active",
            RunStatus::Waiting => "waiting",
            RunStatus::Pending => "pending",
            RunStatus::Interrupted => "interrupted",
        }
    }
}

/// A run's status, derived from its events in the section 4 order. `lock_alive` is whether
/// the run's `.lock` holds a live pid.
/// Only a root-level final state finishes a run: a nested one is passed through at once.
pub fn run_status(m: &LoadedMachine, events: &[Map<String, Value>], lock_alive: bool) -> RunStatus {
    let in_final = current_state(events)
        .and_then(|s| m.find(s))
        .is_some_and(|s| is_root_final(m, s));
    let last = events.last();
    let last_type = last.and_then(|e| e.get("type")).and_then(Value::as_str);
    let last_source = last.and_then(|e| e.get("source")).and_then(Value::as_str);
    if in_final {
        RunStatus::Finished
    } else if lock_alive {
        RunStatus::Active
    } else if last_type == Some("waiting") {
        RunStatus::Waiting
    } else if last_type == Some("received")
        || (last_type == Some("transition") && last_source == Some("retry"))
    {
        RunStatus::Pending
    } else {
        RunStatus::Interrupted
    }
}

/// Section 4, Lifecycle step 3: an invalid message starts its run in `failed`. Appends the
/// one `transition` event, and mirrors `state: failed` when `mirror` is set (it is not
/// when the frontmatter itself did not parse). Nothing runs.
pub fn reject(
    events: &mut EventLog,
    run_dir: &Path,
    file: &str,
    reason: &str,
    mirror: bool,
) -> Result<(), InterpreterError> {
    let fields = json!({
        "from": null,
        "event": "claimed",
        "to": FAILED,
        "source": "invalid_message",
        "exit_code": null,
        "file": file,
        "error": reason,
    });
    let Value::Object(fields) = fields else {
        unreachable!("event fields are a JSON object");
    };
    let path = run_dir.join(EVENTS_FILE);
    events.append("transition", fields).map_err(io_err(&path))?;
    if mirror {
        mirror_state(&run_dir.join(MESSAGE_FILE), FAILED)?;
    }
    Ok(())
}

/// Set frontmatter `state` in the message at `path`, keeping every other key, the key
/// order and the body bytes (section 4, Parsing and writing).
pub fn mirror_state(path: &Path, state: &str) -> Result<(), InterpreterError> {
    let bytes = fs::read(path).map_err(io_err(path))?;
    let (mut mapping, body) = split_message(path, &bytes)?;
    mapping.insert("state".into(), state.into());
    let yaml = serde_norway::to_string(&mapping).map_err(|e| InterpreterError::Message {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    let mut out = format!("---\n{yaml}---\n").into_bytes();
    out.extend_from_slice(body.as_bytes());
    write_replace(path, &out)
}

/// A message's frontmatter and its body, exactly as written (section 4, Parsing and
/// writing). `path` names the message in errors.
fn split_message(
    path: &Path,
    bytes: &[u8],
) -> Result<(serde_norway::Mapping, String), InterpreterError> {
    let message_err = |message: String| InterpreterError::Message {
        path: path.to_path_buf(),
        message,
    };
    let text = std::str::from_utf8(bytes).map_err(|e| message_err(e.to_string()))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    let mut lines = text.split_inclusive('\n');
    match lines.next() {
        Some(first) if first.trim_end() == "---" => {
            let mut yaml = String::new();
            let mut offset = first.len();
            let mut closed = false;
            for line in lines {
                offset += line.len();
                if line.trim_end() == "---" {
                    closed = true;
                    break;
                }
                yaml.push_str(line);
            }
            if !closed {
                return Err(message_err(
                    "line 1: frontmatter has no closing `---`".to_string(),
                ));
            }
            let mapping = if yaml.trim().is_empty() {
                serde_norway::Mapping::new()
            } else {
                serde_norway::from_str(&yaml)
                    .map_err(|e| message_err(format!("frontmatter: {e}")))?
            };
            Ok((mapping, text[offset..].to_string()))
        }
        _ => Ok((serde_norway::Mapping::new(), text.to_string())),
    }
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
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    fs::write(&tmp, bytes).map_err(io_err(&tmp))?;
    fs::rename(&tmp, path).map_err(io_err(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::{load_machine_text, CheckEnv};
    use std::collections::{BTreeSet, HashSet};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use tempfile::TempDir;

    const RUN_ID: &str = "20261001T143005Z-3fa9c1";
    const BODY: &str = "# Task\r\nDo the thing.\n";

    fn repo() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    /// A temp project holding fixture machine `tests/fixtures/machines/step/<name>.yml` (or
    /// several machines, the first of which the run uses), a run folder with its
    /// `message.md`, and every script the machines name: `record.sh` installed under that
    /// name, unless `scripts` maps the name to another fixture.
    struct Project {
        tmp: TempDir,
        name: String,
        machines: BTreeMap<String, LoadedMachine>,
        default_router: Option<String>,
        shutdown: Arc<AtomicBool>,
    }

    /// Every script machine `m` names: invokes, `ask` scripts, `onentry` and `onexit`.
    fn script_names(m: &LoadedMachine) -> Vec<&str> {
        let mut names = Vec::new();
        for node in &m.nodes {
            match &node.invoke {
                Some(Invoke::Script(name)) => names.push(name.as_str()),
                Some(Invoke::Choose(c)) => names.extend(c.ask.as_deref()),
                _ => {}
            }
            names.extend(node.onentry.iter().chain(&node.onexit).map(String::as_str));
        }
        names
    }

    impl Project {
        fn new(name: &str, scripts: &[(&str, &str)]) -> Self {
            let fixture = repo().join(format!("tests/fixtures/machines/step/{name}.yml"));
            let text = fs::read_to_string(&fixture).unwrap();
            Self::from_text(name, &text, scripts)
        }

        /// The same, for machine `name` written as `text`.
        fn from_text(name: &str, text: &str, scripts: &[(&str, &str)]) -> Self {
            Self::from_texts(&[(name, text)], scripts, None)
        }

        /// The same, for several machines: the run uses the first. `default_router` is the
        /// config value.
        fn from_texts(
            machines: &[(&str, &str)],
            scripts: &[(&str, &str)],
            default_router: Option<&str>,
        ) -> Self {
            let tmp = TempDir::new().unwrap();
            let decree = tmp.path().join(DECREE_DIR);
            let loaded: BTreeMap<String, LoadedMachine> = machines
                .iter()
                .map(|(name, text)| {
                    let m = load_machine_text(name, Path::new("m.yml"), text).unwrap();
                    (name.to_string(), m)
                })
                .collect();

            let script_dir = decree.join("scripts");
            fs::create_dir_all(&script_dir).unwrap();
            let fixtures = repo().join("tests/fixtures/scripts");
            for script in loaded.values().flat_map(script_names) {
                let file = scripts
                    .iter()
                    .find(|(s, _)| *s == script)
                    .map_or("record", |(_, f)| f);
                // fs::copy keeps the fixture's executable bit.
                fs::copy(
                    fixtures.join(format!("{file}.sh")),
                    script_dir.join(format!("{script}.sh")),
                )
                .unwrap();
            }
            // Every fixture machine passes `decree check`.
            let ids: BTreeSet<String> = loaded.keys().cloned().collect();
            let env = CheckEnv {
                decree_dir: &decree,
                shared_source: None,
                machine_ids: &ids,
                machines: &loaded,
                default_router,
            };
            for (name, text) in machines {
                let problems = loaded[*name].validate(text, &env);
                assert!(problems.is_empty(), "{name}: {problems:?}");
            }

            let name = machines[0].0;
            let project = Project {
                tmp,
                name: name.to_string(),
                machines: loaded,
                default_router: default_router.map(String::from),
                shutdown: Arc::new(AtomicBool::new(false)),
            };
            fs::create_dir_all(project.run_dir()).unwrap();
            let message =
                format!("---\nid: {RUN_ID}\nmachine: {name}\ntrigger: inbox\n---\n{BODY}");
            fs::write(project.run_dir().join(MESSAGE_FILE), message).unwrap();
            project
        }

        /// The machine the run uses.
        fn machine(&self) -> &LoadedMachine {
            &self.machines[&self.name]
        }

        fn ctx(&self) -> Context<'_> {
            Context {
                project_root: self.root(),
                shared_source: None,
                machines: &self.machines,
                default_router: self.default_router.clone(),
                max_attempts: 3,
                max_depth: 10,
                max_log_size: 0,
                shutdown: Arc::clone(&self.shutdown),
            }
        }

        fn root(&self) -> PathBuf {
            self.tmp.path().to_path_buf()
        }

        fn run_dir(&self) -> PathBuf {
            self.root().join(".decree/runs").join(RUN_ID)
        }

        fn executor(&self, trigger: &str, params: &serde_norway::Mapping) -> Executor {
            self.ctx()
                .executor(self.machine(), RUN_ID, trigger, params, None)
                .unwrap()
        }

        fn run(&self) -> Outcome {
            self.run_with("inbox", "inbox.md", &serde_norway::Mapping::new())
        }

        fn run_with(&self, trigger: &str, file: &str, params: &serde_norway::Mapping) -> Outcome {
            self.start(trigger, file, params).unwrap()
        }

        fn start(
            &self,
            trigger: &str,
            file: &str,
            params: &serde_norway::Mapping,
        ) -> Result<Outcome, InterpreterError> {
            let input = RunInput {
                params: params.clone(),
                message_body: BODY.to_string(),
                file: Some(file.to_string()),
                depth: 0,
            };
            let ctx = self.ctx();
            Interpreter::new(&ctx, self.machine(), self.executor(trigger, params), input)?.start()
        }

        /// The log of the first `script` event of `script`.
        fn log_of(&self, script: &str) -> String {
            let event = self
                .events_of("script")
                .into_iter()
                .find(|e| e["script"] == script)
                .unwrap_or_else(|| panic!("no script event for {script}"));
            fs::read_to_string(self.run_dir().join(event["log"].as_str().unwrap())).unwrap()
        }

        /// The script names in the order they ran.
        fn order(&self) -> Vec<String> {
            fs::read_to_string(self.root().join("order.txt"))
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect()
        }

        fn events(&self) -> Vec<Map<String, Value>> {
            read_events(&self.run_dir()).unwrap()
        }

        fn events_of(&self, kind: &str) -> Vec<Map<String, Value>> {
            self.events()
                .into_iter()
                .filter(|e| e["type"] == kind)
                .collect()
        }

        /// `from event to source` of every `transition` event, `-` for a null `from`.
        fn transitions(&self) -> Vec<String> {
            self.events_of("transition")
                .iter()
                .map(|e| {
                    format!(
                        "{} {} {} {}",
                        e["from"].as_str().unwrap_or("-"),
                        e["event"].as_str().unwrap(),
                        e["to"].as_str().unwrap(),
                        e["source"].as_str().unwrap()
                    )
                })
                .collect()
        }

        fn message(&self) -> String {
            fs::read_to_string(self.run_dir().join(MESSAGE_FILE)).unwrap()
        }

        fn processed(&self) -> String {
            fs::read_to_string(self.root().join(".decree/processed.md")).unwrap_or_default()
        }
    }

    /// The input of an inbox message `inbox.md` with no params.
    fn inbox_input() -> RunInput {
        RunInput {
            message_body: BODY.to_string(),
            file: Some("inbox.md".to_string()),
            ..RunInput::default()
        }
    }

    fn params(yaml: &str) -> serde_norway::Mapping {
        serde_norway::from_str(yaml).unwrap()
    }

    fn mirrored_state(p: &Project) -> String {
        let message = p.message();
        let line = message
            .lines()
            .find(|l| l.starts_with("state: "))
            .unwrap_or_default();
        line.trim_start_matches("state: ").to_string()
    }

    // ---------------------------------------------------------------
    // Exit and entry order (section 7, Step loop)
    // ---------------------------------------------------------------

    #[test]
    fn order_normal_path_to_a_final_state() {
        let p = Project::new("step_normal", &[]);
        assert_eq!(p.run(), Outcome::Finished("done".into()));
        assert_eq!(
            p.order(),
            [
                "root_entry",
                "a_entry",
                "a_invoke",
                "a_exit",
                "b_entry",
                "b_exit",
                "done_entry",
                "root_exit"
            ]
        );
        assert_eq!(
            p.transitions(),
            [
                "- claimed a claim",
                "a done b exit_code",
                "b done done exit_code"
            ]
        );
        let transitions = p.events_of("transition");
        assert_eq!(transitions[0]["file"], "inbox.md");
        assert_eq!(transitions[1]["exit_code"], 0);
        // A pass-through has no invoke, so no exit code.
        assert_eq!(transitions[2]["exit_code"], Value::Null);
        // Reaching a final state: root onexit has run, then `run_finished` is last.
        let events = p.events();
        let last = events.last().unwrap();
        assert_eq!(last["type"], "run_finished");
        assert_eq!(last["state"], "done");
        assert!(last["duration_ms"].as_u64().is_some());
        let before = &events[events.len() - 2];
        assert_eq!(
            (&before["type"], &before["script"]),
            (&json!("script"), &json!("root_exit"))
        );
        assert_eq!(mirrored_state(&p), "done");
        assert_eq!(run_status(p.machine(), &events, false), RunStatus::Finished);
    }

    #[test]
    fn order_self_transition_exits_and_reenters_the_state() {
        let p = Project::new("step_self", &[]);
        assert_eq!(p.run(), Outcome::Finished("done".into()));
        assert_eq!(
            p.order(),
            [
                "root_entry",
                "a_entry",
                "a_exit",
                "a_entry",
                "a_exit",
                "done_entry",
                "root_exit"
            ]
        );
        assert_eq!(
            p.transitions(),
            ["- claimed a claim", "a yes a check", "a no done check"]
        );
        assert_eq!(visits(&p.events())["a"], 2);
    }

    #[test]
    fn order_entering_and_leaving_a_compound_state() {
        let p = Project::new("step_compound", &[]);
        assert_eq!(p.run(), Outcome::Finished("done".into()));
        assert_eq!(
            p.order(),
            [
                "root_entry",
                "outer_entry",
                "inner_entry",
                "inner_invoke",
                "inner_exit",
                "outer_exit",
                "after_entry",
                "after_invoke",
                "after_exit",
                "done_entry",
                "root_exit"
            ]
        );
        // The claim follows `initial` down to the atomic state.
        assert_eq!(
            p.transitions(),
            [
                "- claimed inner claim",
                "inner done after exit_code",
                "after done done exit_code"
            ]
        );
        // Only atomic states have visits.
        let v = visits(&p.events());
        assert_eq!(v.get("inner"), Some(&1));
        assert_eq!(v.get("outer"), None);
    }

    #[test]
    fn order_unhandled_error_goes_to_failed() {
        let p = Project::new("step_error", &[]);
        assert_eq!(p.run(), Outcome::Finished("failed".into()));
        assert_eq!(
            p.order(),
            [
                "root_entry",
                "a_entry",
                "a_invoke_fail",
                "a_exit",
                "failed_entry",
                "root_exit"
            ]
        );
        assert_eq!(
            p.transitions(),
            ["- claimed a claim", "a error failed exit_code"]
        );
        assert_eq!(p.events_of("transition")[1]["exit_code"], 1);
        assert_eq!(p.events_of("run_finished")[0]["state"], "failed");
        assert_eq!(mirrored_state(&p), "failed");
    }

    #[test]
    fn order_onentry_failure_skips_the_rest_and_the_invoke() {
        let p = Project::new("step_entry_fail", &[]);
        assert_eq!(p.run(), Outcome::Finished("failed".into()));
        assert_eq!(
            p.order(),
            [
                "root_entry",
                "a_entry_fail",
                "a_exit",
                "failed_entry",
                "root_exit"
            ]
        );
        assert_eq!(
            p.transitions(),
            ["- claimed a claim", "a error failed exit_code"]
        );
        // No invoke ran, so no exit code.
        assert_eq!(p.events_of("transition")[1]["exit_code"], Value::Null);
    }

    #[test]
    fn order_root_onentry_failure_targets_failed() {
        let p = Project::new("step_root_entry_fail", &[]);
        assert_eq!(p.run(), Outcome::Finished("failed".into()));
        // `a` handles `error`, but a root `onentry` failure always targets `failed`.
        assert_eq!(
            p.order(),
            ["root_entry_fail", "a_exit", "failed_entry", "root_exit"]
        );
        assert_eq!(
            p.transitions(),
            ["- claimed a claim", "a error failed exit_code"]
        );
    }

    #[test]
    fn order_onexit_failure_is_recorded_and_changes_nothing() {
        let p = Project::new("step_exit_fail", &[]);
        assert_eq!(p.run(), Outcome::Finished("done".into()));
        assert_eq!(
            p.order(),
            [
                "root_entry",
                "a_entry",
                "a_invoke",
                "a_exit_fail",
                "a_exit",
                "done_entry",
                "root_exit"
            ]
        );
        assert_eq!(
            p.transitions(),
            ["- claimed a claim", "a done done exit_code"]
        );
        assert_eq!(
            p.events_of("transition")[1]["exit_failures"],
            json!(["a_exit_fail"])
        );
    }

    #[test]
    fn order_final_state_onentry_failure_moves_to_failed() {
        let p = Project::new("step_final_fail", &[]);
        assert_eq!(p.run(), Outcome::Finished("failed".into()));
        assert_eq!(
            p.order(),
            [
                "root_entry",
                "a_invoke",
                "done_entry_fail",
                "failed_entry",
                "root_exit"
            ]
        );
        assert_eq!(
            p.transitions(),
            [
                "- claimed a claim",
                "a done done exit_code",
                "done error failed exit_code"
            ]
        );
        assert_eq!(p.events_of("run_finished")[0]["state"], "failed");
        assert_eq!(mirrored_state(&p), "failed");
    }

    #[test]
    fn failing_onentry_on_failed_itself_is_only_logged() {
        let p = Project::new("step_failed_entry_fail", &[]);
        assert_eq!(p.run(), Outcome::Finished("failed".into()));
        assert_eq!(
            p.order(),
            // Section 6: the remaining `onentry` scripts are skipped; the run still ends.
            ["a_invoke_fail", "failed_entry_fail", "root_exit"]
        );
        assert_eq!(
            p.transitions(),
            ["- claimed a claim", "a error failed exit_code"]
        );
        let scripts = p.events_of("script");
        let failed_entry = scripts
            .iter()
            .find(|e| e["script"] == "failed_entry_fail")
            .unwrap();
        assert_eq!(failed_entry["exit_code"], 1);
    }

    // ---------------------------------------------------------------
    // Migrations: the ledger line (section 4, rule 5; section 7, step 7)
    // ---------------------------------------------------------------

    #[test]
    fn migration_ledger_line_is_written_before_final_onentry() {
        let p = Project::new("step_normal", &[("done_entry", "copy_ledger")]);
        fs::write(p.root().join(".decree/processed.md"), "44-prev.md").unwrap();
        let outcome = p.run_with("migration", "45-next.md", &serde_norway::Mapping::new());
        assert_eq!(outcome, Outcome::Finished("done".into()));
        let seen = fs::read_to_string(p.root().join("ledger.txt")).unwrap();
        assert_eq!(seen, "44-prev.md\n45-next.md\n");
        assert_eq!(p.processed(), "44-prev.md\n45-next.md\n");
        assert!(!p.root().join(".decree/.processed.md.tmp").exists());
    }

    #[test]
    fn migration_ledger_line_is_removed_when_final_onentry_fails() {
        let p = Project::new("step_final_fail", &[]);
        fs::write(p.root().join(".decree/processed.md"), "44-prev.md\n").unwrap();
        let outcome = p.run_with("migration", "45-next.md", &serde_norway::Mapping::new());
        assert_eq!(outcome, Outcome::Finished("failed".into()));
        assert_eq!(p.processed(), "44-prev.md\n");
    }

    #[test]
    fn failed_migration_writes_no_ledger_line() {
        let p = Project::new("step_error", &[]);
        p.run_with("migration", "45-next.md", &serde_norway::Mapping::new());
        assert_eq!(p.processed(), "");
    }

    // ---------------------------------------------------------------
    // Attempts and visits
    // ---------------------------------------------------------------

    #[test]
    fn invoke_failing_twice_then_succeeding_takes_done_after_two_attempts() {
        let p = Project::new("step_attempts", &[("fail_until_final", "fail_until_final")]);
        assert_eq!(p.run(), Outcome::Finished("done".into()));
        assert_eq!(
            p.transitions(),
            [
                "- claimed work claim",
                "work error work attempt",
                "work error work attempt",
                "work done done exit_code"
            ]
        );
        let scripts = p.events_of("script");
        let attempts: Vec<_> = scripts
            .iter()
            .map(|e| {
                (
                    e["attempt"].as_u64().unwrap(),
                    e["exit_code"].as_i64().unwrap(),
                )
            })
            .collect();
        assert_eq!(attempts, [(1, 1), (2, 1), (3, 0)]);
        // `fail_until_final` exits 0 only when DECREE_FINAL_ATTEMPT=true.
        let third = scripts[2]["log"].as_str().unwrap();
        let log = fs::read_to_string(p.run_dir().join(third)).unwrap();
        assert_eq!(log, "attempt 3 of 3\n");
        // Attempts are not visits.
        assert_eq!(visits(&p.events())["work"], 1);
    }

    #[test]
    fn visits_check_ends_a_retry_loop_after_two_visits() {
        let p = Project::new(
            "step_retry_loop",
            &[
                ("fail_until_final", "fail_until_final"),
                ("verify", "exit_zero"),
            ],
        );
        assert_eq!(p.run(), Outcome::Finished("done".into()));
        let events = p.events();
        assert_eq!(visits(&events)["implement"], 2);
        let attempts = p
            .events_of("transition")
            .iter()
            .filter(|e| e["source"] == "attempt" && e["to"] == "implement")
            .count();
        assert_eq!(attempts, 2);
        assert_eq!(
            p.transitions(),
            [
                "- claimed implement claim",
                "implement error implement attempt",
                "implement done verify exit_code",
                "verify done rounds_left exit_code",
                "rounds_left yes implement check",
                "implement error implement attempt",
                "implement done verify exit_code",
                "verify done rounds_left exit_code",
                "rounds_left no done check"
            ]
        );
        let decisions = p.events_of("decision");
        assert_eq!(decisions.len(), 2);
        assert_eq!(decisions[1]["event"], "no");
    }

    #[test]
    fn visits_count_claim_and_retry_but_not_attempts() {
        let events: Vec<Map<String, Value>> = [
            json!({"type": "transition", "from": null, "to": "a", "source": "claim"}),
            json!({"type": "transition", "from": "a", "to": "a", "source": "attempt"}),
            json!({"type": "script", "state": "a"}),
            json!({"type": "transition", "from": "a", "to": "b", "source": "exit_code"}),
            json!({"type": "transition", "from": "b", "to": "b", "source": "retry"}),
        ]
        .into_iter()
        .map(|v| v.as_object().unwrap().clone())
        .collect();
        let v = visits(&events);
        assert_eq!(v["a"], 1);
        assert_eq!(v["b"], 2);
        assert_eq!(current_state(&events), Some("b"));
    }

    // ---------------------------------------------------------------
    // Check (section 7, Check)
    // ---------------------------------------------------------------

    /// Machine `step_check`: `work` runs `script` (a fixture name), then `decide` checks
    /// `condition` (YAML flow mapping), with `input: work` if `input` is set.
    fn check_project(condition: &str, input: bool, script: &str) -> Project {
        let input = if input { ", input: work" } else { "" };
        let text = format!(
            "name: step_check\ndescription: Run a script, then check a condition.\n\
             data:\n  max_rounds: {{ type: int, default: 2 }}\n  limit: {{ type: int, default: 1 }}\n  \
             mode: {{ type: string, default: fast }}\n  strict: {{ type: bool, default: true }}\n\
             initial: work\nstates:\n  \
             work: {{ invoke: work, transitions: {{ done: decide }} }}\n  \
             decide:\n    invoke: {{ check: {condition}{input} }}\n    transitions: {{ yes: passed, no: refused }}\n  \
             passed: {{ final: true }}\n  refused: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        Project::from_text("step_check", &text, &[("work", script)])
    }

    /// The event a check produces, from its `decision` event, after a clean run.
    fn check_event(condition: &str, params_yaml: &str) -> String {
        let p = check_project(condition, false, "exit_zero");
        let outcome = p.run_with("inbox", "inbox.md", &params(params_yaml));
        let decision = &p.events_of("decision")[0];
        let event = decision["event"].as_str().unwrap().to_string();
        let expected = if event == "yes" { "passed" } else { "refused" };
        assert_eq!(outcome, Outcome::Finished(expected.into()), "{condition}");
        event
    }

    #[test]
    fn check_each_operator_on_visits() {
        // `work` has been entered once when `decide` runs.
        for (op, yes, no) in [
            ("equals", 1, 2),
            ("not_equals", 2, 1),
            ("less_than", 2, 1),
            ("at_most", 1, 0),
            ("more_than", 0, 1),
            ("at_least", 1, 2),
        ] {
            let cond = |n: i32| format!("{{ visits: work, {op}: {n} }}");
            assert_eq!(check_event(&cond(yes), "{}"), "yes", "{op} {yes}");
            assert_eq!(check_event(&cond(no), "{}"), "no", "{op} {no}");
        }
    }

    #[test]
    fn check_each_operator_on_data() {
        for (cond, yes) in [
            ("{ data: mode, equals: fast }", true),
            ("{ data: mode, not_equals: fast }", false),
            ("{ data: strict, equals: true }", true),
            ("{ data: strict, not_equals: true }", false),
            ("{ data: max_rounds, less_than: 3 }", true),
            ("{ data: max_rounds, at_most: 1 }", false),
            ("{ data: max_rounds, more_than: 1 }", true),
            ("{ data: max_rounds, at_least: 3 }", false),
        ] {
            let want = if yes { "yes" } else { "no" };
            assert_eq!(check_event(cond, "{}"), want, "{cond}");
        }
    }

    #[test]
    fn check_compares_with_a_data_value_set_by_params() {
        let cond = "{ visits: work, less_than: { data: max_rounds } }";
        assert_eq!(check_event(cond, "{}"), "yes");
        assert_eq!(check_event(cond, "max_rounds: 1"), "no");
        let cond = "{ data: limit, equals: { data: max_rounds } }";
        assert_eq!(check_event(cond, "{}"), "no");
        assert_eq!(check_event(cond, "limit: 2"), "yes");
    }

    #[test]
    fn check_matches_reads_the_input_states_output() {
        // `exit_zero` prints `hello`; `stderr` writes `[stderr] to stderr` to its log.
        for (script, cond, want) in [
            ("exit_zero", "{ matches: '(?m)^hello$' }", "yes"),
            ("exit_zero", "{ matches: hel+o }", "yes"),
            ("exit_zero", "{ matches: '(?m)^bye$' }", "no"),
            (
                "stderr",
                "{ matches: '(?m)^\\[stderr\\] to stderr$' }",
                "yes",
            ),
        ] {
            let p = check_project(cond, true, script);
            p.run();
            let decision = &p.events_of("decision")[0];
            assert_eq!(decision["event"], want, "{script} {cond}");
        }
    }

    #[test]
    fn check_matches_without_input_reads_the_most_recent_invoke() {
        let p = check_project("{ matches: '^hello' }", false, "exit_zero");
        assert_eq!(p.run(), Outcome::Finished("passed".into()));
    }

    #[test]
    fn check_appends_a_decision_before_its_transition() {
        let p = check_project(
            "{ visits: work, less_than: { data: max_rounds } }",
            false,
            "exit_zero",
        );
        assert_eq!(p.run(), Outcome::Finished("passed".into()));
        let events = p.events();
        let i = events.iter().position(|e| e["type"] == "decision").unwrap();
        let d = &events[i];
        assert_eq!(d["state"], "decide");
        assert_eq!(d["kind"], "check");
        assert_eq!(d["event"], "yes");
        assert_eq!(
            d["condition"],
            json!({ "visits": "work", "less_than": { "data": "max_rounds" } })
        );
        let t = &events[i + 1];
        assert_eq!(t["type"], "transition");
        assert_eq!(
            (
                &t["from"],
                &t["event"],
                &t["to"],
                &t["source"],
                &t["exit_code"]
            ),
            (
                &json!("decide"),
                &json!("yes"),
                &json!("passed"),
                &json!("check"),
                &Value::Null
            )
        );
        // No script ran for the check.
        assert!(p.events_of("script").iter().all(|e| e["state"] != "decide"));
    }

    // ---------------------------------------------------------------
    // Other events
    // ---------------------------------------------------------------

    #[test]
    fn undeclared_printed_event_becomes_error_with_invalid_event() {
        let p = Project::new("step_normal", &[("a_invoke", "print_undeclared")]);
        assert_eq!(p.run(), Outcome::Finished("failed".into()));
        let t = &p.events_of("transition")[1];
        assert_eq!(t["event"], "error");
        assert_eq!(t["source"], "stdout");
        assert_eq!(t["invalid_event"], "nope");
        assert_eq!(t["to"], "failed");
    }

    #[test]
    fn timed_out_invoke_gives_error() {
        let p = Project::new("step_timeout", &[("sleep_long", "sleep_long")]);
        assert_eq!(p.run(), Outcome::Finished("failed".into()));
        let script = &p.events_of("script")[0];
        assert_eq!(script["timed_out"], true);
        assert_eq!(script["exit_code"], Value::Null);
        assert_eq!(
            p.transitions(),
            ["- claimed work claim", "work error failed exit_code"]
        );
    }

    #[test]
    fn signal_interrupts_the_run_in_its_current_state() {
        let p = Project::new("step_normal", &[]);
        p.shutdown.store(true, Ordering::SeqCst);
        assert_eq!(p.run(), Outcome::Interrupted("a".into()));
        let events = p.events();
        let last = events.last().unwrap();
        assert_eq!(last["type"], "interrupted");
        assert_eq!(last["state"], "a");
        assert_eq!(last["cause"], "signal");
        assert_eq!(last["script"], "root_entry");
        assert!(p.order().is_empty());
        assert_eq!(
            run_status(p.machine(), &events, false),
            RunStatus::Interrupted
        );
    }

    // ---------------------------------------------------------------
    // Composition: bubbling, `type: internal`, nested final states (M3.4)
    // ---------------------------------------------------------------

    #[test]
    fn composition_unhandled_event_is_taken_by_the_nearest_ancestor() {
        let p = Project::new("step_bubble", &[("a_invoke", "print_pass")]);
        assert_eq!(p.run(), Outcome::Finished("done".into()));
        // `a` does not handle `pass`; `work` does, before `outer`. The domain is the root,
        // so `a`, `work` and `outer` are all exited, innermost first.
        assert_eq!(
            p.transitions(),
            [
                "- claimed a claim",
                "a pass after stdout",
                "after done done exit_code"
            ]
        );
        assert_eq!(
            p.order(),
            [
                "root_entry",
                "outer_entry",
                "work_entry",
                "a_entry",
                "a_exit",
                "work_exit",
                "outer_exit",
                "after_entry",
                "done_entry",
                "root_exit"
            ]
        );
    }

    #[test]
    fn internal_transition_on_a_compound_state_runs_none_of_its_scripts() {
        let p = Project::new("step_internal", &[("a_invoke", "print_pass")]);
        assert_eq!(p.run(), Outcome::Finished("done".into()));
        assert_eq!(
            p.transitions(),
            [
                "- claimed a claim",
                "a pass b stdout",
                "b done done exit_code"
            ]
        );
        // `p` is entered once at the claim and exited once on the way to `done`.
        assert_eq!(
            p.order(),
            [
                "root_entry",
                "p_entry",
                "a_entry",
                "a_exit",
                "b_entry",
                "b_invoke",
                "b_exit",
                "p_exit",
                "done_entry",
                "root_exit"
            ]
        );
    }

    #[test]
    fn external_transition_on_a_compound_state_exits_and_reenters_it() {
        let p = Project::new("step_external", &[("a_invoke", "print_pass")]);
        assert_eq!(p.run(), Outcome::Finished("done".into()));
        assert_eq!(
            p.order(),
            [
                "root_entry",
                "p_entry",
                "a_entry",
                "a_exit",
                "p_exit",
                "p_entry",
                "b_entry",
                "b_invoke",
                "b_exit",
                "p_exit",
                "done_entry",
                "root_exit"
            ]
        );
    }

    #[test]
    fn nested_final_state_raises_done_state_at_once_and_the_run_goes_on() {
        let p = Project::new("step_nested_final", &[]);
        assert_eq!(p.run(), Outcome::Finished("done".into()));
        assert_eq!(
            p.transitions(),
            [
                "- claimed a claim",
                "a done finished exit_code",
                "finished done.state.work after internal",
                "after done done exit_code"
            ]
        );
        assert_eq!(
            p.order(),
            [
                "root_entry",
                "work_entry",
                "a_entry",
                "a_invoke",
                "a_exit",
                "finished_entry",
                "work_exit",
                "after_entry",
                "after_invoke",
                "done_entry",
                "root_exit"
            ]
        );
        // Handled at once: after the nested final state's own `onentry` script, only the
        // `onexit` scripts of the states it leaves run before the `done.state.work`
        // transition is recorded (steps 5 and 6).
        let events = p.events();
        let at = events
            .iter()
            .position(|e| e["type"] == "transition" && e["to"] == "finished")
            .unwrap();
        assert_eq!(events[at + 1]["script"], "finished_entry");
        assert_eq!(events[at + 2]["script"], "work_exit");
        let raised = &events[at + 3];
        assert_eq!(raised["type"], "transition");
        assert_eq!(raised["event"], "done.state.work");
        assert_eq!(raised["source"], "internal");
        assert_eq!(raised["exit_code"], Value::Null);
        // Only a root-level final state ends the run.
        let run_finished: Vec<_> = p.events_of("run_finished");
        assert_eq!(run_finished.len(), 1);
        assert_eq!(run_finished[0]["state"], "done");
        assert_eq!(
            run_status(p.machine(), &events[..=at + 1], false),
            RunStatus::Interrupted
        );
        assert_eq!(visits(&events)["finished"], 1);
    }

    #[test]
    fn nested_final_state_writes_no_ledger_line() {
        let p = Project::new("step_nested_final", &[("after_invoke", "copy_ledger")]);
        fs::write(p.root().join(".decree/processed.md"), "45-prev.md\n").unwrap();
        let outcome = p.run_with("migration", "46-next.md", &serde_norway::Mapping::new());
        assert_eq!(outcome, Outcome::Finished("done".into()));
        // `copy_ledger` ran after `finished` was entered, before the root `done`.
        let seen = fs::read_to_string(p.root().join("ledger.txt")).unwrap();
        assert_eq!(seen, "45-prev.md\n");
        assert_eq!(p.processed(), "45-prev.md\n46-next.md\n");
    }

    #[test]
    fn nested_final_onentry_failure_is_error_resolved_from_that_state() {
        let p = Project::new("step_nested_final", &[("finished_entry", "exit_three")]);
        assert_eq!(p.run(), Outcome::Finished("recovered".into()));
        // No `done.state.work`: the error leaves `work` through its `error` transition.
        assert_eq!(
            p.transitions(),
            [
                "- claimed a claim",
                "a done finished exit_code",
                "finished error recovered exit_code"
            ]
        );
        assert_eq!(
            p.order(),
            [
                "root_entry",
                "work_entry",
                "a_entry",
                "a_invoke",
                "a_exit",
                "work_exit",
                "recovered_entry",
                "root_exit"
            ]
        );
    }

    // ---------------------------------------------------------------
    // Choose: person, interpreter side (section 4, Replies; section 7)
    // ---------------------------------------------------------------

    /// The `step_person` project, run until it waits. `ask_person` prints its environment.
    fn person_project(scripts: &[(&str, &str)]) -> (Project, String) {
        let mut all = vec![("ask_person", "print_env")];
        all.extend_from_slice(scripts);
        let p = Project::new("step_person", &all);
        let outcome = p.run();
        let entered = p
            .events_of("transition")
            .into_iter()
            .find(|e| e["to"] == "approval")
            .unwrap();
        let wait_id = format!("{RUN_ID}.w{}", entered["seq"]);
        assert_eq!(
            outcome,
            Outcome::Waiting {
                state: "approval".into(),
                wait_id: wait_id.clone()
            }
        );
        (p, wait_id)
    }

    /// Append a `received` event, as reply delivery (M4.3) does, then continue the run.
    fn receive(p: &Project, fields: Value) -> Outcome {
        let mut executor = p.executor("inbox", &serde_norway::Mapping::new());
        executor
            .events()
            .append("received", fields.as_object().unwrap().clone())
            .unwrap();
        let ctx = p.ctx();
        Interpreter::new(&ctx, p.machine(), executor, inbox_input())
            .unwrap()
            .resume()
            .unwrap()
    }

    #[test]
    fn person_ask_script_sees_the_wait_id_and_choices_then_the_run_waits() {
        let before = Utc::now();
        let (p, wait_id) = person_project(&[]);
        assert_eq!(
            p.order(),
            ["root_entry", "build_invoke", "gate_entry", "approval_entry"]
        );
        let log = p.log_of("ask_person");
        let choices = p.run_dir().join(CHOICES_FILE);
        for line in [
            format!("DECREE_WAIT_ID={wait_id}"),
            format!("DECREE_CHOICES={}", choices.display()),
            "DECREE_QUESTION=Ship this build?".to_string(),
            "DECREE_EVENTS=approve reject".to_string(),
            "DECREE_STATE=approval".to_string(),
            "DECREE_PHASE=invoke".to_string(),
        ] {
            assert!(log.lines().any(|l| l == line), "{line}\n{log}");
        }
        let written: Value = serde_json::from_str(&fs::read_to_string(&choices).unwrap()).unwrap();
        assert_eq!(
            written,
            json!({ "approve": "Ship this build.", "reject": "Do not ship." })
        );
        let script = p
            .events_of("script")
            .into_iter()
            .find(|e| e["script"] == "ask_person")
            .unwrap();
        assert_eq!(script["phase"], "invoke");

        let events = p.events();
        let last = events.last().unwrap();
        assert_eq!(last["type"], "waiting");
        assert_eq!(last["state"], "approval");
        assert_eq!(last["wait_id"], json!(wait_id));
        assert_eq!(last["options"], json!(["approve", "reject"]));
        let timeout_at = DateTime::parse_from_rfc3339(last["timeout_at"].as_str().unwrap())
            .unwrap()
            .with_timezone(&Utc);
        let ahead = (timeout_at - before).num_seconds();
        assert!((59..=61).contains(&ahead), "{ahead}");
        assert!(p.events_of("run_finished").is_empty());
        assert_eq!(run_status(p.machine(), &events, false), RunStatus::Waiting);
        assert_eq!(mirrored_state(&p), "approval");
    }

    #[test]
    fn person_without_timeout_has_null_timeout_at() {
        let text = fs::read_to_string(repo().join("tests/fixtures/machines/step/step_person.yml"))
            .unwrap()
            .replace(", timeout_s: 60", "");
        let p = Project::from_text("step_person", &text, &[]);
        assert!(matches!(p.run(), Outcome::Waiting { .. }));
        assert_eq!(p.events_of("waiting")[0]["timeout_at"], Value::Null);
    }

    #[test]
    fn person_reply_continues_with_source_person_and_reruns_nothing() {
        let (p, wait_id) = person_project(&[("ship_invoke", "print_env")]);
        let outcome = receive(
            &p,
            json!({ "wait_id": wait_id, "event": "approve", "file": "reply.md" }),
        );
        assert_eq!(outcome, Outcome::Finished("done".into()));
        // Nothing before the wait ran again: no root, `gate` or `approval` entry scripts.
        assert_eq!(
            p.order(),
            [
                "root_entry",
                "build_invoke",
                "gate_entry",
                "approval_entry",
                "approval_exit",
                "gate_exit",
                "ship_entry",
                "done_entry",
                "root_exit"
            ]
        );
        let scripts = p.events_of("script");
        assert_eq!(
            scripts
                .iter()
                .filter(|e| e["script"] == "ask_person")
                .count(),
            1
        );
        // A `decision` event, then the transition it causes.
        let events = p.events();
        let i = events.iter().position(|e| e["type"] == "decision").unwrap();
        assert_eq!(events[i - 1]["type"], "received");
        let d = &events[i];
        assert_eq!(d["state"], "approval");
        assert_eq!(d["kind"], "person");
        assert_eq!(d["event"], "approve");
        assert_eq!(d["options"], json!(["approve", "reject"]));
        assert_eq!(d["reply"], "reply.md");
        // Then the `onexit` scripts, then the transition (section 7, steps 5 and 6).
        let taken = events[i..]
            .iter()
            .find(|e| e["type"] == "transition")
            .unwrap();
        assert_eq!(taken["from"], "approval");
        assert_eq!(taken["event"], "approve");
        assert_eq!(taken["to"], "ship");
        assert_eq!(taken["source"], "person");
        assert_eq!(taken["exit_code"], Value::Null);
        // Later scripts see the reply, and no wait.
        let log = p.log_of("ship_invoke");
        let reply = p.run_dir().join("received/reply.md");
        assert!(
            log.contains(&format!("DECREE_RECEIVED={}\n", reply.display())),
            "{log}"
        );
        assert!(log.contains("DECREE_WAIT_ID=\n"), "{log}");
        assert!(log.contains("DECREE_CHOICES=\n"), "{log}");
        assert_eq!(run_status(p.machine(), &events, false), RunStatus::Finished);
        // Log numbers continue after the logs written before the wait.
        let logs: Vec<&str> = scripts.iter().map(|e| e["log"].as_str().unwrap()).collect();
        let unique: HashSet<&&str> = logs.iter().collect();
        assert_eq!(unique.len(), logs.len(), "{logs:?}");
    }

    #[test]
    fn person_timeout_error_goes_to_failed_with_source_timeout() {
        let (p, wait_id) = person_project(&[]);
        let outcome = receive(
            &p,
            json!({ "wait_id": wait_id, "event": "error", "timed_out": true }),
        );
        assert_eq!(outcome, Outcome::Finished("failed".into()));
        assert!(p
            .transitions()
            .contains(&"approval error failed timeout".to_string()));
        assert!(p.events_of("decision").is_empty());
        assert_eq!(
            &p.order()[4..],
            ["approval_exit", "gate_exit", "failed_entry", "root_exit"]
        );
    }

    #[test]
    fn person_ask_script_failure_is_error_and_does_not_wait() {
        let p = Project::new("step_person", &[("ask_person", "exit_three")]);
        assert_eq!(p.run(), Outcome::Finished("failed".into()));
        assert!(p.events_of("waiting").is_empty());
        let t = p.events_of("transition");
        let taken = t.iter().find(|e| e["from"] == "approval").unwrap();
        assert_eq!(taken["event"], "error");
        assert_eq!(taken["source"], "exit_code");
        assert_eq!(taken["exit_code"], 3);
    }

    #[test]
    fn person_onentry_failure_is_error_and_does_not_ask() {
        let p = Project::new("step_person", &[("approval_entry", "exit_three")]);
        assert_eq!(p.run(), Outcome::Finished("failed".into()));
        assert!(p.events_of("waiting").is_empty());
        assert!(p
            .events_of("script")
            .iter()
            .all(|e| e["script"] != "ask_person"));
    }

    #[test]
    fn resume_refuses_a_run_that_has_not_received_an_event() {
        let (p, _) = person_project(&[]);
        let params = serde_norway::Mapping::new();
        let ctx = p.ctx();
        let err = Interpreter::new(
            &ctx,
            p.machine(),
            p.executor("inbox", &params),
            inbox_input(),
        )
        .unwrap()
        .resume()
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "cannot continue the run: its last event is not `received`"
        );
        assert_eq!(p.events().last().unwrap()["type"], "waiting");
    }

    #[test]
    fn every_decision_waiting_and_received_field_appears() {
        let (p, wait_id) = person_project(&[]);
        receive(
            &p,
            json!({ "wait_id": wait_id, "event": "approve", "file": "reply.md" }),
        );
        let (q, wait_id) = person_project(&[]);
        receive(
            &q,
            json!({ "wait_id": wait_id, "event": "error", "timed_out": true }),
        );
        let c = check_project("{ visits: work, equals: 1 }", false, "exit_zero");
        c.run();
        let common = ["v", "seq", "ts", "type", "run_id", "machine", "trigger"];
        let keys = |kind: &str| -> HashSet<String> {
            p.events_of(kind)
                .into_iter()
                .chain(q.events_of(kind))
                .chain(c.events_of(kind))
                .flat_map(|e| e.keys().cloned().collect::<Vec<_>>())
                .collect()
        };
        for (kind, fields) in [
            (
                "waiting",
                &["state", "wait_id", "options", "timeout_at"][..],
            ),
            ("received", &["wait_id", "event", "file", "timed_out"][..]),
            (
                "decision",
                &["state", "kind", "event", "condition", "options", "reply"][..],
            ),
        ] {
            let want: HashSet<String> =
                common.iter().chain(fields).map(|s| s.to_string()).collect();
            assert_eq!(keys(kind), want, "{kind}");
        }
    }

    // ---------------------------------------------------------------
    // Sub-machines (section 7)
    // ---------------------------------------------------------------

    fn fixture(name: &str) -> String {
        fs::read_to_string(repo().join(format!("tests/fixtures/machines/step/{name}.yml"))).unwrap()
    }

    /// `step_parent` invoking `step_child`, whose `child_work` script is `child_work`.
    fn parent_project(child_work: &str) -> Project {
        Project::from_texts(
            &[
                ("step_parent", &fixture("step_parent")),
                ("step_child", &fixture("step_child")),
            ],
            &[("child_work", child_work)],
            None,
        )
    }

    impl Project {
        fn child_dir(&self, id: &str) -> PathBuf {
            self.root().join(".decree/runs").join(id)
        }

        fn child_events(&self, id: &str) -> Vec<Map<String, Value>> {
            read_events(&self.child_dir(id)).unwrap()
        }

        /// The child run id named by the first `waiting` event.
        fn child_id(&self) -> String {
            self.events_of("waiting")[0]["child"]
                .as_str()
                .unwrap()
                .to_string()
        }
    }

    fn is_run_id(id: &str) -> bool {
        let (stamp, hex) = id.split_once('-').unwrap();
        stamp.len() == 16
            && DateTime::parse_from_str(&format!("{stamp}+0000"), "%Y%m%dT%H%M%SZ%z").is_ok()
            && hex.len() == 6
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }

    #[test]
    fn machine_invoke_runs_a_child_run_in_its_own_folder() {
        let p = parent_project("print_env");
        assert_eq!(p.run(), Outcome::Finished("done".into()));
        let child = p.child_id();
        assert!(is_run_id(&child), "{child}");

        // The child's message.md: machine, id, parent, depth, trigger, params, the body.
        let message = fs::read_to_string(p.child_dir(&child).join(MESSAGE_FILE)).unwrap();
        assert_eq!(
            message,
            format!(
                "---\nmachine: step_child\nid: {child}\nparent: {RUN_ID}\ndepth: 1\n\
                 trigger: invoke\nparams:\n  label: release\nstate: done\n---\n{BODY}"
            )
        );
        // An ordinary run with its own events and logs.
        let events = p.child_events(&child);
        let claim = &events[0];
        assert_eq!(claim["machine"], "step_child");
        assert_eq!(claim["run_id"], json!(child));
        assert_eq!(claim["trigger"], "invoke");
        assert_eq!(claim["file"], Value::Null);
        assert_eq!(events.last().unwrap()["type"], "run_finished");
        assert_eq!(events.last().unwrap()["state"], "done");
        let script = events.iter().find(|e| e["type"] == "script").unwrap();
        let log =
            fs::read_to_string(p.child_dir(&child).join(script["log"].as_str().unwrap())).unwrap();
        for line in [
            format!("DECREE_PARENT={RUN_ID}"),
            format!("DECREE_MESSAGE_ID={child}"),
            "DECREE_MACHINE=step_child".to_string(),
            "DECREE_TRIGGER=invoke".to_string(),
            "DECREE_DATA_LABEL=release".to_string(),
            "DECREE_REQUEST=".to_string(),
            "DECREE_REPLY=".to_string(),
        ] {
            assert!(log.lines().any(|l| l == line), "{line}\n{log}");
        }

        // The parent: waiting for the child, received its final state, then the transition.
        let kinds: Vec<String> = p
            .events()
            .iter()
            .map(|e| e["type"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            kinds,
            [
                "transition",
                "waiting",
                "received",
                "script",
                "transition",
                "run_finished"
            ]
        );
        let waiting = &p.events_of("waiting")[0];
        assert_eq!(waiting["state"], "build");
        assert!(waiting.get("wait_id").is_none());
        let received = &p.events_of("received")[0];
        assert_eq!(received["event"], "done");
        assert_eq!(received["child"], json!(child));
        assert!(received.get("wait_id").is_none());
        let taken = &p.events_of("transition")[1];
        assert_eq!(p.transitions()[1], "build done done machine");
        assert_eq!(taken["exit_code"], Value::Null);
        assert_eq!(p.order(), ["build_exit"]);
    }

    #[test]
    fn machine_invoke_takes_the_childs_final_state_and_failed_as_error() {
        let p = parent_project("print_reject");
        assert_eq!(p.run(), Outcome::Finished("rejected".into()));
        assert_eq!(p.events_of("received")[0]["event"], "rejected");
        assert_eq!(p.transitions()[1], "build rejected rejected machine");

        let p = parent_project("exit_three");
        assert_eq!(p.run(), Outcome::Finished("failed".into()));
        let child = p.child_id();
        assert_eq!(p.child_events(&child).last().unwrap()["state"], "failed");
        assert_eq!(p.events_of("received")[0]["event"], "error");
        assert_eq!(p.transitions()[1], "build error failed machine");
    }

    #[test]
    fn machine_invoke_past_max_depth_starts_no_child_and_is_error() {
        let p = parent_project("print_env");
        let input = RunInput {
            depth: 10,
            ..inbox_input()
        };
        let ctx = p.ctx();
        let empty = serde_norway::Mapping::new();
        let outcome = Interpreter::new(&ctx, p.machine(), p.executor("emit", &empty), input)
            .unwrap()
            .start()
            .unwrap();
        assert_eq!(outcome, Outcome::Finished("failed".into()));
        assert!(p.events_of("waiting").is_empty());
        let taken = &p.events_of("transition")[1];
        assert_eq!(p.transitions()[1], "build error failed machine");
        assert_eq!(taken["error"], "max_depth 10 reached");
        let runs: Vec<_> = fs::read_dir(p.root().join(".decree/runs"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(runs, [RUN_ID]);
    }

    /// `step_parent` invoking `step_person`, run until the child waits for a reply.
    fn parent_of_person() -> (Project, String, String) {
        let parent = fixture("step_parent").replace(
            "machine: step_child, params: { label: release }",
            "machine: step_person",
        );
        let p = Project::from_texts(
            &[
                ("step_parent", &parent),
                ("step_person", &fixture("step_person")),
            ],
            &[],
            None,
        );
        let outcome = p.run();
        let child = p.child_id();
        let entered = p
            .child_events(&child)
            .into_iter()
            .find(|e| e["type"] == "transition" && e["to"] == "approval")
            .unwrap();
        let wait_id = format!("{child}.w{}", entered["seq"]);
        assert_eq!(
            outcome,
            Outcome::Child {
                state: "build".into(),
                child: child.clone(),
                outcome: Box::new(Outcome::Waiting {
                    state: "approval".into(),
                    wait_id: wait_id.clone()
                })
            }
        );
        (p, child, wait_id)
    }

    /// Append a `received` event to run `id`, as reply delivery (M4.3) does.
    fn deliver(p: &Project, id: &str, fields: Value) {
        let m = &p.machines[p.child_events(id)[0]["machine"].as_str().unwrap()];
        let mut events = EventLog::open(&p.child_dir(id), id, &m.id, "invoke").unwrap();
        events
            .append("received", fields.as_object().unwrap().clone())
            .unwrap();
    }

    #[test]
    fn child_waiting_for_a_person_leaves_the_parent_waiting_until_the_reply_finishes_both() {
        let (p, child, wait_id) = parent_of_person();
        let ctx = p.ctx();
        let events = p.events();
        assert_eq!(events.last().unwrap()["type"], "waiting");
        assert_eq!(events.last().unwrap()["child"], json!(child));
        assert_eq!(ctx.status(p.machine(), &events, false), RunStatus::Waiting);
        let person = &p.machines["step_person"];
        assert_eq!(
            run_status(person, &p.child_events(&child), false),
            RunStatus::Waiting
        );

        // The reply finishes the child, then the parent.
        deliver(
            &p,
            &child,
            json!({ "wait_id": wait_id, "event": "approve", "file": "reply.md" }),
        );
        assert_eq!(
            continue_run(&ctx, &child).unwrap(),
            Outcome::Finished("done".into())
        );
        let child_events = p.child_events(&child);
        assert_eq!(child_events.last().unwrap()["type"], "run_finished");
        assert_eq!(child_events.last().unwrap()["state"], "done");
        let received = &p.events_of("received")[0];
        assert_eq!(received["child"], json!(child));
        assert_eq!(received["event"], "done");
        assert_eq!(
            p.transitions(),
            ["- claimed build claim", "build done done machine"]
        );
        let events = p.events();
        assert_eq!(events.last().unwrap()["type"], "run_finished");
        assert_eq!(ctx.status(p.machine(), &events, false), RunStatus::Finished);
        // The child's scripts ran in the child; the parent's onexit after it finished.
        assert_eq!(p.order().last().unwrap(), "build_exit");
        assert_eq!(mirrored_state(&p), "done");
    }

    #[test]
    fn parent_of_a_finished_child_is_pending_and_continues() {
        let (p, child, wait_id) = parent_of_person();
        let ctx = p.ctx();
        deliver(
            &p,
            &child,
            json!({ "wait_id": wait_id, "event": "reject", "file": "reply.md" }),
        );
        // Continue only the child, as if decree stopped before continuing the parent.
        let person = &p.machines["step_person"];
        let executor = ctx
            .executor(
                person,
                &child,
                "invoke",
                &serde_norway::Mapping::new(),
                Some(RUN_ID),
            )
            .unwrap();
        let input = RunInput {
            depth: 1,
            ..inbox_input()
        };
        let outcome = Interpreter::new(&ctx, person, executor, input)
            .unwrap()
            .resume()
            .unwrap();
        assert_eq!(outcome, Outcome::Finished("rejected".into()));
        let events = p.events();
        assert_eq!(run_status(p.machine(), &events, false), RunStatus::Waiting);
        assert_eq!(ctx.status(p.machine(), &events, false), RunStatus::Pending);

        assert_eq!(
            continue_run(&ctx, RUN_ID).unwrap(),
            Outcome::Finished("rejected".into())
        );
        assert_eq!(p.transitions()[1], "build rejected rejected machine");
    }

    #[test]
    fn parent_of_an_unfinished_child_does_not_continue() {
        let (p, child, _) = parent_of_person();
        let err = continue_run(&p.ctx(), RUN_ID).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("cannot continue the run: child run `{child}` has not finished")
        );
        assert_eq!(p.events().last().unwrap()["type"], "waiting");
    }

    #[test]
    fn interrupted_child_leaves_the_parent_waiting() {
        let p = parent_project("print_env");
        p.shutdown.store(true, Ordering::SeqCst);
        let outcome = p.run();
        let child = p.child_id();
        assert_eq!(
            outcome,
            Outcome::Child {
                state: "build".into(),
                child: child.clone(),
                outcome: Box::new(Outcome::Interrupted("work".into()))
            }
        );
        assert_eq!(
            p.child_events(&child).last().unwrap()["type"],
            "interrupted"
        );
        let events = p.events();
        assert_eq!(events.last().unwrap()["type"], "waiting");
        assert_eq!(
            p.ctx().status(p.machine(), &events, false),
            RunStatus::Waiting
        );
    }

    #[test]
    fn run_ids_are_unique_and_skip_ids_taken_in_the_inbox() {
        let tmp = TempDir::new().unwrap();
        let decree = tmp.path().join(DECREE_DIR);
        let mut ids = BTreeSet::new();
        for _ in 0..50 {
            let (id, dir) = create_run_dir(&decree).unwrap();
            assert!(is_run_id(&id), "{id}");
            assert!(dir.is_dir());
            assert!(ids.insert(id));
        }
        // Every next id of this second is queued in the inbox: the run takes another.
        fs::create_dir_all(decree.join(INBOX_DIR)).unwrap();
        let (id, _) = create_run_dir(&decree).unwrap();
        let stamp = id.split_once('-').unwrap().0.to_string();
        let low = u32::from_str_radix(id.split_once('-').unwrap().1, 16).unwrap();
        for k in 1..=3 {
            let next = format!("{stamp}-{:06x}", (low + k) & 0xff_ffff);
            fs::write(decree.join(INBOX_DIR).join(format!("{next}.md")), "").unwrap();
        }
        let _ = id;
    }

    // ---------------------------------------------------------------
    // W3C SCXML IRP tests (tests/fixtures/scxml/)
    // ---------------------------------------------------------------

    #[test]
    fn scxml_irp_fixtures_pass_and_the_readme_lists_the_rest() {
        let dir = repo().join("tests/fixtures/scxml");
        let mut ported = BTreeSet::new();
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("yml") {
                continue;
            }
            let stem = path.file_stem().unwrap().to_str().unwrap().to_string();
            let text = fs::read_to_string(&path).unwrap();
            let m = load_machine_text(&stem, &path, &text).unwrap();
            let p = Project::new("step_normal", &[]);
            let params = serde_norway::Mapping::new();
            let input = RunInput {
                file: Some("irp.md".to_string()),
                ..RunInput::default()
            };
            let ctx = p.ctx();
            let outcome = Interpreter::new(&ctx, &m, p.executor("inbox", &params), input)
                .unwrap()
                .start()
                .unwrap();
            assert_eq!(outcome, Outcome::Finished("pass".into()), "{stem}");
            ported.insert(stem.trim_start_matches("test").to_string());
        }
        let readme = fs::read_to_string(dir.join("README.md")).unwrap();
        let listed: Vec<String> = readme
            .lines()
            .filter_map(|l| l.strip_prefix("| "))
            .filter_map(|l| l.split(' ').next())
            .filter(|id| id.chars().all(|c| c.is_ascii_digit()) && !id.is_empty())
            .map(str::to_string)
            .collect();
        let unique: BTreeSet<&String> = listed.iter().collect();
        assert_eq!(unique.len(), listed.len(), "a test is listed twice");
        assert!(listed.iter().all(|id| !ported.contains(id)));
        // The IRP manifest of 10 March 2015 has 200 tests.
        assert_eq!(listed.len() + ported.len(), 200);
    }

    // ---------------------------------------------------------------
    // events.jsonl: every section 7 field of the four types
    // ---------------------------------------------------------------

    #[test]
    fn every_transition_script_and_run_finished_field_appears() {
        let mut seen: BTreeMap<String, HashSet<String>> = BTreeMap::new();
        let mut collect = |p: &Project| {
            for e in p.events() {
                let kind = e["type"].as_str().unwrap().to_string();
                seen.entry(kind).or_default().extend(e.keys().cloned());
            }
        };

        let p = Project::new("step_exit_fail", &[]);
        p.run();
        collect(&p);
        let p = Project::new("step_normal", &[("a_invoke", "print_undeclared")]);
        p.run();
        collect(&p);
        let p = Project::new("step_timeout", &[("sleep_long", "sleep_long")]);
        p.run();
        collect(&p);
        // `error` is only on `invalid_message` events.
        let p = Project::new("step_normal", &[]);
        let mut log = p.executor("inbox", &serde_norway::Mapping::new());
        reject(
            log.events(),
            &p.run_dir(),
            "bad.md",
            "unknown machine `x`",
            true,
        )
        .unwrap();
        collect(&p);

        let common = ["v", "seq", "ts", "type", "run_id", "machine", "trigger"];
        let expected: [(&str, &[&str]); 3] = [
            (
                "transition",
                &[
                    "from",
                    "event",
                    "to",
                    "source",
                    "exit_code",
                    "invalid_event",
                    "exit_failures",
                    "file",
                    "error",
                ],
            ),
            (
                "script",
                &[
                    "state",
                    "phase",
                    "script",
                    "path",
                    "attempt",
                    "started_at",
                    "duration_ms",
                    "exit_code",
                    "timed_out",
                    "log",
                ],
            ),
            ("run_finished", &["state", "duration_ms"]),
        ];
        for (kind, fields) in expected {
            let keys = &seen[kind];
            let want: HashSet<String> =
                common.iter().chain(fields).map(|s| s.to_string()).collect();
            assert_eq!(keys, &want, "{kind}");
        }
    }

    #[test]
    fn reject_starts_the_run_in_failed() {
        let p = Project::new("step_normal", &[]);
        let mut executor = p.executor("inbox", &serde_norway::Mapping::new());
        reject(
            executor.events(),
            &p.run_dir(),
            "bad.md",
            "params: unknown name `x`",
            true,
        )
        .unwrap();
        let events = p.events();
        assert_eq!(events.len(), 1);
        let t = &events[0];
        assert_eq!(t["from"], Value::Null);
        assert_eq!(t["to"], "failed");
        assert_eq!(t["source"], "invalid_message");
        assert_eq!(t["error"], "params: unknown name `x`");
        assert_eq!(t["file"], "bad.md");
        assert_eq!(mirrored_state(&p), "failed");
        assert_eq!(run_status(p.machine(), &events, false), RunStatus::Finished);
        assert!(p.order().is_empty());
    }

    // ---------------------------------------------------------------
    // Run status (section 4)
    // ---------------------------------------------------------------

    #[test]
    fn run_status_follows_the_section_4_order() {
        let p = Project::new("step_normal", &[]);
        let m = p.machine();
        let event = |v: Value| v.as_object().unwrap().clone();
        let claim = event(json!({"type": "transition", "to": "a", "source": "claim"}));
        let finished = event(json!({"type": "transition", "to": "done", "source": "exit_code"}));
        let waiting = event(json!({"type": "waiting", "state": "a"}));
        let received = event(json!({"type": "received", "event": "x"}));
        let retry = event(json!({"type": "transition", "to": "a", "source": "retry"}));
        let script = event(json!({"type": "script", "state": "a"}));

        assert_eq!(
            run_status(m, &[claim.clone(), finished], true),
            RunStatus::Finished
        );
        assert_eq!(
            run_status(m, std::slice::from_ref(&claim), true),
            RunStatus::Active
        );
        assert_eq!(
            run_status(m, &[claim.clone(), waiting], false),
            RunStatus::Waiting
        );
        assert_eq!(
            run_status(m, &[claim.clone(), received], false),
            RunStatus::Pending
        );
        assert_eq!(
            run_status(m, &[claim.clone(), retry], false),
            RunStatus::Pending
        );
        assert_eq!(
            run_status(m, &[claim.clone(), script], false),
            RunStatus::Interrupted
        );
        assert_eq!(run_status(m, &[claim], false), RunStatus::Interrupted);
        assert_eq!(run_status(m, &[], false), RunStatus::Interrupted);
    }

    // ---------------------------------------------------------------
    // Mirroring `state` into message.md
    // ---------------------------------------------------------------

    fn mirror(text: &[u8]) -> Result<Vec<u8>, InterpreterError> {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(MESSAGE_FILE);
        fs::write(&path, text).unwrap();
        mirror_state(&path, "verify")?;
        assert!(!tmp.path().join(".message.md.tmp").exists());
        Ok(fs::read(&path).unwrap())
    }

    #[test]
    fn mirror_keeps_keys_order_and_body_bytes() {
        let out = mirror(b"---\nzeta: 1\nid: x\nstate: old\ncustom: [a, b]\n---\r\nbody\r\nmore\n")
            .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "---\nzeta: 1\nid: x\nstate: verify\ncustom:\n- a\n- b\n---\nbody\r\nmore\n"
        );
    }

    #[test]
    fn mirror_reads_bom_crlf_and_trailing_spaces_on_fences() {
        let out = mirror("\u{feff}---  \r\nid: x\r\n--- \r\nbody\r\n".as_bytes()).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "---\nid: x\nstate: verify\n---\nbody\r\n"
        );
    }

    #[test]
    fn mirror_adds_frontmatter_to_a_message_without_one() {
        let out = mirror(b"# Just a body\n").unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "---\nstate: verify\n---\n# Just a body\n"
        );
    }

    #[test]
    fn mirror_rejects_an_unclosed_fence_and_duplicate_keys() {
        let err = mirror(b"---\nid: x\nbody\n").unwrap_err().to_string();
        assert!(
            err.contains("line 1: frontmatter has no closing `---`"),
            "{err}"
        );
        let err = mirror(b"---\nid: x\nid: y\n---\n").unwrap_err().to_string();
        assert!(err.contains("frontmatter"), "{err}");
    }
}
