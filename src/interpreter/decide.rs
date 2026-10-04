//! The decision invokes (docs/reference/runs.md, Check, Model, Person): a `check` evaluates its
//! condition; a `model` writes a request, runs the router machine as a child run and validates
//! its reply; a `person` runs its `ask` script and waits.

use std::collections::BTreeMap;
use std::fs;
use std::io;

use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};

use super::{io_err, write_replace, Decision, Interpreter, InterpreterError, Invoked, Outcome};
use crate::cond;
use crate::cond::Condition;
use crate::events::{confidences, is_transition, is_type, text, timestamp, visits, Event};
use crate::layout::RECEIVED_DIR;
use crate::machine::{LoadedMachine, ModelInvoke, PersonInvoke, FAILED, ROUTER_MACHINE};
use crate::runtime::{Phase, ScriptRun};

/// The JSON file, in the run folder, mapping each option of the `person` state the
/// run waits in to its description: what `DECREE_CHOICES` names (docs/reference/scripts.md).
pub const CHOICES_FILE: &str = "choices.json";

/// The request a `model` invoke writes in its router run's folder (docs/reference/runs.md).
pub const REQUEST_FILE: &str = "request.json";

/// Where a router machine writes its reply, in its run folder (docs/reference/runs.md).
pub const REPLY_FILE: &str = "reply.json";

/// `request.json` (docs/reference/runs.md, Model, step 1). Field order is the reference's.
#[derive(serde::Serialize)]
pub(super) struct Request<'r> {
    v: u32,
    machine: &'r str,
    machine_description: &'r str,
    state: &'r str,
    state_description: &'r str,
    question: &'r str,
    options: Vec<RequestOption<'r>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_confidence: Option<f64>,
    input: String,
    message_body: &'r str,
    history: Vec<String>,
    reply_schema: Value,
}

#[derive(serde::Serialize)]
pub(super) struct RequestOption<'r> {
    event: &'r str,
    description: &'r str,
}

/// A router's reply (docs/reference/runs.md, Model, steps 3 and 4).
#[derive(Debug, PartialEq)]
pub(super) enum Reply {
    /// `reply.json` names one of the options.
    Pick {
        event: String,
        reason: Option<String>,
        confidence: Option<f64>,
        probabilities: Option<Map<String, Value>>,
    },
    /// No valid reply: the `router_error`.
    Rejected(String),
}

impl Reply {
    /// Validate `reply.json` against the state's options. Only `event` is required, and it
    /// must name an option exactly. `reason`, `confidence` (0 to 1) and `probabilities`
    /// (option to number) must have their types when present; `null` counts as absent.
    pub(super) fn parse(bytes: &[u8], options: &[String]) -> Result<Reply, String> {
        let bad = |what: &str| format!("{REPLY_FILE}: {what}");
        let value: Value =
            serde_json::from_slice(bytes).map_err(|e| bad(&format!("not JSON: {e}")))?;
        let Value::Object(mut reply) = value else {
            return Err(bad("not a JSON object"));
        };
        let mut take = |key: &str| reply.remove(key).filter(|v| !v.is_null());
        let event = match take("event") {
            Some(Value::String(event)) => event,
            Some(_) => return Err(bad("`event` is not a string")),
            None => return Err(bad("no `event`")),
        };
        if !options.contains(&event) {
            return Err(bad(&format!(
                "`{event}` is not one of the options: {}",
                options.join(", ")
            )));
        }
        let reason = match take("reason") {
            Some(Value::String(reason)) => Some(reason),
            Some(_) => return Err(bad("`reason` is not a string")),
            None => None,
        };
        let confidence = match take("confidence") {
            Some(v) => match v.as_f64().filter(|c| (0.0..=1.0).contains(c)) {
                Some(c) => Some(c),
                None => {
                    return Err(bad(&format!(
                        "`confidence` {v} is not a number from 0 to 1"
                    )))
                }
            },
            None => None,
        };
        let probabilities = match take("probabilities") {
            Some(Value::Object(p)) if p.values().all(Value::is_number) => Some(p),
            Some(_) => return Err(bad("`probabilities` is not a map of option to number")),
            None => None,
        };
        Ok(Reply::Pick {
            event,
            reason,
            confidence,
            probabilities,
        })
    }
}

/// `reply_schema` in `request.json` (docs/reference/runs.md, Model, step 1): a JSON Schema
/// (draft 2020-12) for `reply.json` over `options`, which a typed router passes unchanged
/// to a constrained decoder (Ollama's `format`, OpenAI-style `response_format`).
pub(super) fn reply_schema(options: &[String]) -> Value {
    let probabilities: Map<String, Value> = options
        .iter()
        .map(|o| (o.clone(), json!({ "type": "number" })))
        .collect();
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "event": { "enum": options },
            "confidence": { "type": "number", "minimum": 0, "maximum": 1 },
            "reason": { "type": "string" },
            "probabilities": {
                "type": "object",
                "properties": probabilities,
                "additionalProperties": false,
            },
        },
        "required": ["event"],
        "additionalProperties": false,
    })
}

