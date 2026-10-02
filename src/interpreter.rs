//! The step loop (spec section 7): moves one run through one machine with SCXML's exit
//! and entry order, asks the router in router states, and appends every step to the run's
//! `events.jsonl`. State, status and visits are derived from that log (section 4, Source
//! of truth).
//!
//! Interpreted here: the whole section 5 subset. Transitions on compound states, with
//! events bubbling from the atomic state outward; `type: internal`; final states at any
//! level, a nested one raising `done.state.<parent>`; and waiting states, which append
//! `waiting` and stop until a `received` event continues the run. Delivering replies is
//! ticket M4.3; `decree retry` and the run lock are ticket M4.2.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};

use crate::cond::{self, CondError};
use crate::config::{DECREE_DIR, PROCESSED_FILE};
use crate::machine::{event_matches, LoadedMachine};
use crate::router::{Router, RouterOption, RouterReply, RouterRequest};
use crate::runtime::{
    is_reserved_event, timestamp, EventLog, Execution, Executor, InvokeEvent, Phase, RuntimeError,
    ScriptRun, EVENTS_FILE, MESSAGE_FILE, RECEIVED_DIR, ROOT_STATE,
};

/// Root-level final state an unhandled `error` goes to (section 5, Rules).
const FAILED: &str = "failed";

/// Script name in the router log's filename, `NNNN-<state>-_router.log`.
const ROUTER_LOG: &str = "_router";

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

    #[error("machine `{machine}`: {at}: cond `{cond}`: {source}")]
    Cond {
        machine: String,
        at: String,
        cond: String,
        source: CondError,
    },
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> InterpreterError + '_ {
    move |source| InterpreterError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// What the interpreter needs from the claimed message.
#[derive(Debug, Clone)]
pub struct RunInput<'a> {
    /// Frontmatter `params`, already validated against the machine's `data`.
    pub params: &'a serde_norway::Mapping,
    /// The message body, for the router.
    pub message_body: &'a str,
    /// Original inbox or migration filename, recorded on the claim event. For a migration
    /// (`trigger: migration`) it is also the `processed.md` ledger line.
    pub file: &'a str,
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
    /// The run entered this waiting state, its `onentry` scripts ran, and a `waiting`
    /// event was appended. A reply must name `wait_id` (section 4, Events for waiting runs).
    Waiting { state: String, wait_id: String },
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
    machine: &'a LoadedMachine,
    executor: Executor,
    router: &'a dyn Router,
    data: BTreeMap<String, cond::Value>,
    message_body: String,
    file: String,
    /// The atomic state the run is in, for the `interrupted` event.
    current: usize,
    /// `seq` of the last `transition` event, which names the wait id of a waiting state.
    entered_seq: u64,
}

impl<'a> Interpreter<'a> {
    pub fn new(
        machine: &'a LoadedMachine,
        executor: Executor,
        router: &'a dyn Router,
        input: RunInput,
    ) -> Result<Self, InterpreterError> {
        Ok(Interpreter {
            machine,
            data: data_values(machine, input.params)?,
            executor,
            router,
            message_body: input.message_body.to_string(),
            file: input.file.to_string(),
            current: 0,
            entered_seq: 0,
        })
    }

    /// Start a new run (step 1) and step it until it finishes, waits or is interrupted.
    pub fn start(&mut self) -> Result<Outcome, InterpreterError> {
        let result = self.claim_and_run();
        self.interrupt_on_signal(result)
    }

    /// Continue a waiting run whose last event is `received` (step 1): take that event's
    /// transition at step 4 with `source: "external"`. Nothing is re-run, because the run
    /// only paused.
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
        let received = events
            .last()
            .filter(|e| e.get("type").and_then(Value::as_str) == Some("received"))
            .ok_or_else(|| not_received("its last event is not `received`"))?;
        let event = received
            .get("event")
            .and_then(Value::as_str)
            .ok_or_else(|| not_received("the `received` event has no `event`"))?;
        let s = current_state(&events)
            .and_then(|id| m.find(id))
            .filter(|&s| is_waiting(m, s))
            .ok_or_else(|| not_received("its current state is not a waiting state"))?;
        self.entered_seq = events
            .iter()
            .rev()
            .find(|e| is_transition(e))
            .and_then(|e| e.get("seq"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        // `DECREE_RECEIVED`: the last reply, not a timeout, which has no file.
        let run_dir = self.executor.info().run_dir.clone();
        self.executor.received = events
            .iter()
            .rev()
            .filter(|e| e.get("type").and_then(Value::as_str) == Some("received"))
            .find_map(|e| e.get("file").and_then(Value::as_str))
            .map(|file| run_dir.join(RECEIVED_DIR).join(file));
        self.current = s;
        self.step_from(s, Some(Decision::new(event, "external", None)))
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
                None => self.decide(s)?,
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

    /// Step 8, and the waiting stop of step 3, once `t` and the states above it have been
    /// entered. `failed_at` is the state whose `onentry` script failed, if one did.
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
        // `done.state.<id>` or a wait.
        if let Some(n) = failed_at {
            return Ok(Next::Step(Some(Decision::entry_error(n == 0))));
        }
        // A nested final state raises `done.state.<parent>`, handled before anything else.
        if let Some(p) = node.parent.filter(|_| node.is_final) {
            let event = format!("done.state.{}", m.nodes[p].id);
            return Ok(Next::Step(Some(Decision::new(&event, "internal", None))));
        }
        if is_waiting(m, t) {
            return self.wait(t).map(Next::Stop);
        }
        Ok(Next::Step(None))
    }

    /// Step 3 for waiting state `s`, after its `onentry` scripts ran: append `waiting` and
    /// stop (section 4, Events for waiting runs).
    fn wait(&mut self, s: usize) -> Result<Outcome, InterpreterError> {
        let node = &self.machine.nodes[s];
        let wait_id = self.wait_id();
        let timeout_at = node.timeout_s.map(|secs| {
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
                "accepts": accepts(self.machine, s),
                "timeout_at": timeout_at,
            }),
        )?;
        Ok(Outcome::Waiting {
            state: node.id.clone(),
            wait_id,
        })
    }

