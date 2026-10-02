//! The router seam (spec section 7, Router): what a router state asks and what comes back.
//!
//! The contract between decree and a real backend is being decided by the spike in
//! `docs/spikes/router.md` (tickets M3.2 and M3.3). Only the structured seam lives here,
//! plus `ScriptedRouter`, which tests use so that no test calls a model.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};

use serde::Serialize;

/// One event a router state's router may pick, with its transition's `description`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RouterOption {
    pub event: String,
    pub description: String,
}

/// Everything a router sees for one decision.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RouterRequest {
    pub machine: String,
    pub machine_description: String,
    pub state: String,
    pub state_description: String,
    /// `transitions` key order, after `cond`s.
    pub options: Vec<RouterOption>,
    /// Last 50 stdout lines of the invoke.
    pub step_output: String,
    pub message_body: String,
    /// Set on the second ask: why the first was rejected.
    pub previous_error: Option<String>,
}

/// A router's answer. decree accepts it only if `event` is one of the options.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RouterReply {
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// 0..1, if the backend reports it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// Per option, if the backend reports it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probabilities: Option<BTreeMap<String, f64>>,
}

impl RouterReply {
    /// A reply naming `event`, with nothing else.
    pub fn event(event: &str) -> Self {
        RouterReply {
            event: event.to_string(),
            reason: None,
            confidence: None,
            probabilities: None,
        }
    }
}

/// A router that gave no reply: the backend failed, or had nothing to say.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{0}")]
pub struct RouterError(pub String);

pub trait Router {
    fn decide(&self, req: &RouterRequest) -> Result<RouterReply, RouterError>;
}

/// A router that returns queued replies in order, for tests. An empty queue is a failure.
#[derive(Debug, Default)]
pub struct ScriptedRouter {
    pub replies: RefCell<VecDeque<Result<RouterReply, RouterError>>>,
}

impl ScriptedRouter {
    pub fn new(replies: impl IntoIterator<Item = Result<RouterReply, RouterError>>) -> Self {
        ScriptedRouter {
            replies: RefCell::new(replies.into_iter().collect()),
        }
    }
}

impl Router for ScriptedRouter {
    fn decide(&self, _req: &RouterRequest) -> Result<RouterReply, RouterError> {
        self.replies
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Err(RouterError("no scripted reply left".to_string())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> RouterRequest {
        RouterRequest {
            machine: "m".into(),
            machine_description: "A machine.".into(),
            state: "s".into(),
            state_description: "A state.".into(),
            options: vec![RouterOption {
                event: "pass".into(),
                description: "Passed.".into(),
            }],
            step_output: String::new(),
            message_body: String::new(),
            previous_error: None,
        }
    }

    #[test]
    fn scripted_router_returns_replies_in_order_then_fails() {
        let router =
            ScriptedRouter::new([Ok(RouterReply::event("a")), Err(RouterError("down".into()))]);
        assert_eq!(router.decide(&request()).unwrap().event, "a");
        assert_eq!(router.decide(&request()).unwrap_err().0, "down");
        assert_eq!(
            router.decide(&request()).unwrap_err().0,
            "no scripted reply left"
        );
    }

    #[test]
    fn reply_serializes_only_reported_fields() {
        let json = serde_json::to_string(&RouterReply::event("pass")).unwrap();
        assert_eq!(json, r#"{"event":"pass"}"#);
    }
}