/// The options of `model` or `person` state `n`, in name order (docs/reference/machines.md, Choices).
pub(super) fn option_names(m: &LoadedMachine, n: usize) -> Vec<String> {
    m.options(n).map(|e| e.event.clone()).collect()
}

impl Interpreter<'_> {
    /// Step 2 for a `person` state (docs/reference/messages.md, Replies): write its options to
    /// `choices.json`, run its `ask` script with the wait id, then append `waiting` and stop.
    /// An `ask` script that exits non-zero gives `error` instead, since nobody was told.
    pub(super) fn ask(
        &mut self,
        s: usize,
        person: &PersonInvoke,
    ) -> Result<Invoked, InterpreterError> {
        let m = self.machine;
        let node = &m.nodes[s];
        let ask = person
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
            question: person.question.as_deref().unwrap_or_default(),
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

        let timeout_at = person.timeout_s.map(|secs| {
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
    pub(super) fn wait_id(&self) -> String {
        format!("{}.w{}", self.executor.info().run_id, self.entered_seq)
    }
    /// Step 2 for a `model` invoke (docs/reference/runs.md, Model): write the request,
    /// run the router machine as a child run, and validate its reply.
    pub(super) fn ask_model(
        &mut self,
        s: usize,
        model: &ModelInvoke,
    ) -> Result<Invoked, InterpreterError> {
        let request = self.request(s, model)?;
        let router = Self::router(model);
        let params = serde_norway::Mapping::new();
        let child = match self.run_child(s, &router, &params, Some(&request))? {
            Ok(child) => child,
            Err(reason) => {
                let reply = Reply::Rejected(reason);
                return self
                    .model_decision(s, model, &router, None, reply, 0)
                    .map(Invoked::Event);
            }
        };
        match child.outcome {
            Outcome::Finished(state) => self
                .model_finished(s, model, &child.id, &state, child.duration_ms)
                .map(Invoked::Event),
            outcome => Ok(self.wait_for_child(s, child.id, outcome)),
        }
    }
    /// The router machine of `model` state `s`: its `router`, else the machine named
    /// [`ROUTER_MACHINE`].
    pub(super) fn router(model: &ModelInvoke) -> String {
        model
            .router
            .clone()
            .unwrap_or_else(|| ROUTER_MACHINE.to_string())
    }
    /// docs/reference/runs.md, Model, step 1: the request for state `s`, as the text of
    /// `request.json`, its keys in docs/reference/runs.md's order.
    pub(super) fn request(
        &self,
        s: usize,
        model: &ModelInvoke,
    ) -> Result<String, InterpreterError> {
        let m = self.machine;
        let node = &m.nodes[s];
        let events = self.read_events()?;
        let history = events
            .iter()
            .filter(|e| is_transition(e))
            .filter(|e| text(e, "source") != Some("claim"))
            .map(|e| {
                let field = |key: &str| text(e, key).unwrap_or_default();
                format!("{}: {}", field("from"), field("event"))
            })
            .collect();
        let request = Request {
            v: 1,
            machine: &m.id,
            machine_description: m.description(),
            state: &node.id,
            state_description: node.description.as_deref().unwrap_or_default(),
            question: model.question.as_deref().unwrap_or_default(),
            options: m
                .options(s)
                .map(|e| RequestOption {
                    event: &e.event,
                    description: e.description.as_deref().unwrap_or_default(),
                })
                .collect(),
            min_confidence: model.min_confidence,
            input: match model.output.as_deref() {
                Some(state) => self.output_text(&events, state)?,
                None => String::new(),
            },
            message_body: &self.message_body,
            history,
            reply_schema: reply_schema(&option_names(m, s)),
        };
        Ok(serde_json::to_string_pretty(&request).unwrap_or_default() + "\n")
    }
    /// A router run finished in `state`: validate its reply and append the `decision` event.
    pub(super) fn model_finished(
        &mut self,
        s: usize,
        model: &ModelInvoke,
        child: &str,
        state: &str,
        duration_ms: u64,
    ) -> Result<Decision, InterpreterError> {
        let router = Self::router(model);
        let reply = if state == FAILED {
            Reply::Rejected(format!("router run `{child}` ended in `{FAILED}`"))
        } else {
            let path = self.ctx.runs_dir().join(child).join(REPLY_FILE);
            match fs::read(&path) {
                Ok(bytes) => {
                    let options = option_names(self.machine, s);
                    Reply::parse(&bytes, &options).unwrap_or_else(Reply::Rejected)
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    Reply::Rejected(format!("router run `{child}` wrote no {REPLY_FILE}"))
                }
                Err(e) => return Err(io_err(&path)(e)),
            }
        };
        self.model_decision(s, model, &router, Some(child), reply, duration_ms)
    }
    /// docs/reference/runs.md, Model, step 4: the event from a validated `reply`, which is
    /// `unsure` when `min_confidence` is set and the confidence is missing or lower, and
    /// the `decision` event that records it.
    pub(super) fn model_decision(
        &mut self,
        s: usize,
        model: &ModelInvoke,
        router: &str,
        child: Option<&str>,
        reply: Reply,
        duration_ms: u64,
    ) -> Result<Decision, InterpreterError> {
        let mut fields = json!({
            "state": self.machine.nodes[s].id,
            "kind": "model",
            "options": option_names(self.machine, s),
            "router": router,
        });
        if let Some(child) = child {
            fields["child_run"] = json!(child);
        }
        let event = match reply {
            Reply::Rejected(reason) => {
                fields["router_error"] = json!(reason);
                "error".to_string()
            }
            Reply::Pick {
                event,
                reason,
                confidence,
                probabilities,
            } => {
                fields["pick"] = json!(event);
                if let Some(reason) = reason {
                    fields["reason"] = json!(reason);
                }
                if let Some(confidence) = confidence {
                    fields["confidence"] = json!(confidence);
                }
                if let Some(probabilities) = probabilities {
                    fields["probabilities"] = Value::Object(probabilities);
                }
                let sure = model
                    .min_confidence
                    .is_none_or(|min| confidence.is_some_and(|c| c >= min));
                if sure {
                    event
                } else {
                    "unsure".to_string()
                }
            }
        };
        fields["event"] = json!(event);
        fields["duration_ms"] = json!(duration_ms);
        self.append("decision", fields)?;
        Ok(Decision::new(&event, "model", None))
    }
    /// docs/reference/runs.md, Check: evaluate the condition against an `output` state's output,
    /// `data`, visits and confidences, append a `decision` event, and produce `true` or `false`.
    /// No script runs.
    pub(super) fn check(
        &mut self,
        s: usize,
        check: &Condition,
    ) -> Result<Decision, InterpreterError> {
        let m = self.machine;
        let events = self.read_events()?;
        let output = match check.output.as_deref() {
            Some(state) => self.output_text(&events, state)?,
            None => String::new(),
        };
        let result = check
            .eval(&cond::Facts {
                data: &self.data,
                visits: &visits(&events),
                confidence: &confidences(&events),
                output: &output,
            })
            .map_err(|source| InterpreterError::Check {
                machine: m.id.clone(),
                at: m.state_path(s),
                source,
            })?;
        let event = if result { "true" } else { "false" };
        self.append(
            "decision",
            json!({
                "state": m.nodes[s].id,
                "kind": "check",
                "event": event,
                "condition": check,
            }),
        )?;
        Ok(Decision::new(event, "check", None))
    }
    /// docs/reference/machines.md, Output: the log of the latest invoke script of `state`. Empty
    /// if it has not run.
    pub(super) fn output_text(
        &self,
        events: &[Event],
        state: &str,
    ) -> Result<String, InterpreterError> {
        let log = events
            .iter()
            .rev()
            .filter(|e| is_type(e, "script"))
            .filter(|e| text(e, "phase") == Some(Phase::Invoke.as_str()))
            .find(|e| text(e, "state") == Some(state))
            .and_then(|e| text(e, "log"));
        let Some(log) = log else {
            return Ok(String::new());
        };
        let path = self.executor.info().run_dir.join(log);
        let bytes = fs::read(&path).map_err(io_err(&path))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// docs/reference/messages.md, Replies: a reply or a timeout was delivered to
    /// `person` state `s` as `event`, the last of `events`. A reply appends the
    /// `decision` event and is `DECREE_RECEIVED` for the scripts that follow.
    pub(super) fn person_received(
        &mut self,
        s: usize,
        events: &[Event],
        event: &str,
    ) -> Result<Decision, InterpreterError> {
        let m = self.machine;
        let Some(last) = events.last() else {
            return Err(InterpreterError::NotReceived(
                "it has no events".to_string(),
            ));
        };
        let timed_out = last.get("timed_out").and_then(Value::as_bool) == Some(true);
        let reply = text(last, "file").map(String::from);
        // `DECREE_RECEIVED`: the last reply, not a timeout, which has no file.
        let run_dir = self.executor.info().run_dir.clone();
        self.executor.received = events
            .iter()
            .rev()
            .filter(|e| is_type(e, "received"))
            .find_map(|e| text(e, "file"))
            .map(|file| run_dir.join(RECEIVED_DIR).join(file));
        if timed_out {
            return Ok(Decision::new(event, "timeout", None));
        }
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
        Ok(Decision::new(event, "person", None))
    }
}