    /// `<run id>.w<seq>`, where `seq` is that of the `transition` event that entered the
    /// current state.
    fn wait_id(&self) -> String {
        format!("{}.w{}", self.executor.info().run_id, self.entered_seq)
    }

    /// Steps 2 and 3 for atomic state `s`: run the invoke, then take its event, a
    /// pass-through `done`, or the router's choice.
    fn decide(&mut self, s: usize) -> Result<Decision, InterpreterError> {
        let m = self.machine;
        let node = &m.nodes[s];
        let visits = self.visits_of(s)?;
        match self.executor.run_invoke(m, s, visits)? {
            Some(out) => {
                let exit_code = out.execution.exit_code;
                Ok(match out.event {
                    InvokeEvent::ExitCode(e) => Decision::new(e, "exit_code", exit_code),
                    InvokeEvent::Stdout(e) => Decision::new(&e, "stdout", exit_code),
                    InvokeEvent::Invalid(e) => Decision {
                        invalid_event: Some(e),
                        ..Decision::new("error", "stdout", exit_code)
                    },
                    InvokeEvent::Router => self.route(s, Some(&out.execution))?,
                })
            }
            None if node.router.is_some() => self.route(s, None),
            None if m.handles(s, "done") => Ok(Decision::new("done", "exit_code", None)),
            // `after_entry` stops in a waiting state before it is stepped.
            None => Err(self.invalid(format!(
                "`{}` waits for an external event and cannot be stepped",
                node.id
            ))),
        }
    }

