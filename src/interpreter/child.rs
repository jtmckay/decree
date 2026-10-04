//! Child runs (docs/reference/runs.md, Sub-machines): a `machine` invoke, and the router run
//! of a `model` invoke. The parent appends `waiting`, the same process steps the
//! child, and a finished child continues its parent.

use std::time::Instant;

use serde_json::json;

use super::decide::REQUEST_FILE;
use super::{
    write_replace, Context, Decision, Interpreter, InterpreterError, Invoked, Outcome, RunInput,
};
use crate::events::{claim_event, is_type, text, waiting_child};
use crate::layout::DECREE_DIR;
use crate::layout::MESSAGE_FILE;
use crate::machine::{MachineInvoke, FAILED};
use crate::message::{create_run_dir, Message, MAX_DEPTH};
use crate::trace::{self, TraceParent, TRACEPARENT_KEY, TRACESTATE_KEY};

/// A child run this run started, and how stepping it ended.
pub(super) struct Child {
    pub(super) id: String,
    pub(super) outcome: Outcome,
    /// Wall time of stepping the child.
    pub(super) duration_ms: u64,
}

/// Continue `pending` run `run_id` from its folder (docs/reference/runs.md, step 1): its last event is
/// `received`, or `waiting` for a child run that has finished. A child run that finishes
/// continues its parent, if the parent waits for it, and so on up: the result is that of
/// the last run continued.
pub fn continue_run(ctx: &Context, run_id: &str) -> Result<Outcome, InterpreterError> {
    let message_path = ctx.runs_dir().join(run_id).join(MESSAGE_FILE);
    let message = Message::read(&message_path)?;
    let name = message.machine().ok_or_else(|| InterpreterError::Message {
        path: message_path.clone(),
        message: "frontmatter names no `machine`".to_string(),
    })?;
    let machine = ctx
        .machines
        .get(name)
        .ok_or_else(|| InterpreterError::Invalid {
            machine: name.to_string(),
            message: "no such machine".to_string(),
        })?;
    let trigger = message.text("trigger").unwrap_or("inbox");
    let parent = message.text("parent");
    let file = claim_event(&ctx.events(run_id)?)
        .and_then(|e| text(e, "file"))
        .map(String::from);
    let input = RunInput::new(&message, file);
    let executor = ctx.executor(machine, run_id, trigger, &input.params, parent)?;
    let outcome = Interpreter::new(ctx, machine, executor, input)?.resume()?;
    let Some(parent) = parent.filter(|_| trigger == "invoke") else {
        return Ok(outcome);
    };
    if !matches!(outcome, Outcome::Finished(_)) {
        return Ok(outcome);
    }
    if waiting_child(&ctx.events(parent)?) == Some(run_id) {
        continue_run(ctx, parent)
    } else {
        Ok(outcome)
    }
}

impl Interpreter<'_> {
    /// Step 2 for a `machine` invoke (docs/reference/runs.md, Sub-machines): run the machine as a child
    /// run. Its final state is the event, `failed` as `error`.
    pub(super) fn invoke_machine(
        &mut self,
        s: usize,
        invoke: &MachineInvoke,
    ) -> Result<Invoked, InterpreterError> {
        let child = match self.run_child(s, &invoke.name, &invoke.params, None)? {
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
    /// The parent's side of a child that stopped before finishing: the run stays `waiting`.
    pub(super) fn wait_for_child(&self, s: usize, child: String, outcome: Outcome) -> Invoked {
        Invoked::Wait(Outcome::Child {
            state: self.machine.nodes[s].id.clone(),
            child,
            outcome: Box::new(outcome),
        })
    }
    /// A `machine` invoke's child reached root final state `state`: append the `received`
    /// event, whose event is that state (`failed` as `error`).
    pub(super) fn machine_finished(
        &mut self,
        child: &str,
        state: &str,
    ) -> Result<Decision, InterpreterError> {
        let event = if state == FAILED { "error" } else { state };
        let mut fields = json!({ "event": event, "child": child });
        if let Some(span) = self.child_span(child)? {
            fields["span_id"] = json!(span);
        }
        self.append("received", fields)?;
        Ok(Decision::new(event, "machine", None))
    }
    /// Start machine `name` as a child run of state `s` and step it until it finishes, waits
    /// or is interrupted (docs/reference/runs.md, Sub-machines). The child's `message.md` holds `machine`,
    /// `id`, `parent`, `depth`, `trigger: invoke`, any `params` and this run's body; a
    /// `request` is written to its `request.json`, which makes it a router run. Appends the
    /// `waiting` event naming the child first. `Err` holds why no child started: its
    /// `depth` would exceed `max_depth`.
    pub(super) fn run_child(
        &mut self,
        s: usize,
        name: &str,
        params: &serde_norway::Mapping,
        request: Option<&str>,
    ) -> Result<Result<Child, String>, InterpreterError> {
        let ctx = self.ctx;
        let machine = ctx
            .machines
            .get(name)
            .ok_or_else(|| self.invalid(format!("machine `{name}` does not exist")))?;
        let depth = self.depth + 1;
        if depth > MAX_DEPTH {
            return Ok(Err(format!("max_depth {MAX_DEPTH} reached")));
        }
        let (id, run_dir) = create_run_dir(&ctx.project_root.join(DECREE_DIR))?;
        let parent = self.executor.info().run_id.clone();
        let mut message = Message::new(self.message_body.as_str());
        message.set("machine", name);
        message.set("id", id.as_str());
        message.set("parent", parent.as_str());
        message.set("depth", depth);
        message.set("trigger", "invoke");
        if !params.is_empty() {
            message.set("params", params.clone());
        }
        // The child's run span is a child of this state's span: the router's `decision`, or
        // the `machine` invoke's wait (docs/reference/observability.md, Traces). Its claim
        // event records that span as `parent_span_id`, where `child_span` finds it.
        let events = self.executor.events();
        let traceparent = TraceParent::format(events.trace_id(), &trace::new_span_id());
        let tracestate = events.tracestate().map(String::from);
        message.set(TRACEPARENT_KEY, traceparent);
        if let Some(tracestate) = tracestate {
            message.set(TRACESTATE_KEY, tracestate);
        }
        message.write(&run_dir.join(MESSAGE_FILE))?;
        if let Some(request) = request {
            write_replace(&run_dir.join(REQUEST_FILE), request.as_bytes())?;
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
    /// The span child run `child` was started under: the `parent_span_id` of its claim
    /// event, set from the `traceparent` `run_child` wrote. The `decision` (router) or the
    /// wait (`machine` invoke) that ends takes it as its `span_id`.
    pub(super) fn child_span(&self, child: &str) -> Result<Option<String>, InterpreterError> {
        Ok(claim_event(&self.ctx.events(child)?)
            .and_then(|e| text(e, "parent_span_id"))
            .map(String::from))
    }
    /// How long finished child run `child` took: its `run_finished` event's `duration_ms`.
    pub(super) fn child_duration(&self, child: &str) -> Result<u64, InterpreterError> {
        Ok(self
            .ctx
            .events(child)?
            .iter()
            .rev()
            .find(|e| is_type(e, "run_finished"))
            .and_then(|e| e.get("duration_ms"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0))
    }
}