    /// Router steps 1–6 for router state `s`. Appends the `router` event.
    fn route(
        &mut self,
        s: usize,
        execution: Option<&Execution>,
    ) -> Result<Decision, InterpreterError> {
        let m = self.machine;
        let node = &m.nodes[s];
        let start = Instant::now();
        let exit_code = execution.and_then(|e| e.exit_code);

        // 1. Options: every event but `error`, without those whose `cond` is false.
        let visits = visits(&self.read_events()?);
        let mut options = Vec::new();
        for edge in node.transitions.iter().filter(|e| e.event != "error") {
            if let Some(text) = &edge.cond {
                let cond_err = |source| InterpreterError::Cond {
                    machine: m.id.clone(),
                    at: m.state_path(s),
                    cond: text.clone(),
                    source,
                };
                let parsed = cond::parse(text).map_err(cond_err)?;
                if !parsed.eval(&self.data, &visits).map_err(cond_err)? {
                    continue;
                }
            }
            options.push(RouterOption {
                event: edge.event.clone(),
                description: edge.description.clone().unwrap_or_default(),
            });
        }
        let names: Vec<String> = options.iter().map(|o| o.event.clone()).collect();
        let mut fields = json!({ "state": node.id, "options": names });

        // 2. One option is taken without asking.
        if let [only] = names.as_slice() {
            fields["event"] = json!(only);
            fields["source"] = json!("single_option");
            fields["duration_ms"] = json!(start.elapsed().as_millis() as u64);
            self.append("router", fields)?;
            return Ok(Decision::new(only, "single_option", exit_code));
        }

        // 3–5. Ask, validate, ask again once.
        let mut request = RouterRequest {
            machine: m.root().id.clone(),
            machine_description: m.description().to_string(),
            state: node.id.clone(),
            state_description: node.description.clone().unwrap_or_default(),
            options,
            step_output: execution
                .map(|e| e.stdout_tail.join("\n"))
                .unwrap_or_default(),
            message_body: self.message_body.clone(),
            previous_error: None,
        };
        let log = self.executor.reserve_log(&node.id, ROUTER_LOG);
        let mut log_text = String::new();
        let mut rejections = Vec::new();
        let mut accepted: Option<RouterReply> = None;
        let mut asks = 0;
        while asks < 2 && accepted.is_none() {
            asks += 1;
            let result = self.router.decide(&request);
            let line = match &result {
                Ok(reply) => json!({ "ask": asks, "request": request, "reply": reply }),
                Err(e) => json!({ "ask": asks, "request": request, "error": e.0 }),
            };
            log_text.push_str(&line.to_string());
            log_text.push('\n');
            match result {
                Ok(reply) if names.contains(&reply.event) => accepted = Some(reply),
                Ok(reply) => rejections.push(format!(
                    "`{}` is not one of the options: {}",
                    reply.event,
                    names.join(", ")
                )),
                Err(e) => rejections.push(format!("the router failed: {e}")),
            }
            request.previous_error = rejections.last().cloned();
        }
        let log_path = self.executor.info().run_dir.join(&log);
        fs::write(&log_path, log_text).map_err(io_err(&log_path))?;

        let source = match &accepted {
            Some(reply) => {
                fields["event"] = json!(reply.event);
                fields["source"] = json!("llm");
                if let Some(reason) = &reply.reason {
                    fields["reason"] = json!(reason);
                }
                if let Some(confidence) = reply.confidence {
                    fields["confidence"] = json!(confidence);
                }
                if let Some(probabilities) = &reply.probabilities {
                    fields["probabilities"] = json!(probabilities);
                }
                "llm"
            }
            // 6. Fall back to `default`.
            None => {
                let default = node.default.as_deref().ok_or_else(|| {
                    self.invalid(format!("router state `{}` has no default", node.id))
                })?;
                fields["event"] = json!(default);
                fields["source"] = json!("default");
                let numbered: Vec<String> = rejections
                    .iter()
                    .enumerate()
                    .map(|(i, r)| format!("ask {}: {r}", i + 1))
                    .collect();
                fields["router_error"] = json!(numbered.join("; "));
                "default"
            }
        };
        fields["asks"] = json!(asks);
        fields["duration_ms"] = json!(start.elapsed().as_millis() as u64);
        fields["log"] = json!(log);
        let event = fields["event"].as_str().unwrap_or_default().to_string();
        self.append("router", fields)?;
        Ok(Decision::new(&event, source, exit_code))
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
    /// first that exits non-zero and returns the state it belongs to. A waiting state's
    /// scripts see its wait id and accepted events.
    fn run_entry(&mut self, states: &[usize]) -> Result<Option<usize>, InterpreterError> {
        let m = self.machine;
        for &n in states {
            let (name, visits) = self.script_state(n)?;
            let max_attempts = self.executor.max_attempts(m, n);
            let (wait_id, accepts) = if n != 0 && is_waiting(m, n) {
                (self.wait_id(), accepts(m, n))
            } else {
                (String::new(), Vec::new())
            };
            for script in &m.nodes[n].onentry {
                let execution = self.executor.run_script(&ScriptRun {
                    visits,
                    max_attempts,
                    wait_id: &wait_id,
                    accepts: &accepts,
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
        let mut failures = Vec::new();
        for script in &m.nodes[n].onexit {
            let execution = self.executor.run_script(&ScriptRun {
                visits,
                max_attempts,
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
        (self.executor.info().trigger == "migration" && finishes).then(|| self.file.clone())
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

/// A waiting state (section 5, Kinds of state): atomic, no `invoke`, not a router, and no
/// `done`, itself or through an ancestor.
fn is_waiting(m: &LoadedMachine, n: usize) -> bool {
    let node = &m.nodes[n];
    !node.is_final
        && !m.is_compound(n)
        && node.invoke.is_none()
        && node.router.is_none()
        && !m.handles(n, "done")
}

/// A final state whose parent is the root: entering it ends the run.
fn is_root_final(m: &LoadedMachine, n: usize) -> bool {
    m.nodes[n].is_final && m.nodes[n].parent == Some(0)
}

/// The events waiting state `n` accepts from a reply, in name order: every event of its
/// own transitions and its ancestors' (section 5, Event matching), except the reserved
/// ones (section 5, Rules).
fn accepts(m: &LoadedMachine, n: usize) -> Vec<String> {
    let mut events: Vec<String> = m
        .chain(n)
        .flat_map(|a| m.nodes[a].transitions.iter())
        .map(|e| e.event.clone())
        .filter(|e| !is_reserved_event(e))
        .collect();
    events.sort();
    events.dedup();
    events
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
    let message_err = |message: String| InterpreterError::Message {
        path: path.to_path_buf(),
        message,
    };
    let bytes = fs::read(path).map_err(io_err(path))?;
    let text = std::str::from_utf8(&bytes).map_err(|e| message_err(e.to_string()))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    let mut lines = text.split_inclusive('\n');
    let (mut mapping, body) = match lines.next() {
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
            (mapping, &text[offset..])
        }
        _ => (serde_norway::Mapping::new(), text),
    };
    mapping.insert("state".into(), state.into());
    let yaml = serde_norway::to_string(&mapping).map_err(|e| message_err(e.to_string()))?;
    let mut out = format!("---\n{yaml}---\n").into_bytes();
    out.extend_from_slice(body.as_bytes());
    write_replace(path, &out)
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
    use crate::router::{RouterError, ScriptedRouter};
    use crate::runtime::{data_env, RunInfo};
    use std::collections::{BTreeSet, HashSet};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use tempfile::TempDir;

    const RUN_ID: &str = "20261001T143005Z-3fa9c1";
    const BODY: &str = "# Task\r\nDo the thing.\n";

    fn repo() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    /// A temp project holding fixture machine `tests/fixtures/machines/step/<name>.yml`, a run
    /// folder with its `message.md`, and every script the machine names: `record.sh`
    /// installed under that name, unless `scripts` maps the name to another fixture.
    struct Project {
        tmp: TempDir,
        machine: LoadedMachine,
        shutdown: Arc<AtomicBool>,
    }

    impl Project {
        fn new(name: &str, scripts: &[(&str, &str)]) -> Self {
            let tmp = TempDir::new().unwrap();
            let decree = tmp.path().join(DECREE_DIR);
            let fixture = repo().join(format!("tests/fixtures/machines/step/{name}.yml"));
            let text = fs::read_to_string(&fixture).unwrap();
            let machine = load_machine_text(name, &fixture, &text).unwrap();

            let script_dir = decree.join("scripts");
            fs::create_dir_all(&script_dir).unwrap();
            let fixtures = repo().join("tests/fixtures/scripts");
            for node in &machine.nodes {
                for script in node.invoke.iter().chain(&node.onentry).chain(&node.onexit) {
                    let file = scripts
                        .iter()
                        .find(|(s, _)| s == script)
                        .map_or("record", |(_, f)| f);
                    // fs::copy keeps the fixture's executable bit.
                    fs::copy(
                        fixtures.join(format!("{file}.sh")),
                        script_dir.join(format!("{script}.sh")),
                    )
                    .unwrap();
                }
            }
            // Every fixture machine passes `decree check`.
            let ids = BTreeSet::from([name.to_string()]);
            let env = CheckEnv {
                decree_dir: &decree,
                shared_source: None,
                machine_ids: &ids,
            };
            let problems = machine.validate(&text, &env);
            assert!(problems.is_empty(), "{name}: {problems:?}");

            let project = Project {
                tmp,
                machine,
                shutdown: Arc::new(AtomicBool::new(false)),
            };
            fs::create_dir_all(project.run_dir()).unwrap();
            let message =
                format!("---\nid: {RUN_ID}\nmachine: {name}\ntrigger: inbox\n---\n{BODY}");
            fs::write(project.run_dir().join(MESSAGE_FILE), message).unwrap();
            project
        }

        fn root(&self) -> PathBuf {
            self.tmp.path().to_path_buf()
        }

        fn run_dir(&self) -> PathBuf {
            self.root().join(".decree/runs").join(RUN_ID)
        }

        fn executor(&self, trigger: &str, params: &serde_norway::Mapping) -> Executor {
            let info = RunInfo {
                project_root: self.root(),
                shared_source: None,
                run_dir: self.run_dir(),
                run_id: RUN_ID.to_string(),
                machine: self.machine.id.clone(),
                trigger: trigger.to_string(),
                data: data_env(&self.machine.data, params),
                max_attempts: 3,
                max_log_size: 0,
            };
            Executor::open(info, Arc::clone(&self.shutdown)).unwrap()
        }

        fn run(&self, router: &ScriptedRouter) -> Outcome {
            self.run_with(router, "inbox", "inbox.md", &serde_norway::Mapping::new())
        }

        fn run_with(
            &self,
            router: &ScriptedRouter,
            trigger: &str,
            file: &str,
            params: &serde_norway::Mapping,
        ) -> Outcome {
            let input = RunInput {
                params,
                message_body: BODY,
                file,
            };
            Interpreter::new(&self.machine, self.executor(trigger, params), router, input)
                .unwrap()
                .start()
                .unwrap()
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

    fn no_router() -> ScriptedRouter {
        ScriptedRouter::new([])
    }

    fn reply(event: &str) -> Result<RouterReply, RouterError> {
        Ok(RouterReply::event(event))
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
        assert_eq!(p.run(&no_router()), Outcome::Finished("done".into()));
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
        assert_eq!(run_status(&p.machine, &events, false), RunStatus::Finished);
    }

    #[test]
    fn order_self_transition_exits_and_reenters_the_state() {
        let p = Project::new("step_self", &[]);
        let router = ScriptedRouter::new([reply("again"), reply("finish")]);
        assert_eq!(p.run(&router), Outcome::Finished("done".into()));
        assert_eq!(
            p.order(),
            [
                "root_entry",
                "a_entry",
                "a_invoke",
                "a_exit",
                "a_entry",
                "a_invoke",
                "a_exit",
                "done_entry",
                "root_exit"
            ]
        );
        assert_eq!(
            p.transitions(),
            ["- claimed a claim", "a again a llm", "a finish done llm"]
        );
        assert_eq!(visits(&p.events())["a"], 2);
    }

    #[test]
    fn order_entering_and_leaving_a_compound_state() {
        let p = Project::new("step_compound", &[]);
        assert_eq!(p.run(&no_router()), Outcome::Finished("done".into()));
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
        assert_eq!(p.run(&no_router()), Outcome::Finished("failed".into()));
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
        assert_eq!(p.run(&no_router()), Outcome::Finished("failed".into()));
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
        assert_eq!(p.run(&no_router()), Outcome::Finished("failed".into()));
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
        assert_eq!(p.run(&no_router()), Outcome::Finished("done".into()));
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
        assert_eq!(p.run(&no_router()), Outcome::Finished("failed".into()));
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
        assert_eq!(p.run(&no_router()), Outcome::Finished("failed".into()));
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
        let outcome = p.run_with(
            &no_router(),
            "migration",
            "45-next.md",
            &serde_norway::Mapping::new(),
        );
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
        let outcome = p.run_with(
            &no_router(),
            "migration",
            "45-next.md",
            &serde_norway::Mapping::new(),
        );
        assert_eq!(outcome, Outcome::Finished("failed".into()));
        assert_eq!(p.processed(), "44-prev.md\n");
    }

    #[test]
    fn failed_migration_writes_no_ledger_line() {
        let p = Project::new("step_error", &[]);
        p.run_with(
            &no_router(),
            "migration",
            "45-next.md",
            &serde_norway::Mapping::new(),
        );
        assert_eq!(p.processed(), "");
    }

    // ---------------------------------------------------------------
    // Attempts and visits
    // ---------------------------------------------------------------

    #[test]
    fn invoke_failing_twice_then_succeeding_takes_done_after_two_attempts() {
        let p = Project::new("step_attempts", &[("fail_until_final", "fail_until_final")]);
        assert_eq!(p.run(&no_router()), Outcome::Finished("done".into()));
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
    fn visits_cond_ends_a_retry_loop_after_two_visits() {
        let p = Project::new(
            "step_retry_loop",
            &[
                ("fail_until_final", "fail_until_final"),
                ("verify", "exit_zero"),
            ],
        );
        // The router would retry forever; the cond stops it.
        let router = ScriptedRouter::new([reply("retry"), reply("retry")]);
        assert_eq!(p.run(&router), Outcome::Finished("done".into()));
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
                "verify retry implement llm",
                "implement error implement attempt",
                "implement done verify exit_code",
                "verify pass done single_option"
            ]
        );
        let routers = p.events_of("router");
        assert_eq!(routers[0]["options"], json!(["pass", "retry"]));
        assert_eq!(routers[1]["options"], json!(["pass"]));
        // The second decision did not ask.
        assert_eq!(router.replies.borrow().len(), 1);
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
    // Router steps 1–6 (section 7, Router)
    // ---------------------------------------------------------------

    fn router_project() -> Project {
        Project::new("step_router", &[("verify", "exit_zero")])
    }

    #[test]
    fn router_valid_reply_is_taken_with_source_llm() {
        let p = router_project();
        let router = ScriptedRouter::new([Ok(RouterReply {
            event: "pass".into(),
            reason: Some("All tests pass.".into()),
            confidence: Some(0.9),
            probabilities: Some(BTreeMap::from([
                ("ask".to_string(), 0.05),
                ("pass".to_string(), 0.9),
                ("retry".to_string(), 0.05),
            ])),
        })]);
        assert_eq!(p.run(&router), Outcome::Finished("done".into()));
        assert_eq!(
            p.transitions(),
            ["- claimed verify claim", "verify pass done llm"]
        );
        assert_eq!(p.events_of("transition")[1]["exit_code"], 0);
        let events = p.events();
        // The router event comes right before the transition it causes.
        let i = events.iter().position(|e| e["type"] == "router").unwrap();
        assert_eq!(events[i + 1]["type"], "transition");
        let r = &events[i];
        assert_eq!(r["state"], "verify");
        assert_eq!(r["options"], json!(["ask", "pass", "retry"]));
        assert_eq!(r["event"], "pass");
        assert_eq!(r["source"], "llm");
        assert_eq!(r["reason"], "All tests pass.");
        assert_eq!(r["confidence"], 0.9);
        assert_eq!(r["probabilities"]["pass"], 0.9);
        assert_eq!(r["asks"], 1);
        assert!(r["duration_ms"].as_u64().is_some());
        assert!(r.get("router_error").is_none());
        // The router log holds the request: options with descriptions, the invoke's
        // stdout tail and the message body.
        let log = r["log"].as_str().unwrap();
        assert!(log.ends_with("-verify-_router.log"), "{log}");
        let text = fs::read_to_string(p.run_dir().join(log)).unwrap();
        let line: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        let request = &line["request"];
        assert_eq!(request["machine"], "step_router");
        assert_eq!(request["state"], "verify");
        assert_eq!(
            request["state_description"],
            "Tests have run; decide what happens next."
        );
        assert_eq!(request["options"][1]["event"], "pass");
        assert_eq!(
            request["options"][1]["description"],
            "All acceptance criteria are met."
        );
        assert_eq!(request["step_output"], "hello");
        assert_eq!(request["message_body"], BODY);
        assert_eq!(request["previous_error"], Value::Null);
        assert_eq!(line["reply"]["event"], "pass");
    }

    #[test]
    fn router_invalid_event_then_valid_takes_the_second_reply() {
        let p = router_project();
        let router = ScriptedRouter::new([reply("bogus"), reply("retry")]);
        assert_eq!(p.run(&router), Outcome::Finished("retried".into()));
        assert_eq!(
            p.transitions(),
            ["- claimed verify claim", "verify retry retried llm"]
        );
        let r = &p.events_of("router")[0];
        assert_eq!(r["source"], "llm");
        assert_eq!(r["asks"], 2);
        let log = fs::read_to_string(p.run_dir().join(r["log"].as_str().unwrap())).unwrap();
        let lines: Vec<Value> = log
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[1]["request"]["previous_error"],
            "`bogus` is not one of the options: ask, pass, retry"
        );
    }

    #[test]
    fn router_two_rejected_replies_take_default_with_router_error() {
        let p = router_project();
        let router = ScriptedRouter::new([Err(RouterError("backend down".into())), reply("Pass")]);
        assert_eq!(p.run(&router), Outcome::Finished("asked".into()));
        assert_eq!(
            p.transitions(),
            ["- claimed verify claim", "verify ask asked default"]
        );
        let r = &p.events_of("router")[0];
        assert_eq!(r["event"], "ask");
        assert_eq!(r["source"], "default");
        assert_eq!(r["asks"], 2);
        // Never fuzzy-matched: `Pass` is not `pass`.
        assert_eq!(
            r["router_error"],
            "ask 1: the router failed: backend down; \
             ask 2: `Pass` is not one of the options: ask, pass, retry"
        );
    }

    #[test]
    fn router_empty_queue_counts_as_a_failure() {
        let p = router_project();
        assert_eq!(p.run(&no_router()), Outcome::Finished("asked".into()));
        assert_eq!(p.events_of("router")[0]["source"], "default");
    }

    #[test]
    fn router_single_option_after_conds_does_not_ask() {
        let p = router_project();
        let router = ScriptedRouter::new([reply("pass")]);
        let outcome = p.run_with(&router, "inbox", "inbox.md", &params("open: false"));
        assert_eq!(outcome, Outcome::Finished("asked".into()));
        assert_eq!(
            p.transitions(),
            ["- claimed verify claim", "verify ask asked single_option"]
        );
        let r = &p.events_of("router")[0];
        assert_eq!(r["options"], json!(["ask"]));
        assert_eq!(r["source"], "single_option");
        assert!(r.get("asks").is_none());
        assert!(r.get("log").is_none());
        assert!(r["duration_ms"].as_u64().is_some());
        // The queue is untouched.
        assert_eq!(router.replies.borrow().len(), 1);
        assert_eq!(router.replies.borrow()[0], reply("pass"));
    }

    #[test]
    fn printed_event_in_a_router_state_skips_the_router() {
        let p = Project::new("step_router", &[("verify", "print_pass")]);
        let router = ScriptedRouter::new([reply("retry")]);
        assert_eq!(p.run(&router), Outcome::Finished("done".into()));
        assert_eq!(
            p.transitions(),
            ["- claimed verify claim", "verify pass done stdout"]
        );
        assert!(p.events_of("router").is_empty());
        assert_eq!(router.replies.borrow().len(), 1);
    }

    // ---------------------------------------------------------------
    // Other events
    // ---------------------------------------------------------------

    #[test]
    fn undeclared_printed_event_becomes_error_with_invalid_event() {
        let p = Project::new("step_normal", &[("a_invoke", "print_undeclared")]);
        assert_eq!(p.run(&no_router()), Outcome::Finished("failed".into()));
        let t = &p.events_of("transition")[1];
        assert_eq!(t["event"], "error");
        assert_eq!(t["source"], "stdout");
        assert_eq!(t["invalid_event"], "nope");
        assert_eq!(t["to"], "failed");
    }

    #[test]
    fn timed_out_invoke_gives_error() {
        let p = Project::new("step_timeout", &[("sleep_long", "sleep_long")]);
        assert_eq!(p.run(&no_router()), Outcome::Finished("failed".into()));
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
        assert_eq!(p.run(&no_router()), Outcome::Interrupted("a".into()));
        let events = p.events();
        let last = events.last().unwrap();
        assert_eq!(last["type"], "interrupted");
        assert_eq!(last["state"], "a");
        assert_eq!(last["cause"], "signal");
        assert_eq!(last["script"], "root_entry");
        assert!(p.order().is_empty());
        assert_eq!(
            run_status(&p.machine, &events, false),
            RunStatus::Interrupted
        );
    }

    // ---------------------------------------------------------------
    // Composition: bubbling, `type: internal`, nested final states (M3.4)
    // ---------------------------------------------------------------

    #[test]
    fn composition_unhandled_event_is_taken_by_the_nearest_ancestor() {
        let p = Project::new("step_bubble", &[("a_invoke", "print_pass")]);
        assert_eq!(p.run(&no_router()), Outcome::Finished("done".into()));
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
        assert_eq!(p.run(&no_router()), Outcome::Finished("done".into()));
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
        assert_eq!(p.run(&no_router()), Outcome::Finished("done".into()));
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
        assert_eq!(p.run(&no_router()), Outcome::Finished("done".into()));
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
            run_status(&p.machine, &events[..=at + 1], false),
            RunStatus::Interrupted
        );
        assert_eq!(visits(&events)["finished"], 1);
    }

    #[test]
    fn nested_final_state_writes_no_ledger_line() {
        let p = Project::new("step_nested_final", &[("after_invoke", "copy_ledger")]);
        fs::write(p.root().join(".decree/processed.md"), "45-prev.md\n").unwrap();
        let outcome = p.run_with(
            &no_router(),
            "migration",
            "46-next.md",
            &serde_norway::Mapping::new(),
        );
        assert_eq!(outcome, Outcome::Finished("done".into()));
        // `copy_ledger` ran after `finished` was entered, before the root `done`.
        let seen = fs::read_to_string(p.root().join("ledger.txt")).unwrap();
        assert_eq!(seen, "45-prev.md\n");
        assert_eq!(p.processed(), "45-prev.md\n46-next.md\n");
    }

    #[test]
    fn nested_final_onentry_failure_is_error_resolved_from_that_state() {
        let p = Project::new("step_nested_final", &[("finished_entry", "exit_three")]);
        assert_eq!(p.run(&no_router()), Outcome::Finished("recovered".into()));
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
    // Waiting states, interpreter side (M3.4)
    // ---------------------------------------------------------------

    /// The `step_waiting` project, run until it waits. `ask_person` prints its environment.
    fn waiting_project(scripts: &[(&str, &str)]) -> (Project, String) {
        let mut all = vec![("ask_person", "print_env")];
        all.extend_from_slice(scripts);
        let p = Project::new("step_waiting", &all);
        let outcome = p.run(&no_router());
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
        let router = no_router();
        let params = serde_norway::Mapping::new();
        let input = RunInput {
            params: &params,
            message_body: BODY,
            file: "inbox.md",
        };
        Interpreter::new(&p.machine, executor, &router, input)
            .unwrap()
            .resume()
            .unwrap()
    }

    #[test]
    fn waiting_state_runs_onentry_with_wait_env_appends_waiting_and_stops() {
        let before = Utc::now();
        let (p, wait_id) = waiting_project(&[]);
        assert_eq!(p.order(), ["root_entry", "build_invoke", "gate_entry"]);
        // `ask_person` sees the wait id and the accepted events, its ancestor's included.
        let ask = p
            .events_of("script")
            .into_iter()
            .find(|e| e["script"] == "ask_person")
            .unwrap();
        let log = fs::read_to_string(p.run_dir().join(ask["log"].as_str().unwrap())).unwrap();
        assert!(
            log.contains(&format!("DECREE_WAIT_ID={wait_id}\n")),
            "{log}"
        );
        assert!(
            log.contains("DECREE_ACCEPTS=approve cancel reject\n"),
            "{log}"
        );
        assert!(log.contains("DECREE_STATE=approval\n"), "{log}");
        assert!(log.contains("DECREE_PHASE=onentry\n"), "{log}");

        let events = p.events();
        let last = events.last().unwrap();
        assert_eq!(last["type"], "waiting");
        assert_eq!(last["state"], "approval");
        assert_eq!(last["wait_id"], json!(wait_id));
        assert_eq!(last["accepts"], json!(["approve", "cancel", "reject"]));
        let timeout_at = DateTime::parse_from_rfc3339(last["timeout_at"].as_str().unwrap())
            .unwrap()
            .with_timezone(&Utc);
        let ahead = (timeout_at - before).num_seconds();
        assert!((59..=61).contains(&ahead), "{ahead}");
        assert!(p.events_of("run_finished").is_empty());
        assert_eq!(run_status(&p.machine, &events, false), RunStatus::Waiting);
        assert_eq!(mirrored_state(&p), "approval");
    }

    #[test]
    fn waiting_state_without_timeout_has_null_timeout_at() {
        let text = fs::read_to_string(repo().join("tests/fixtures/machines/step/step_waiting.yml"))
            .unwrap()
            .replace("        timeout_s: 60\n", "");
        let m = load_machine_text("step_waiting", Path::new("step_waiting.yml"), &text).unwrap();
        let p = Project::new("step_waiting", &[]);
        let router = no_router();
        let params = serde_norway::Mapping::new();
        let input = RunInput {
            params: &params,
            message_body: BODY,
            file: "inbox.md",
        };
        let outcome = Interpreter::new(&m, p.executor("inbox", &params), &router, input)
            .unwrap()
            .start()
            .unwrap();
        assert!(matches!(outcome, Outcome::Waiting { .. }), "{outcome:?}");
        assert_eq!(p.events_of("waiting")[0]["timeout_at"], Value::Null);
    }

    #[test]
    fn waiting_received_event_continues_without_rerunning_any_script() {
        let (p, wait_id) = waiting_project(&[("ship_invoke", "print_env")]);
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
        let t = p.events_of("transition");
        let taken = t.iter().find(|e| e["from"] == "approval").unwrap();
        assert_eq!(taken["event"], "approve");
        assert_eq!(taken["to"], "ship");
        assert_eq!(taken["source"], "external");
        assert_eq!(taken["exit_code"], Value::Null);
        // Later scripts see the reply.
        let ship = scripts
            .iter()
            .find(|e| e["script"] == "ship_invoke")
            .unwrap();
        let log = fs::read_to_string(p.run_dir().join(ship["log"].as_str().unwrap())).unwrap();
        let reply = p.run_dir().join("received/reply.md");
        assert!(
            log.contains(&format!("DECREE_RECEIVED={}\n", reply.display())),
            "{log}"
        );
        assert!(log.contains("DECREE_WAIT_ID=\n"), "{log}");
        let events = p.events();
        assert_eq!(run_status(&p.machine, &events, false), RunStatus::Finished);
        // Log numbers continue after the logs written before the wait.
        let logs: Vec<&str> = scripts.iter().map(|e| e["log"].as_str().unwrap()).collect();
        let unique: HashSet<&&str> = logs.iter().collect();
        assert_eq!(unique.len(), logs.len(), "{logs:?}");
    }

    #[test]
    fn waiting_received_event_bubbles_to_an_ancestor() {
        let (p, wait_id) = waiting_project(&[]);
        let outcome = receive(&p, json!({ "wait_id": wait_id, "event": "cancel" }));
        assert_eq!(outcome, Outcome::Finished("cancelled".into()));
        assert!(p
            .transitions()
            .contains(&"approval cancel cancelled external".to_string()));
    }

    #[test]
    fn waiting_timeout_error_goes_to_failed() {
        let (p, wait_id) = waiting_project(&[]);
        let outcome = receive(
            &p,
            json!({ "wait_id": wait_id, "event": "error", "timed_out": true }),
        );
        assert_eq!(outcome, Outcome::Finished("failed".into()));
        assert!(p
            .transitions()
            .contains(&"approval error failed external".to_string()));
        assert_eq!(
            &p.order()[3..],
            ["approval_exit", "gate_exit", "failed_entry", "root_exit"]
        );
    }

    #[test]
    fn waiting_state_onentry_failure_is_error_and_does_not_wait() {
        let p = Project::new("step_waiting", &[("ask_person", "exit_three")]);
        assert_eq!(p.run(&no_router()), Outcome::Finished("failed".into()));
        assert!(p.events_of("waiting").is_empty());
        assert!(p
            .transitions()
            .contains(&"approval error failed exit_code".to_string()));
    }

    #[test]
    fn resume_refuses_a_run_that_has_not_received_an_event() {
        let (p, _) = waiting_project(&[]);
        let router = no_router();
        let params = serde_norway::Mapping::new();
        let input = RunInput {
            params: &params,
            message_body: BODY,
            file: "inbox.md",
        };
        let err = Interpreter::new(&p.machine, p.executor("inbox", &params), &router, input)
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
    fn every_waiting_and_received_field_appears() {
        let (p, wait_id) = waiting_project(&[]);
        receive(
            &p,
            json!({ "wait_id": wait_id, "event": "approve", "file": "reply.md" }),
        );
        let (q, wait_id) = waiting_project(&[]);
        receive(
            &q,
            json!({ "wait_id": wait_id, "event": "error", "timed_out": true }),
        );
        let common = ["v", "seq", "ts", "type", "run_id", "machine", "trigger"];
        let keys = |kind: &str| -> HashSet<String> {
            p.events_of(kind)
                .into_iter()
                .chain(q.events_of(kind))
                .flat_map(|e| e.keys().cloned().collect::<Vec<_>>())
                .collect()
        };
        for (kind, fields) in [
            (
                "waiting",
                &["state", "wait_id", "accepts", "timeout_at"][..],
            ),
            ("received", &["wait_id", "event", "file", "timed_out"][..]),
        ] {
            let want: HashSet<String> =
                common.iter().chain(fields).map(|s| s.to_string()).collect();
            assert_eq!(keys(kind), want, "{kind}");
        }
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
            let router = no_router();
            let params = serde_norway::Mapping::new();
            let input = RunInput {
                params: &params,
                message_body: "",
                file: "irp.md",
            };
            let outcome = Interpreter::new(&m, p.executor("inbox", &params), &router, input)
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
    fn every_transition_script_router_and_run_finished_field_appears() {
        let mut seen: BTreeMap<String, HashSet<String>> = BTreeMap::new();
        let mut collect = |p: &Project| {
            for e in p.events() {
                let kind = e["type"].as_str().unwrap().to_string();
                seen.entry(kind).or_default().extend(e.keys().cloned());
            }
        };

        let p = Project::new("step_exit_fail", &[]);
        p.run(&no_router());
        collect(&p);
        let p = Project::new("step_normal", &[("a_invoke", "print_undeclared")]);
        p.run(&no_router());
        collect(&p);
        let p = Project::new("step_timeout", &[("sleep_long", "sleep_long")]);
        p.run(&no_router());
        collect(&p);
        let p = router_project();
        p.run(&ScriptedRouter::new([Ok(RouterReply {
            event: "pass".into(),
            reason: Some("ok".into()),
            confidence: Some(0.8),
            probabilities: Some(BTreeMap::from([("pass".to_string(), 0.8)])),
        })]));
        collect(&p);
        let p = router_project();
        p.run(&no_router());
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
        let expected: [(&str, &[&str]); 4] = [
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
            (
                "router",
                &[
                    "state",
                    "options",
                    "event",
                    "source",
                    "reason",
                    "confidence",
                    "probabilities",
                    "router_error",
                    "asks",
                    "duration_ms",
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
        assert_eq!(run_status(&p.machine, &events, false), RunStatus::Finished);
        assert!(p.order().is_empty());
    }

    // ---------------------------------------------------------------
    // Run status (section 4)
    // ---------------------------------------------------------------

    #[test]
    fn run_status_follows_the_section_4_order() {
        let p = Project::new("step_normal", &[]);
        let m = &p.machine;
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
