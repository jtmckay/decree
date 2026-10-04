//! Machines: SCXML statecharts written as YAML (docs/reference/machines.md).
//!
//! `load_machines` reads `.decree/machines/*.yml` and flattens each machine into an arena of `Node`s. The interpreter, validator and graph
//! exporter work on the arena, never on the raw structs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

use crate::cond::{Condition, Operand, Subject, Test};
use crate::error::DecreeError;
use crate::runtime::{is_reserved_event, resolve_script};

/// Directory holding machine files, relative to `.decree/`.
pub const MACHINES_DIR: &str = "machines";

/// Root-level final state an unhandled `error` goes to (V5).
pub const FAILED: &str = "failed";

/// Events a `choose` state handles beside its options (docs/reference/machines.md, Choices).
const NOT_OPTIONS: [&str; 2] = ["unsure", "error"];

/// A machine file: one SCXML document (`<scxml>`), written as YAML.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Machine {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub data: BTreeMap<String, DataSpec>,
    #[serde(default)]
    pub onentry: Vec<String>,
    #[serde(default)]
    pub onexit: Vec<String>,
    pub initial: String,
    pub states: BTreeMap<String, State>,
}

/// One `data` entry: SCXML `<data id>` with a type and a required default.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataSpec {
    #[serde(rename = "type")]
    pub kind: DataType,
    pub default: serde_norway::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataType {
    String,
    Int,
    Bool,
}

/// An SCXML `<state>` or `<final>`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    #[serde(default, rename = "final")]
    pub is_final: bool,
    pub description: Option<String>,
    pub invoke: Option<Invoke>,
    pub max_attempts: Option<u32>,
    pub timeout_s: Option<u64>,
    #[serde(default)]
    pub onentry: Vec<String>,
    #[serde(default)]
    pub onexit: Vec<String>,
    pub initial: Option<String>,
    #[serde(default)]
    pub states: BTreeMap<String, State>,
    #[serde(default)]
    pub transitions: BTreeMap<String, Transition>,
    #[serde(default)]
    pub emits: Vec<String>,
}

/// A state's function, SCXML `<invoke type>` (docs/reference/machines.md, Invoke): a script name, or an
/// object whose key `machine`, `check` or `choose` names the type.
#[derive(Debug, Clone, PartialEq)]
pub enum Invoke {
    /// `decree:script`
    Script(String),
    /// SCXML's own type: a child state machine.
    Machine(MachineInvoke),
    /// `decree:check`
    Check(CheckInvoke),
    /// `decree:model` or `decree:person`
    Choose(ChooseInvoke),
}

/// `{ machine, params? }`
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineInvoke {
    pub machine: String,
    #[serde(default)]
    pub params: serde_norway::Mapping,
}

/// `{ check, input? }`
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckInvoke {
    pub check: Condition,
    pub input: Option<String>,
}

/// `{ choose: model | person, question, router?, min_confidence?, input?, ask?, timeout_s? }`.
/// `question` is optional here so that V8, not the parser, reports it missing.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChooseInvoke {
    pub choose: ChooseKind,
    pub question: Option<String>,
    pub router: Option<String>,
    pub min_confidence: Option<f64>,
    pub input: Option<String>,
    pub ask: Option<String>,
    pub timeout_s: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChooseKind {
    Model,
    Person,
}

impl<'de> Deserialize<'de> for Invoke {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_norway::Value::deserialize(deserializer)?;
        let has = |key: &str| value.get(key).is_some();
        let result =
            match &value {
                serde_norway::Value::String(name) => return Ok(Invoke::Script(name.clone())),
                serde_norway::Value::Mapping(_) if has("machine") => {
                    serde_norway::from_value(value).map(Invoke::Machine)
                }
                serde_norway::Value::Mapping(_) if has("check") => {
                    serde_norway::from_value(value).map(Invoke::Check)
                }
                serde_norway::Value::Mapping(_) if has("choose") => {
                    serde_norway::from_value(value).map(Invoke::Choose)
                }
                _ => return Err(D::Error::custom(
                    "`invoke` is a script name or an object with `machine`, `check` or `choose`",
                )),
            };
        result.map_err(D::Error::custom)
    }
}

impl Invoke {
    /// The script an invoke runs, if it is a script invoke.
    pub fn script(&self) -> Option<&str> {
        match self {
            Invoke::Script(name) => Some(name),
            _ => None,
        }
    }

    /// The state whose output a `check` or `choose: model` reads (docs/reference/machines.md, Input).
    pub fn input(&self) -> Option<&str> {
        match self {
            Invoke::Check(c) => c.input.as_deref(),
            Invoke::Choose(c) => c.input.as_deref(),
            _ => None,
        }
    }

    /// The `choose` invoke, if this is one of `kind`.
    pub fn choose(&self, kind: ChooseKind) -> Option<&ChooseInvoke> {
        match self {
            Invoke::Choose(c) if c.choose == kind => Some(c),
            _ => None,
        }
    }
}

/// An SCXML `<transition event target type>`: `event: target` or the long form.
#[derive(Debug)]
pub enum Transition {
    Short(String),
    Long(LongTransition),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LongTransition {
    pub target: String,
    pub description: Option<String>,
    /// Only `internal`; omitted means external.
    #[serde(rename = "type")]
    pub kind: Option<TransitionType>,
}

impl<'de> Deserialize<'de> for Transition {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match serde_norway::Value::deserialize(deserializer)? {
            serde_norway::Value::String(target) => Ok(Transition::Short(target)),
            value @ serde_norway::Value::Mapping(_) => serde_norway::from_value(value)
                .map(Transition::Long)
                .map_err(D::Error::custom),
            _ => Err(D::Error::custom(
                "a transition is a target state or `{ target, description?, type? }`",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransitionType {
    Internal,
}

/// A transition in the arena, always in long form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub event: String,
    pub target: String,
    pub description: Option<String>,
    pub internal: bool,
}

impl Edge {
    fn new(event: String, transition: Transition) -> Self {
        match transition {
            Transition::Short(target) => Edge {
                event,
                target,
                description: None,
                internal: false,
            },
            Transition::Long(long) => Edge {
                event,
                target: long.target,
                description: long.description,
                internal: long.kind == Some(TransitionType::Internal),
            },
        }
    }
}

/// One state in a machine's arena. Index 0 is the root (`<scxml>`), whose `id` is the
/// machine name; the other nodes follow in depth-first order, children in `BTreeMap` order.
/// Names are kept as written: the validator, not the loader, checks that they resolve.
#[derive(Debug, Clone)]
pub struct Node {
    pub id: String,
    pub parent: Option<usize>,
    pub depth: usize,
    pub children: Vec<usize>,
    pub is_final: bool,
    pub description: Option<String>,
    pub invoke: Option<Invoke>,
    pub max_attempts: Option<u32>,
    pub timeout_s: Option<u64>,
    pub onentry: Vec<String>,
    pub onexit: Vec<String>,
    pub initial: Option<String>,
    /// In event-name order.
    pub transitions: Vec<Edge>,
    pub emits: Vec<String>,
}

/// A loaded machine: its `data` and its arena.
#[derive(Debug)]
pub struct LoadedMachine {
    /// The file stem, which V1 requires to equal `name`.
    pub id: String,
    pub data: BTreeMap<String, DataSpec>,
    pub nodes: Vec<Node>,
}

impl LoadedMachine {
    pub fn root(&self) -> &Node {
        &self.nodes[0]
    }

    pub fn description(&self) -> &str {
        self.root().description.as_deref().unwrap_or_default()
    }

    /// Index of the state with this id. The root is not a state and is never returned.
    pub fn find(&self, id: &str) -> Option<usize> {
        self.nodes
            .iter()
            .skip(1)
            .position(|n| n.id == id)
            .map(|i| i + 1)
    }

    /// Dotted ids from the outermost state down to `index`, e.g. `work.implement`.
    pub fn state_path(&self, index: usize) -> String {
        let mut ids = Vec::new();
        let mut cur = Some(index);
        while let Some(i) = cur.filter(|&i| i != 0) {
            ids.push(self.nodes[i].id.as_str());
            cur = self.nodes[i].parent;
        }
        ids.reverse();
        ids.join(".")
    }
}

/// Flatten a parsed machine into its arena.
pub fn flatten(id: &str, machine: Machine) -> LoadedMachine {
    let root = Node {
        id: machine.name,
        parent: None,
        depth: 0,
        children: Vec::new(),
        is_final: false,
        description: Some(machine.description),
        invoke: None,
        max_attempts: None,
        timeout_s: None,
        onentry: machine.onentry,
        onexit: machine.onexit,
        initial: Some(machine.initial),
        transitions: Vec::new(),
        emits: Vec::new(),
    };
    let mut nodes = vec![root];
    push_children(&mut nodes, 0, machine.states);
    LoadedMachine {
        id: id.to_string(),
        data: machine.data,
        nodes,
    }
}

fn push_children(nodes: &mut Vec<Node>, parent: usize, states: BTreeMap<String, State>) {
    let depth = nodes[parent].depth + 1;
    for (id, state) in states {
        let index = nodes.len();
        nodes[parent].children.push(index);
        nodes.push(Node {
            id,
            parent: Some(parent),
            depth,
            children: Vec::new(),
            is_final: state.is_final,
            description: state.description,
            invoke: state.invoke,
            max_attempts: state.max_attempts,
            timeout_s: state.timeout_s,
            onentry: state.onentry,
            onexit: state.onexit,
            initial: state.initial,
            transitions: state
                .transitions
                .into_iter()
                .map(|(event, t)| Edge::new(event, t))
                .collect(),
            emits: state.emits,
        });
        push_children(nodes, index, state.states);
    }
}

/// Load every machine in `<decree_dir>/machines/*.yml` (docs/reference/README.md). A missing `machines/`
/// directory holds no machines.
pub fn load_machines(decree_dir: &Path) -> Result<BTreeMap<String, LoadedMachine>, DecreeError> {
    let mut machines = BTreeMap::new();
    for (id, path) in machine_paths(decree_dir)? {
        let machine = load_machine_file(&id, &path)?;
        machines.insert(id, machine);
    }
    Ok(machines)
}

/// The file each machine id loads from.
pub fn machine_paths(decree_dir: &Path) -> Result<BTreeMap<String, PathBuf>, DecreeError> {
    Ok(machine_files(&decree_dir.join(MACHINES_DIR))?
        .into_iter()
        .collect())
}

/// `(stem, path)` of every `*.yml` file in `dir`, sorted by stem.
fn machine_files(dir: &Path) -> Result<Vec<(String, PathBuf)>, DecreeError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut files = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "yml") && path.is_file() {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                files.push((stem.to_string(), path));
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Read and flatten one machine file. Errors read `machines/<id>.yml: <where>: <message>`.
fn load_machine_file(id: &str, path: &Path) -> Result<LoadedMachine, DecreeError> {
    let text = std::fs::read_to_string(path)?;
    load_machine_text(id, &text)
}

/// Parse and flatten machine `id`, whose file contents are `text`.
pub fn load_machine_text(id: &str, text: &str) -> Result<LoadedMachine, DecreeError> {
    let machine = parse_machine(text)
        .map_err(|e| DecreeError::Other(format!("{MACHINES_DIR}/{id}.yml: {e}")))?;
    Ok(flatten(id, machine))
}

/// Keys outside the SCXML subset that a state may be written with, and what to use instead
/// (V19). SCXML elements are named as such; the rest are decree 0.5 drafts that the
/// decision invokes replaced.
const UNSUPPORTED_STATE_KEYS: [(&str, &str); 13] = [
    ("router", "router on a state is not supported: make the decision a state with invoke: { choose: model, question: ... }"),
    ("default", "default on a state is not supported: a choose: model state takes unsure, or error, instead"),
    ("cond", "cond is not supported: make the decision a state with invoke: { check: ... }"),
    ("parallel", "SCXML <parallel> is not supported: a run is always in exactly one atomic state"),
    ("history", "SCXML <history> is not supported: a run is always in exactly one atomic state"),
    ("send", "SCXML <send> is not supported: a script runs decree emit"),
    ("raise", "SCXML <raise> is not supported: a script prints its event as a JSON line"),
    ("assign", "SCXML <assign> is not supported: data is read-only"),
    ("script", "SCXML <script> is not supported: name a script in invoke, onentry or onexit"),
    ("if", "SCXML <if> is not supported: make the decision a state with invoke: { check: ... }"),
    ("foreach", "SCXML <foreach> is not supported: loop inside a script"),
    ("log", "SCXML <log> is not supported: every script's output is logged"),
    ("donedata", "SCXML <donedata> is not supported: data is read-only"),
];

/// The docs/reference/machines.md message for `cond` on a transition (V19).
const COND_ON_TRANSITION: &str =
    "cond on a transition is not supported: make the decision a state with invoke: { check: ... }";

/// Parse machine YAML. On failure the error starts with the dotted path of the state that
/// fails to deserialize (`work.implement: unknown field ...`), or with `line <n>` when the
/// problem is in the YAML syntax or at the root. A key outside the SCXML subset is reported
/// with its decree alternative, and every unknown key is tagged `(V19)`.
pub fn parse_machine(text: &str) -> Result<Machine, String> {
    let err = match serde_norway::from_str::<Machine>(text) {
        Ok(machine) => return Ok(machine),
        Err(e) => e,
    };
    let at_line = |e: &serde_norway::Error, msg: String| match e.location() {
        Some(loc) => format!("line {}: {msg}", loc.line()),
        None => msg,
    };
    let value: serde_norway::Value = match serde_norway::from_str(text) {
        Ok(value) => value,
        Err(syntax) => return Err(at_line(&syntax, syntax.to_string())),
    };
    if let Some(states) = value.get("states") {
        if let Some(found) = unsupported_key(states, "") {
            return Err(found);
        }
        if let Some((path, msg)) = locate_state_error(states, "") {
            return Err(format!("{path}: {}", tag_unknown(msg)));
        }
    }
    // Not inside a state: check the root alone, without its states, for a clean message.
    let mut root = value;
    if let Some(map) = root.as_mapping_mut() {
        if map.contains_key("states") {
            map.insert("states".into(), serde_norway::Mapping::new().into());
        }
    }
    let msg = match serde_norway::from_value::<Machine>(root) {
        Err(e) => e.to_string(),
        Ok(_) => err.to_string(),
    };
    Err(at_line(&err, tag_unknown(msg)))
}

/// An unknown key is outside the SCXML subset (V19).
fn tag_unknown(msg: String) -> String {
    if msg.contains("unknown field") {
        format!("{msg} (V19)")
    } else {
        msg
    }
}

/// The first state key, or transition `cond`, outside the SCXML subset: `<path>: <message>`.
fn unsupported_key(states: &serde_norway::Value, prefix: &str) -> Option<String> {
    for (key, state) in states.as_mapping()? {
        let path = join_path(prefix, key);
        for (name, message) in UNSUPPORTED_STATE_KEYS {
            if state.get(name).is_some() {
                return Some(format!("{path}: {message} (V19)"));
            }
        }
        let transitions = state.get("transitions").and_then(|t| t.as_mapping());
        for (event, transition) in transitions.into_iter().flatten() {
            if transition.get("cond").is_some() {
                let event = event.as_str().unwrap_or_default();
                return Some(format!(
                    "{path}: transition `{event}`: {COND_ON_TRANSITION} (V19)"
                ));
            }
        }
        if let Some(found) = state
            .get("states")
            .and_then(|children| unsupported_key(children, &path))
        {
            return Some(found);
        }
    }
    None
}

fn join_path(prefix: &str, key: &serde_norway::Value) -> String {
    let id = match key.as_str() {
        Some(id) => id.to_string(),
        None => serde_norway::to_string(key)
            .unwrap_or_default()
            .trim_end()
            .to_string(),
    };
    if prefix.is_empty() {
        id
    } else {
        format!("{prefix}.{id}")
    }
}

/// Find the outermost state whose own keys fail to deserialize, checking each state with its
/// child `states` removed, then descending. Returns its dotted path and the serde message.
fn locate_state_error(states: &serde_norway::Value, prefix: &str) -> Option<(String, String)> {
    let map = states.as_mapping()?;
    for (key, value) in map {
        let path = join_path(prefix, key);
        let mut own = value.clone();
        let children = own
            .as_mapping_mut()
            .and_then(|m| m.insert("states".into(), serde_norway::Mapping::new().into()));
        if let Err(e) = serde_norway::from_value::<State>(own) {
            return Some((path, e.to_string()));
        }
        if let Some(children) = children {
            if let Some(found) = locate_state_error(&children, &path) {
                return Some(found);
            }
        }
    }
    None
}

// =================================================================
// Validation (docs/reference/machines.md, Validation): rules V1–V21 on the arena
// =================================================================

/// The router of a `choose: model` that names none (docs/reference/runs.md, The default router).
pub const ROUTER_MACHINE: &str = "router";

/// Everything outside the machine file that validation reads.
pub struct CheckEnv<'a> {
    pub decree_dir: &'a Path,
    /// Ids of every machine file, including ones that fail to load (V13, V16).
    pub machine_ids: &'a BTreeSet<String>,
    /// Every machine that loaded, for the checks that look into another machine (V8, V16,
    /// V20).
    pub machines: &'a BTreeMap<String, LoadedMachine>,
}

/// One validation error: where it is (a state path or `line <n>`) and what is wrong.
/// The message ends with the rule, e.g. `(V4)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub at: String,
    pub message: String,
}

impl DataType {
    pub fn as_str(self) -> &'static str {
        match self {
            DataType::String => "string",
            DataType::Int => "int",
            DataType::Bool => "bool",
        }
    }

    /// Whether `value` has this type: a YAML string, an integer, or a boolean.
    pub fn matches(self, value: &serde_norway::Value) -> bool {
        match self {
            DataType::String => value.is_string(),
            DataType::Int => value.as_i64().is_some(),
            DataType::Bool => value.is_bool(),
        }
    }
}

/// `^[a-z][a-z0-9_]*$`, the pattern for machine names, state ids and script names.
pub(crate) fn is_ident(s: &str) -> bool {
    let mut bytes = s.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// `^[a-z][a-z0-9_]*(\.[a-z0-9_]+)*$`, the pattern for event names (docs/reference/machines.md, Rules).
pub(crate) fn is_event_name(s: &str) -> bool {
    let mut parts = s.split('.');
    parts.next().is_some_and(is_ident)
        && parts.all(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
}

/// SCXML event matching: descriptor `d` matches `event` if they are equal, or `event`
/// extends `d` after a `.` (`done.state` matches `done.state.work`).
pub fn event_matches(descriptor: &str, event: &str) -> bool {
    event
        .strip_prefix(descriptor)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

impl LoadedMachine {
    pub fn is_compound(&self, i: usize) -> bool {
        !self.nodes[i].children.is_empty()
    }

    /// `i` and its ancestors, innermost first, up to and including the root.
    pub fn chain(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(Some(i), |&n| self.nodes[n].parent)
    }

    /// Whether `event` selects a transition on `i` or one of its ancestors.
    pub fn handles(&self, i: usize, event: &str) -> bool {
        self.chain(i).any(|n| {
            self.nodes[n]
                .transitions
                .iter()
                .any(|e| event_matches(&e.event, event))
        })
    }

    /// The root-level final state `failed`, if the machine has one.
    pub fn failed_state(&self) -> Option<usize> {
        self.nodes[0]
            .children
            .iter()
            .copied()
            .find(|&c| self.nodes[c].id == FAILED && self.nodes[c].is_final)
    }

    /// The atomic (or final) state reached by entering `i`: follow `initial` down.
    /// `None` if an `initial` does not name a direct child (V3, V6 report that).
    pub fn enter(&self, i: usize) -> Option<usize> {
        let mut cur = i;
        while self.is_compound(cur) {
            let initial = self.nodes[cur].initial.as_deref()?;
            cur = self.nodes[cur]
                .children
                .iter()
                .copied()
                .find(|&c| self.nodes[c].id == initial)?;
        }
        Some(cur)
    }

    /// The options of `choose` state `i` (docs/reference/machines.md, Choices): its own transitions except
    /// `unsure` and `error`, in name order.
    pub fn options(&self, i: usize) -> impl Iterator<Item = &Edge> {
        self.nodes[i]
            .transitions
            .iter()
            .filter(|e| !NOT_OPTIONS.contains(&e.event.as_str()))
    }

    /// `DECREE_EVENTS` for scripts of state `i` (docs/reference/scripts.md): the events of its own and its
    /// ancestors' transitions that a script may print, in name order. Empty for the root.
    pub fn accepted_events(&self, i: usize) -> Vec<String> {
        if i == 0 {
            return Vec::new();
        }
        let events: BTreeSet<&str> = self
            .chain(i)
            .flat_map(|n| self.nodes[n].transitions.iter())
            .map(|e| e.event.as_str())
            .filter(|e| !is_reserved_event(e))
            .collect();
        events.into_iter().map(String::from).collect()
    }

    /// The root-level final states of this machine except `failed`: the events a `machine`
    /// invoke of it produces besides `error`.
    pub fn final_events(&self) -> Vec<&str> {
        self.nodes[0]
            .children
            .iter()
            .map(|&c| &self.nodes[c])
            .filter(|n| n.is_final && n.id != FAILED)
            .map(|n| n.id.as_str())
            .collect()
    }

    /// `(state, machine)` for every machine this one runs as a child: `machine` invokes, and
    /// the router of each `choose: model` (its `router`, else [`ROUTER_MACHINE`]).
    pub fn invoked_machines(&self) -> Vec<(usize, &str)> {
        let mut out = Vec::new();
        for (i, node) in self.nodes.iter().enumerate() {
            let child = match &node.invoke {
                Some(Invoke::Machine(m)) => Some(m.machine.as_str()),
                Some(Invoke::Choose(c)) if c.choose == ChooseKind::Model => {
                    Some(c.router.as_deref().unwrap_or(ROUTER_MACHINE))
                }
                _ => None,
            };
            out.extend(child.map(|c| (i, c)));
        }
        out
    }

    /// The states an event raised in atomic or final state `i` leads to (docs/reference/machines.md, Rules):
    /// the first matching transition of `i` or an ancestor; else the event becomes `error`;
    /// an unhandled `error` goes to `failed`.
    fn resolve(&self, i: usize, event: &str) -> Vec<usize> {
        for n in self.chain(i) {
            if let Some(e) = self.nodes[n]
                .transitions
                .iter()
                .find(|e| event_matches(&e.event, event))
            {
                return self.find(&e.target).into_iter().collect();
            }
        }
        if event != "error" {
            return self.resolve(i, "error");
        }
        self.failed_state().into_iter().collect()
    }

    /// Atomic and final states the run can move to from atomic or final state `i`: the
    /// events its invoke can produce (docs/reference/machines.md, Invoke), `error` from an invoke or an
    /// `onentry` script, and `done.state.<parent>` from a nested final state.
    fn successors(&self, i: usize) -> Vec<usize> {
        let node = &self.nodes[i];
        let mut events: Vec<String> = Vec::new();
        let mut any_event = false;
        let mut can_error;
        if node.is_final {
            match node.parent {
                Some(p) if p != 0 => events.push(format!("done.state.{}", self.nodes[p].id)),
                _ => return Vec::new(),
            }
            can_error = !node.onentry.is_empty();
        } else {
            can_error = self.chain(i).any(|n| !self.nodes[n].onentry.is_empty());
            match &node.invoke {
                None => events.push("done".into()),
                // A script may print, and a child machine may end in, any event.
                Some(Invoke::Script(_) | Invoke::Machine(_)) => {
                    any_event = true;
                    can_error = true;
                }
                Some(Invoke::Check(_)) => events.extend(["yes".into(), "no".into()]),
                Some(Invoke::Choose(c)) => {
                    events.extend(self.options(i).map(|e| e.event.clone()));
                    if c.choose == ChooseKind::Model && c.min_confidence.is_some() {
                        events.push("unsure".into());
                    }
                    can_error = true;
                }
            }
        }
        if can_error {
            events.push("error".into());
        }
        let mut targets: Vec<usize> = Vec::new();
        if any_event {
            for n in self.chain(i) {
                targets.extend(
                    self.nodes[n]
                        .transitions
                        .iter()
                        .filter_map(|e| self.find(&e.target)),
                );
            }
        }
        for event in &events {
            targets.extend(self.resolve(i, event));
        }
        targets.into_iter().filter_map(|t| self.enter(t)).collect()
    }

    /// Run V1–V21 on this machine. `text` is the machine file, for root-level line numbers.
    pub fn validate(&self, text: &str, env: &CheckEnv) -> Vec<Problem> {
        let mut v = Validator {
            m: self,
            text,
            env,
            problems: Vec::new(),
        };
        v.v1_name();
        v.v2_state_ids();
        v.v3_initial();
        v.v4_targets();
        v.v5_failed();
        v.v6_compound();
        v.v7_final();
        v.v8_decisions();
        v.v9_input();
        v.v10_conditions();
        v.v11_reachable();
        v.v12_scripts();
        v.v13_emits();
        v.v14_data();
        v.v15_done_state();
        v.v16_invokes();
        v.v17_internal();
        v.v18_events();
        v.v19_choose_keys();
        v.v20_cycles();
        v.v21_overlapping_events();
        v.problems
    }
}

struct Validator<'a> {
    m: &'a LoadedMachine,
    text: &'a str,
    env: &'a CheckEnv<'a>,
    problems: Vec<Problem>,
}

/// `` `a`, `b` `` for a list of keys.
fn backticked(keys: &[&str]) -> String {
    keys.iter()
        .map(|k| format!("`{k}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

impl Validator<'_> {
    fn push(&mut self, at: String, message: String) {
        self.problems.push(Problem { at, message });
    }

    /// `line <n>` of root key `key`, for problems outside any state.
    fn root_line(&self, key: &str) -> String {
        let n = self
            .text
            .lines()
            .position(|l| l.strip_prefix(key).is_some_and(|r| r.starts_with(':')))
            .map_or(1, |i| i + 1);
        format!("line {n}")
    }

    /// `line <n>` of entry `name` under root key `key` (block style), else of `key`.
    fn nested_line(&self, key: &str, name: &str) -> String {
        let lines: Vec<&str> = self.text.lines().collect();
        let start = lines
            .iter()
            .position(|l| l.strip_prefix(key).is_some_and(|r| r.starts_with(':')));
        let found = start.and_then(|s| {
            lines[s + 1..]
                .iter()
                .take_while(|l| l.is_empty() || l.starts_with([' ', '#']))
                .position(|l| {
                    l.trim_start()
                        .strip_prefix(name)
                        .is_some_and(|r| r.starts_with(':'))
                })
                .map(|i| s + 1 + i)
        });
        match found.or(start) {
            Some(i) => format!("line {}", i + 1),
            None => "line 1".to_string(),
        }
    }

    fn at(&self, i: usize) -> String {
        if i == 0 {
            self.root_line("initial")
        } else {
            self.m.state_path(i)
        }
    }

    fn states(&self) -> std::ops::Range<usize> {
        1..self.m.nodes.len()
    }

    /// Non-final atomic states: the ones that invoke.
    fn atomic(&self) -> Vec<usize> {
        self.states()
            .filter(|&i| !self.m.nodes[i].is_final && !self.m.is_compound(i))
            .collect()
    }

    fn v1_name(&mut self) {
        let name = &self.m.nodes[0].id;
        if !is_ident(name) {
            self.push(
                self.root_line("name"),
                format!("name `{name}` does not match ^[a-z][a-z0-9_]*$ (V1)"),
            );
        }
        if *name != self.m.id {
            self.push(
                self.root_line("name"),
                format!(
                    "name `{name}` does not equal the file stem `{}` (V1)",
                    self.m.id
                ),
            );
        }
    }

    fn v2_state_ids(&mut self) {
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        for i in self.states() {
            let id = self.m.nodes[i].id.as_str();
            if !is_ident(id) {
                self.push(
                    self.m.state_path(i),
                    format!("state id `{id}` does not match ^[a-z][a-z0-9_]*$ (V2)"),
                );
            }
            match seen.get(id) {
                Some(&first) => self.push(
                    self.m.state_path(i),
                    format!(
                        "state id `{id}` is also used by `{}`; state ids are unique across the machine (V2)",
                        self.m.state_path(first)
                    ),
                ),
                None => {
                    seen.insert(id, i);
                }
            }
        }
    }

    fn v3_initial(&mut self) {
        for i in std::iter::once(0).chain(self.states()) {
            let node = &self.m.nodes[i];
            if i != 0 && !self.m.is_compound(i) {
                continue;
            }
            let Some(initial) = &node.initial else {
                continue;
            };
            if !node
                .children
                .iter()
                .any(|&c| self.m.nodes[c].id == *initial)
            {
                self.push(
                    self.at(i),
                    format!("initial `{initial}` is not a direct child state (V3)"),
                );
            }
        }
    }

    fn v4_targets(&mut self) {
        for i in self.states() {
            for e in &self.m.nodes[i].transitions {
                if self.m.find(&e.target).is_none() {
                    self.push(
                        self.m.state_path(i),
                        format!(
                            "transition `{}` targets unknown state `{}` (V4)",
                            e.event, e.target
                        ),
                    );
                }
            }
        }
    }

    fn v5_failed(&mut self) {
        if self.m.failed_state().is_none() {
            self.push(
                self.root_line("states"),
                format!(
                    "no root-level final state `{FAILED}`: every machine needs one for unhandled errors (V5)"
                ),
            );
        }
    }

    fn v6_compound(&mut self) {
        for i in self.states() {
            let node = &self.m.nodes[i];
            if node.is_final {
                continue; // V7
            }
            let at = self.m.state_path(i);
            if self.m.is_compound(i) {
                if node.initial.is_none() {
                    self.push(at.clone(), "compound state has no `initial` (V6)".into());
                }
                if node.invoke.is_some() {
                    self.push(at, "compound state may not have `invoke` (V6)".into());
                }
            } else if node.initial.is_some() {
                self.push(
                    at,
                    "`initial` without `states`: they are present together on compound states (V6)"
                        .into(),
                );
            }
        }
    }

    fn v7_final(&mut self) {
        for i in self.states() {
            let node = &self.m.nodes[i];
            if !node.is_final {
                continue;
            }
            let found: Vec<&str> = [
                ("invoke", node.invoke.is_some()),
                ("max_attempts", node.max_attempts.is_some()),
                ("timeout_s", node.timeout_s.is_some()),
                ("onexit", !node.onexit.is_empty()),
                ("initial", node.initial.is_some()),
                ("states", !node.children.is_empty()),
                ("transitions", !node.transitions.is_empty()),
            ]
            .into_iter()
            .filter_map(|(key, present)| present.then_some(key))
            .collect();
            if !found.is_empty() {
                self.push(
                    self.m.state_path(i),
                    format!(
                        "final state may only have `final`, `description`, `onentry` and `emits`, not {} (V7)",
                        backticked(&found)
                    ),
                );
            }
        }
    }

    fn v8_decisions(&mut self) {
        for i in self.atomic() {
            let at = self.m.state_path(i);
            match &self.m.nodes[i].invoke {
                Some(Invoke::Check(_)) => {
                    for event in ["yes", "no"] {
                        if !self.m.handles(i, event) {
                            self.push(
                                at.clone(),
                                format!("a `check` state must handle `{event}`, itself or through an ancestor (V8)"),
                            );
                        }
                    }
                }
                Some(Invoke::Choose(c)) => {
                    let kind = match c.choose {
                        ChooseKind::Model => "choose: model",
                        ChooseKind::Person => "choose: person",
                    };
                    if c.question.as_deref().is_none_or(|q| q.trim().is_empty()) {
                        self.push(
                            at.clone(),
                            format!(
                                "a `{kind}` state needs a `question`: what is being decided (V8)"
                            ),
                        );
                    }
                    let options: Vec<&Edge> = self.m.options(i).collect();
                    if options.len() < 2 {
                        self.push(
                            at.clone(),
                            format!(
                                "a `{kind}` state needs at least 2 options (transitions other than `unsure` and `error`), has {} (V8)",
                                options.len()
                            ),
                        );
                    }
                    for e in options {
                        if e.description.as_deref().is_none_or(|d| d.trim().is_empty()) {
                            self.push(
                                at.clone(),
                                format!(
                                    "option `{}` needs a `description`: write it as `{}: {{ target: {}, description: ... }}` (V8)",
                                    e.event, e.event, e.target
                                ),
                            );
                        }
                    }
                    if c.choose == ChooseKind::Model
                        && c.min_confidence.is_some()
                        && !self.m.handles(i, "unsure")
                    {
                        self.push(
                            at.clone(),
                            "a `choose: model` state with `min_confidence` must handle `unsure`, itself or through an ancestor (V8)".into(),
                        );
                    }
                }
                Some(Invoke::Machine(mi)) => {
                    let Some(child) = self.env.machines.get(&mi.machine) else {
                        continue; // V16
                    };
                    for event in child.final_events() {
                        if !self.m.handles(i, event) {
                            self.push(
                                at.clone(),
                                format!(
                                    "machine `{}` can end in `{event}`, which this state does not handle, itself or through an ancestor (V8)",
                                    mi.machine
                                ),
                            );
                        }
                    }
                }
                Some(Invoke::Script(_)) | None => {}
            }
        }
    }

    fn v9_input(&mut self) {
        for i in self.atomic() {
            let Some(invoke) = &self.m.nodes[i].invoke else {
                continue;
            };
            let at = self.m.state_path(i);
            if let Some(input) = invoke.input() {
                let is_script = self
                    .m
                    .find(input)
                    .and_then(|s| self.m.nodes[s].invoke.as_ref())
                    .is_some_and(|inv| inv.script().is_some());
                if !is_script {
                    self.push(
                        at,
                        format!("input `{input}` is not a state with a script invoke (V9)"),
                    );
                }
            } else if let Invoke::Check(c) = invoke {
                if matches!(c.check.shape(), Ok((Subject::Matches(_), _))) && !self.script_before(i)
                {
                    self.push(
                        at,
                        "`matches` without `input` reads the most recent script's output, but no script state comes before this state (V9)".into(),
                    );
                }
            }
        }
    }

    /// Whether some state that invokes a script can lead to atomic state `i`.
    fn script_before(&self, i: usize) -> bool {
        let m = self.m;
        let scripts = self.atomic().into_iter().filter(|&s| {
            m.nodes[s]
                .invoke
                .as_ref()
                .is_some_and(|inv| inv.script().is_some())
        });
        for start in scripts {
            let mut seen = vec![false; m.nodes.len()];
            let mut queue = m.successors(start);
            while let Some(s) = queue.pop() {
                if s == i {
                    return true;
                }
                if !std::mem::replace(&mut seen[s], true) {
                    queue.extend(m.successors(s));
                }
            }
        }
        false
    }

    fn v10_conditions(&mut self) {
        for i in self.atomic() {
            let Some(Invoke::Check(c)) = &self.m.nodes[i].invoke else {
                continue;
            };
            let at = self.m.state_path(i);
            for message in self.condition_problems(&c.check) {
                self.push(at.clone(), format!("check: {message} (V10)"));
            }
        }
    }

    /// What is wrong with one condition, without the rule tag.
    fn condition_problems(&self, cond: &Condition) -> Vec<String> {
        let (subject, test) = match cond.shape() {
            Ok(shape) => shape,
            Err(e) => return vec![e.to_string()],
        };
        let mut out = Vec::new();
        let left = match subject {
            Subject::Matches(pattern) => {
                if let Err(e) = crate::cond::compile(pattern) {
                    out.push(e.to_string());
                }
                return out;
            }
            Subject::Visits(state) => {
                let atomic = self
                    .m
                    .find(state)
                    .is_some_and(|s| !self.m.is_compound(s) && !self.m.nodes[s].is_final);
                if !atomic {
                    out.push(format!(
                        "`visits` names `{state}`, which is not an atomic state"
                    ));
                }
                Some(DataType::Int)
            }
            Subject::Data(name) => self.data_type(name, &mut out),
            Subject::Confidence(state) => {
                let model = self.m.find(state).is_some_and(|s| {
                    self.m.nodes[s]
                        .invoke
                        .as_ref()
                        .and_then(|i| i.choose(ChooseKind::Model))
                        .is_some()
                });
                if !model {
                    out.push(format!(
                        "`confidence` names `{state}`, which is not a `choose: model` state"
                    ));
                }
                if let Some(Test::Compare(_, operand)) = test {
                    if let Err(e) = crate::cond::confidence_operand(operand) {
                        out.push(e.to_string());
                    }
                }
                return out;
            }
        };
        let (op, operand) = match test {
            None => return out,
            Some(Test::Matches(pattern)) => {
                match left {
                    Some(DataType::String) | None => {}
                    Some(kind) => out.push(format!(
                        "`matches` needs string data, but `{}` is {}",
                        cond.data.as_deref().unwrap_or_default(),
                        kind.as_str()
                    )),
                }
                if let Err(e) = crate::cond::compile(pattern) {
                    out.push(e.to_string());
                }
                return out;
            }
            Some(Test::Compare(op, operand)) => (op, operand),
        };
        let right = match operand {
            Operand::Int(_) => Some(DataType::Int),
            Operand::Float(x) => {
                out.push(format!(
                    "`{}` compares with {x}, but only `confidence` takes a number that is not an int",
                    subject.key()
                ));
                None
            }
            Operand::Str(_) => Some(DataType::String),
            Operand::Bool(_) => Some(DataType::Bool),
            Operand::Data(name) => self.data_type(name, &mut out),
        };
        if let (Some(left), Some(right)) = (left, right) {
            if left != right {
                out.push(format!(
                    "`{}` compares {} with {}",
                    subject.key(),
                    left.as_str(),
                    right.as_str()
                ));
            } else if op.is_ordering() && left != DataType::Int {
                out.push(format!("`{op}` compares ints only, not {}", left.as_str()));
            }
        }
        out
    }

    /// The type of `data` entry `name`, or a problem if there is none.
    fn data_type(&self, name: &str, out: &mut Vec<String>) -> Option<DataType> {
        match self.m.data.get(name) {
            Some(spec) => Some(spec.kind),
            None => {
                out.push(format!("unknown data `{name}`"));
                None
            }
        }
    }

    fn v11_reachable(&mut self) {
        let m = self.m;
        let start = m.nodes[0]
            .initial
            .as_deref()
            .and_then(|init| {
                m.nodes[0]
                    .children
                    .iter()
                    .copied()
                    .find(|&c| m.nodes[c].id == init)
            })
            .and_then(|c| m.enter(c));
        let Some(start) = start else {
            return; // V3 or V6 already reports the broken `initial`
        };

        // Forward: every atomic or final state the run can be in.
        let n = m.nodes.len();
        let mut visited = vec![false; n];
        let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut queue = vec![start];
        // An unhandled error can always reach `failed`, from the root `onentry` at least.
        queue.extend(m.failed_state());
        while let Some(i) = queue.pop() {
            if std::mem::replace(&mut visited[i], true) {
                continue;
            }
            succ[i] = m.successors(i);
            queue.extend(succ[i].iter().copied().filter(|&s| !visited[s]));
        }
        let mut reachable = vec![false; n];
        for i in (0..n).filter(|&i| visited[i]) {
            for a in m.chain(i) {
                reachable[a] = true;
            }
        }
        for i in self.states() {
            let parent_reachable = m.nodes[i].parent.is_some_and(|p| reachable[p]);
            if !reachable[i] && parent_reachable {
                self.push(
                    m.state_path(i),
                    "state is unreachable from the root `initial` (V11)".into(),
                );
            }
        }

        // Backward: which of those can still reach a root-level final state.
        let mut good: Vec<bool> = (0..n)
            .map(|i| visited[i] && m.nodes[i].is_final && m.nodes[i].parent == Some(0))
            .collect();
        loop {
            let mut changed = false;
            for i in 0..n {
                if visited[i] && !good[i] && succ[i].iter().any(|&s| good[s]) {
                    good[i] = true;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        for i in (0..n).filter(|&i| visited[i] && !good[i] && !m.nodes[i].is_final) {
            self.push(
                m.state_path(i),
                "state cannot reach a root-level final state (V11)".into(),
            );
        }
    }

    fn v12_scripts(&mut self) {
        let mut uses: Vec<(String, &str)> = Vec::new();
        let root = &self.m.nodes[0];
        for name in &root.onentry {
            uses.push((self.root_line("onentry"), name));
        }
        for name in &root.onexit {
            uses.push((self.root_line("onexit"), name));
        }
        for i in self.states() {
            let node = &self.m.nodes[i];
            let at = self.m.state_path(i);
            let invoked = match &node.invoke {
                Some(Invoke::Script(name)) => Some(name.as_str()),
                Some(Invoke::Choose(c)) if c.choose == ChooseKind::Person => {
                    if c.ask.is_none() {
                        self.push(
                            at.clone(),
                            "a `choose: person` state needs an `ask` script, which tells someone how to reply (V12)".into(),
                        );
                    }
                    c.ask.as_deref()
                }
                _ => None,
            };
            let scripts = invoked
                .into_iter()
                .chain(node.onentry.iter().map(String::as_str))
                .chain(node.onexit.iter().map(String::as_str));
            for name in scripts {
                uses.push((at.clone(), name));
            }
        }
        let prefix = format!("{}/", self.env.decree_dir.display());
        for (at, name) in uses {
            if let Err(e) = resolve_script(self.env.decree_dir, &self.m.id, name) {
                self.push(at, format!("{} (V12)", e.to_string().replace(&prefix, "")));
            }
        }
    }

    fn v13_emits(&mut self) {
        for i in self.states() {
            for target in &self.m.nodes[i].emits {
                if !self.env.machine_ids.contains(target) {
                    self.push(
                        self.m.state_path(i),
                        format!("emits unknown machine `{target}` (V13)"),
                    );
                }
            }
        }
    }

    fn v14_data(&mut self) {
        for (name, spec) in &self.m.data {
            if !spec.kind.matches(&spec.default) {
                let value = serde_norway::to_string(&spec.default).unwrap_or_default();
                self.push(
                    self.nested_line("data", name),
                    format!(
                        "data `{name}`: default `{}` is not of type `{}` (V14)",
                        value.trim_end(),
                        spec.kind.as_str()
                    ),
                );
            }
        }
    }

    fn v15_done_state(&mut self) {
        for i in self.states() {
            let node = &self.m.nodes[i];
            let has_final = node.children.iter().any(|&c| self.m.nodes[c].is_final);
            let event = format!("done.state.{}", node.id);
            if has_final && !self.m.handles(i, &event) {
                self.push(
                    self.m.state_path(i),
                    format!("compound state has a final state, but nothing handles `{event}`, itself or through an ancestor (V15)"),
                );
            }
        }
    }

    fn v16_invokes(&mut self) {
        for i in self.states() {
            let node = &self.m.nodes[i];
            let at = self.m.state_path(i);
            match &node.invoke {
                Some(Invoke::Machine(mi)) => self.child_machine(&at, mi),
                Some(Invoke::Choose(c)) if c.choose == ChooseKind::Model => {
                    match c.router.as_deref() {
                        Some(router) if !self.env.machine_ids.contains(router) => self.push(
                            at.clone(),
                            format!("router `{router}` is not a machine (V16)"),
                        ),
                        None if !self.env.machine_ids.contains(ROUTER_MACHINE) => self.push(
                            at.clone(),
                            format!("`choose: model` names no `router`, and there is no machine named `{ROUTER_MACHINE}` (V16)"),
                        ),
                        _ => {}
                    }
                    if let Some(n) = c.min_confidence.filter(|n| !(0.0..=1.0).contains(n)) {
                        self.push(
                            at.clone(),
                            format!("min_confidence {n} is not between 0 and 1 (V16)"),
                        );
                    }
                }
                _ => {}
            }
            let is_script = node
                .invoke
                .as_ref()
                .is_some_and(|inv| inv.script().is_some());
            if node.max_attempts.is_some() && !is_script && !node.is_final {
                self.push(
                    at,
                    "`max_attempts` is only allowed on states that invoke a script (V16)".into(),
                );
            }
        }
    }

    /// V16 for `invoke: { machine, params }`.
    fn child_machine(&mut self, at: &str, mi: &MachineInvoke) {
        if !self.env.machine_ids.contains(&mi.machine) {
            self.push(
                at.to_string(),
                format!("machine `{}` does not exist (V16)", mi.machine),
            );
            return;
        }
        let Some(child) = self.env.machines.get(&mi.machine) else {
            return; // it fails to load, and is reported on its own
        };
        for (key, value) in &mi.params {
            let key = key.as_str().unwrap_or_default();
            match child.data.get(key) {
                None => self.push(
                    at.to_string(),
                    format!(
                        "unknown param `{key}`: machine `{}` has no data `{key}` (V16)",
                        mi.machine
                    ),
                ),
                Some(spec) if !spec.kind.matches(value) => self.push(
                    at.to_string(),
                    format!(
                        "param `{key}` must be of type `{}` (V16)",
                        spec.kind.as_str()
                    ),
                ),
                Some(_) => {}
            }
        }
    }

    fn v17_internal(&mut self) {
        for i in self.states() {
            for e in &self.m.nodes[i].transitions {
                if !e.internal {
                    continue;
                }
                let descendant = self
                    .m
                    .find(&e.target)
                    .is_some_and(|t| t != i && self.m.chain(t).any(|a| a == i));
                if !self.m.is_compound(i) || !descendant {
                    self.push(
                        self.m.state_path(i),
                        format!(
                            "transition `{}`: `type: internal` is only allowed from a compound state to one of its descendants (V17)",
                            e.event
                        ),
                    );
                }
            }
        }
    }

    fn v18_events(&mut self) {
        for i in self.states() {
            let node = &self.m.nodes[i];
            let at = self.m.state_path(i);
            for e in &node.transitions {
                if !is_event_name(&e.event) {
                    self.push(
                        at.clone(),
                        format!(
                            "event `{}` does not match ^[a-z][a-z0-9_]*(\\.[a-z0-9_]+)*$ (V18)",
                            e.event
                        ),
                    );
                }
            }
            if matches!(node.invoke, Some(Invoke::Choose(_))) {
                let reserved: Vec<String> = self
                    .m
                    .options(i)
                    .filter(|e| is_reserved_event(&e.event))
                    .map(|e| e.event.clone())
                    .collect();
                for event in reserved {
                    self.push(
                        at.clone(),
                        format!("option `{event}` is reserved: `done`, `error`, `unsure` and names starting with `done.` or `error.` cannot be options (V18)"),
                    );
                }
            }
        }
    }

    /// V19 inside an invoke: the `choose` keys that belong to the other kind.
    fn v19_choose_keys(&mut self) {
        for i in self.states() {
            let Some(Invoke::Choose(c)) = &self.m.nodes[i].invoke else {
                continue;
            };
            let (kind, keys) = match c.choose {
                ChooseKind::Model => (
                    "choose: model",
                    vec![
                        ("ask", c.ask.is_some()),
                        ("timeout_s", c.timeout_s.is_some()),
                    ],
                ),
                ChooseKind::Person => (
                    "choose: person",
                    vec![
                        ("router", c.router.is_some()),
                        ("min_confidence", c.min_confidence.is_some()),
                        ("input", c.input.is_some()),
                    ],
                ),
            };
            let found: Vec<&str> = keys
                .into_iter()
                .filter_map(|(key, set)| set.then_some(key))
                .collect();
            if !found.is_empty() {
                self.push(
                    self.m.state_path(i),
                    format!("`{kind}` does not take {} (V19)", backticked(&found)),
                );
            }
        }
    }

    fn v20_cycles(&mut self) {
        let me = self.m.id.as_str();
        for (i, child) in self.m.invoked_machines() {
            if let Some(path) = self.invoke_path(child, me) {
                self.push(
                    self.m.state_path(i),
                    format!(
                        "machine `{me}` invokes itself: {me} -> {}; a machine never invokes itself, directly or through others (V20)",
                        path.join(" -> ")
                    ),
                );
            }
        }
    }

    /// V21: no transition's event extends another's in the same state, since SCXML picks
    /// between them by document order, which a YAML map does not keep.
    fn v21_overlapping_events(&mut self) {
        for i in self.states() {
            let events: Vec<&str> = self.m.nodes[i]
                .transitions
                .iter()
                .map(|e| e.event.as_str())
                .collect();
            for &short in &events {
                for &long in &events {
                    if short != long && event_matches(short, long) {
                        self.push(
                            self.m.state_path(i),
                            format!(
                                "events `{short}` and `{long}` overlap: `{short}` also matches `{long}`, so at most one transition per state may match an event (V21)"
                            ),
                        );
                    }
                }
            }
        }
    }

    /// The machines from `from` to `to` through `machine` and router invokes, both ends
    /// included, if `to` can be reached.
    fn invoke_path<'m>(&'m self, from: &'m str, to: &str) -> Option<Vec<&'m str>> {
        let mut parent: BTreeMap<&str, &str> = BTreeMap::new();
        let mut queue = std::collections::VecDeque::from([from]);
        let mut seen = BTreeSet::from([from]);
        while let Some(id) = queue.pop_front() {
            if id == to {
                let mut path = vec![id];
                let mut cur = id;
                while let Some(&p) = parent.get(cur) {
                    path.push(p);
                    cur = p;
                }
                path.reverse();
                return Some(path);
            }
            let Some(m) = self.env.machines.get(id) else {
                continue;
            };
            for (_, child) in m.invoked_machines() {
                if seen.insert(child) {
                    parent.insert(child, id);
                    queue.push_back(child);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// The docs/reference/machines.md examples, plus the router `feature` and `triage` name by default.
    const EXAMPLES: [&str; 5] = ["hello", "deploy", "ship", "feature", "router"];

    fn fixture(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/machines")
            .join(format!("{name}.yml"));
        fs::read_to_string(path).unwrap()
    }

    /// A temp project with `.decree/machines/` holding the given files.
    fn project(files: &[(&str, &str)]) -> TempDir {
        let tmp = TempDir::new().unwrap();
        write_machines(&tmp.path().join(".decree"), files);
        tmp
    }

    fn write_machines(base: &Path, files: &[(&str, &str)]) {
        let dir = base.join(MACHINES_DIR);
        fs::create_dir_all(&dir).unwrap();
        for (id, text) in files {
            fs::write(dir.join(format!("{id}.yml")), text).unwrap();
        }
    }

    fn load_err(id: &str, text: &str) -> String {
        let tmp = project(&[(id, text)]);
        load_machines(&tmp.path().join(".decree"))
            .unwrap_err()
            .to_string()
    }

    fn load(id: &str, text: &str) -> LoadedMachine {
        flatten(id, parse_machine(text).unwrap())
    }

    #[test]
    fn section5_examples_load() {
        let files: Vec<(&str, String)> = EXAMPLES.iter().map(|n| (*n, fixture(n))).collect();
        let refs: Vec<(&str, &str)> = files.iter().map(|(n, t)| (*n, t.as_str())).collect();
        let tmp = project(&refs);
        let machines = load_machines(&tmp.path().join(".decree")).unwrap();
        assert_eq!(
            machines.keys().map(String::as_str).collect::<Vec<_>>(),
            ["deploy", "feature", "hello", "router", "ship"]
        );
        for (id, m) in &machines {
            assert_eq!(&m.root().id, id);
            assert!(!m.description().is_empty());
        }
        assert_eq!(machines["hello"].nodes.len(), 1 + 3);
        assert_eq!(machines["deploy"].nodes.len(), 1 + 6);
        assert_eq!(machines["ship"].nodes.len(), 1 + 4);

        let deploy = &machines["deploy"];
        let approval = &deploy.nodes[deploy.find("approval").unwrap()];
        let Some(Invoke::Choose(c)) = &approval.invoke else {
            panic!("{:?}", approval.invoke);
        };
        assert_eq!(c.choose, ChooseKind::Person);
        assert_eq!(c.question.as_deref(), Some("Ship this build?"));
        assert_eq!(c.ask.as_deref(), Some("ask_person"));
        assert_eq!(c.timeout_s, Some(86400));

        let ship = &machines["ship"];
        let build = &ship.nodes[ship.find("build").unwrap()];
        assert_eq!(
            build.invoke,
            Some(Invoke::Machine(MachineInvoke {
                machine: "feature".into(),
                params: serde_norway::Mapping::new(),
            }))
        );
    }

    #[test]
    fn feature_arena_has_eleven_states_plus_root() {
        let m = load("feature", &fixture("feature"));
        assert_eq!(m.nodes.len(), 11 + 1);

        let root = m.root();
        assert_eq!(root.id, "feature");
        assert_eq!((root.parent, root.depth), (None, 0));
        assert_eq!(root.initial.as_deref(), Some("precheck"));
        assert_eq!(root.onentry, ["git_baseline"]);
        assert_eq!(root.onexit, ["notify"]);
        assert_eq!(m.data["max_rounds"].kind, DataType::Int);
        assert_eq!(m.data["max_rounds"].default, serde_norway::Value::from(2));

        let ids: Vec<&str> = m.nodes[1..].iter().map(|n| n.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "done",
                "failed",
                "precheck",
                "spawn_followups",
                "work",
                "implement",
                "review",
                "rounds_left",
                "triage",
                "verified",
                "verify",
            ]
        );

        let work = m.find("work").unwrap();
        assert_eq!(m.nodes[work].parent, Some(0));
        assert_eq!(m.nodes[work].depth, 1);
        assert_eq!(m.nodes[work].initial.as_deref(), Some("implement"));
        assert_eq!(m.nodes[work].children.len(), 6);
        assert_eq!(m.nodes[work].transitions[0].event, "done.state.work");

        let verify = m.find("verify").unwrap();
        assert_eq!(m.state_path(verify), "work.verify");
        assert_eq!(m.nodes[verify].depth, 2);
        assert_eq!(
            m.nodes[verify].invoke,
            Some(Invoke::Script("verify".into()))
        );

        let rounds = &m.nodes[m.find("rounds_left").unwrap()];
        let Some(Invoke::Check(check)) = &rounds.invoke else {
            panic!("{:?}", rounds.invoke);
        };
        assert_eq!(check.check.visits.as_deref(), Some("implement"));
        assert_eq!(
            check.check.less_than,
            Some(Operand::Data("max_rounds".into()))
        );
        // `yes` and `no` are strings, not YAML 1.1 booleans.
        let events: Vec<&str> = rounds
            .transitions
            .iter()
            .map(|e| e.event.as_str())
            .collect();
        assert_eq!(events, ["no", "yes"]);

        let triage = m.find("triage").unwrap();
        let Some(Invoke::Choose(c)) = &m.nodes[triage].invoke else {
            panic!();
        };
        assert_eq!(c.choose, ChooseKind::Model);
        assert_eq!(c.min_confidence, Some(0.8));
        assert_eq!(c.input.as_deref(), Some("verify"));
        assert_eq!(c.router, None);
        let options: Vec<&str> = m.options(triage).map(|e| e.event.as_str()).collect();
        assert_eq!(options, ["retry", "split"]);
        let retry = m.options(triage).next().unwrap();
        assert_eq!(retry.target, "implement");
        assert_eq!(
            retry.description.as_deref(),
            Some("The failures look fixable; implement again.")
        );
        assert!(!retry.internal);

        let implement = &m.nodes[m.find("implement").unwrap()];
        assert_eq!(implement.max_attempts, Some(2));
        assert_eq!(implement.onentry, ["snapshot"]);
        assert_eq!(implement.onexit, ["collect_logs"]);

        let done = &m.nodes[m.find("done").unwrap()];
        assert!(done.is_final);
        assert_eq!(done.onentry, ["commit"]);
        assert_eq!(
            m.nodes[m.find("spawn_followups").unwrap()].emits,
            ["feature"]
        );
    }

    #[test]
    fn accepted_events_skip_reserved_names_and_include_ancestors() {
        let m = load("feature", &fixture("feature"));
        assert_eq!(
            m.accepted_events(m.find("verify").unwrap()),
            ["fail", "pass"]
        );
        assert_eq!(
            m.accepted_events(m.find("precheck").unwrap()),
            Vec::<String>::new()
        );
        assert_eq!(
            m.accepted_events(m.find("triage").unwrap()),
            ["retry", "split"]
        );
        assert!(m.accepted_events(0).is_empty());
    }

    #[test]
    fn misspelled_state_key_names_file_and_state_path() {
        let text = fixture("feature").replace("max_attempts: 2", "max_attempt: 2");
        let err = load_err("feature", &text);
        assert!(
            err.starts_with("machines/feature.yml: work.implement: unknown field `max_attempt`"),
            "{err}"
        );
        assert!(err.ends_with("(V19)"), "{err}");
    }

    #[test]
    fn misspelled_top_level_state_key() {
        let text = fixture("hello").replace("invoke: greet ", "invokes: greet ");
        let err = load_err("hello", &text);
        assert!(
            err.starts_with("machines/hello.yml: greet: unknown field `invokes`"),
            "{err}"
        );
    }

    #[test]
    fn misspelled_invoke_key_names_its_state() {
        let text = fixture("feature").replace("min_confidence: 0.8", "min_confidense: 0.8");
        let err = load_err("feature", &text);
        assert!(
            err.starts_with("machines/feature.yml: work.triage: unknown field `min_confidense`"),
            "{err}"
        );
    }

    #[test]
    fn misspelled_condition_key_names_its_state() {
        let text = fixture("feature").replace("less_than:", "lesser_than:");
        let err = load_err("feature", &text);
        assert!(
            err.starts_with("machines/feature.yml: work.rounds_left: unknown field `lesser_than`"),
            "{err}"
        );
    }

    #[test]
    fn invoke_object_without_a_type_key() {
        let text = fixture("hello").replace("invoke: greet ", "invoke: { run: greet }");
        let err = load_err("hello", &text);
        assert!(
            err.contains(
                "greet: `invoke` is a script name or an object with `machine`, `check` or `choose`"
            ),
            "{err}"
        );
    }

    #[test]
    fn misspelled_root_key_names_line() {
        let text = fixture("hello").replace("description:", "descripton:");
        let err = load_err("hello", &text);
        assert!(
            err.starts_with("machines/hello.yml: line 3: unknown field `descripton`"),
            "{err}"
        );
    }

    #[test]
    fn yaml_syntax_error_names_line() {
        let err = load_err("bad", "name: bad\nstates: [\n");
        assert!(err.starts_with("machines/bad.yml: line "), "{err}");
    }

    // Keys outside the SCXML subset (V19), with the docs/reference/machines.md messages.

    #[test]
    fn cond_on_a_transition_names_the_alternative() {
        let text = format!(
            "{HEAD}  a:\n    invoke: x\n    transitions:\n      \
             done: {{ target: done, cond: \"visits.a < 2\" }}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        assert_eq!(
            parse_machine(&text).unwrap_err(),
            "a: transition `done`: cond on a transition is not supported: make the decision a state with invoke: { check: ... } (V19)"
        );
    }

    #[test]
    fn router_llm_on_a_state_names_the_alternative() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/check/v19-scxml/fail/machines/b.yml");
        let text = fs::read_to_string(path).unwrap();
        assert_eq!(
            parse_machine(&text).unwrap_err(),
            "work.step: router on a state is not supported: make the decision a state with invoke: { choose: model, question: ... } (V19)"
        );
    }

    #[test]
    fn scxml_elements_name_the_feature() {
        let text = format!(
            "{HEAD}  a: {{ parallel: true, transitions: {{ done: done }} }}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        assert_eq!(
            parse_machine(&text).unwrap_err(),
            "a: SCXML <parallel> is not supported: a run is always in exactly one atomic state (V19)"
        );
    }

    #[test]
    fn machine_error_uses_relative_path() {
        let text = fixture("hello").replace("failed: { final: true }", "failed: { fnal: true }");
        let tmp = project(&[("hello", &text)]);
        let err = load_machines(&tmp.path().join(".decree"))
            .unwrap_err()
            .to_string();
        assert!(
            err.starts_with("machines/hello.yml: failed: unknown field `fnal`"),
            "{err}"
        );
    }

    #[test]
    fn missing_dir_loads_nothing() {
        let tmp = TempDir::new().unwrap();
        let machines = load_machines(&tmp.path().join(".decree")).unwrap();
        assert!(machines.is_empty());
    }

    #[test]
    fn only_yml_files_load() {
        let tmp = project(&[("hello", &fixture("hello"))]);
        let dir = tmp.path().join(".decree").join(MACHINES_DIR);
        fs::write(dir.join("notes.md"), "not a machine").unwrap();
        fs::write(dir.join("other.yaml"), "not: [valid").unwrap();
        let machines = load_machines(&tmp.path().join(".decree")).unwrap();
        assert_eq!(machines.keys().collect::<Vec<_>>(), ["hello"]);
    }

    #[test]
    fn internal_transition_type() {
        let m = load(
            "m",
            "name: m\ndescription: d\ninitial: a\nstates:\n  a:\n    initial: b\n    \
             transitions: { go: { target: b, type: internal } }\n    states:\n      \
             b: { final: true }\n  failed: { final: true }\n",
        );
        let a = &m.nodes[m.find("a").unwrap()];
        assert!(a.transitions[0].internal);
        assert_eq!(m.state_path(m.find("b").unwrap()), "a.b");
    }

    #[test]
    fn yaml_1_1_booleans_are_strings() {
        // The Norway problem: YAML 1.1 reads `on` and `no` as booleans; machines are YAML 1.2.
        let m = load(
            "m",
            "name: m\ndescription: d\ninitial: a\nstates:\n  a:\n    \
             transitions: { on: no }\n  no: { final: true }\n  failed: { final: true }\n",
        );
        let a = &m.nodes[m.find("a").unwrap()];
        assert_eq!(a.transitions[0].event, "on");
        assert_eq!(a.transitions[0].target, "no");
        assert!(m.find("no").is_some());
    }

    /// Problems for machine `m` given as `text`, other than V12 (no scripts exist here).
    /// `others` are more machines in the project.
    fn problems_with(text: &str, others: &[(&str, &str)]) -> Vec<String> {
        let mut machines = BTreeMap::from([("m".to_string(), load("m", text))]);
        for (id, other) in others {
            machines.insert(id.to_string(), load(id, other));
        }
        let ids: BTreeSet<String> = machines.keys().cloned().collect();
        let tmp = TempDir::new().unwrap();
        let env = CheckEnv {
            decree_dir: tmp.path(),
            machine_ids: &ids,
            machines: &machines,
        };
        machines["m"]
            .validate(text, &env)
            .into_iter()
            .map(|p| format!("{}: {}", p.at, p.message))
            .filter(|p| !p.ends_with("(V12)"))
            .collect()
    }

    fn problems(text: &str) -> Vec<String> {
        problems_with(text, &[("router", ROUTER)])
    }

    const HEAD: &str = "name: m\ndescription: d\ninitial: a\nstates:\n";

    const ROUTER: &str = "name: router\ndescription: d\ninitial: a\nstates:\n  \
                          a: { invoke: x, transitions: { done: done } }\n  \
                          done: { final: true }\n  failed: { final: true }\n";

    #[test]
    fn section5_examples_validate() {
        let texts: Vec<(&str, String)> = EXAMPLES.iter().map(|n| (*n, fixture(n))).collect();
        let machines: BTreeMap<String, LoadedMachine> = texts
            .iter()
            .map(|(n, t)| (n.to_string(), load(n, t)))
            .collect();
        let ids: BTreeSet<String> = machines.keys().cloned().collect();
        let tmp = TempDir::new().unwrap();
        let env = CheckEnv {
            decree_dir: tmp.path(),
            machine_ids: &ids,
            machines: &machines,
        };
        for (name, text) in &texts {
            let found: Vec<Problem> = machines[*name]
                .validate(text, &env)
                .into_iter()
                .filter(|p| !p.message.ends_with("(V12)"))
                .collect();
            assert!(found.is_empty(), "{name}: {found:?}");
        }
    }

    #[test]
    fn event_matching_is_scxml_prefix_matching() {
        assert!(event_matches("done", "done"));
        assert!(event_matches("done", "done.state.work"));
        assert!(event_matches("done.state", "done.state.work"));
        assert!(!event_matches("done", "doner"));
        assert!(!event_matches("done.state.work", "done.state"));
    }

    #[test]
    fn event_names() {
        for ok in ["done", "done.state.work", "a1_b", "yes", "x.0"] {
            assert!(is_event_name(ok), "{ok}");
        }
        for bad in ["", "Done", "1a", "a..b", "a.", ".a", "a-b", "a.B"] {
            assert!(!is_event_name(bad), "{bad}");
        }
    }

    #[test]
    fn duplicate_state_id_across_levels() {
        let text = format!(
            "{HEAD}  a:\n    initial: b\n    transitions: {{ done.state.a: b }}\n    states:\n      \
             b: {{ invoke: x, transitions: {{ done: fin }} }}\n      fin: {{ final: true }}\n  \
             b: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        let found = problems(&text);
        assert!(
            found.contains(&"b: state id `b` is also used by `a.b`; state ids are unique across the machine (V2)".to_string()),
            "{found:?}"
        );
    }

    #[test]
    fn compound_initial_must_be_a_direct_child() {
        let text = format!(
            "{HEAD}  a:\n    initial: c\n    transitions: {{ done.state.a: done }}\n    states:\n      \
             b:\n        initial: c\n        transitions: {{ done.state.b: done }}\n        states:\n          \
             c: {{ final: true }}\n  done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        let found = problems(&text);
        assert!(
            found.contains(&"a: initial `c` is not a direct child state (V3)".to_string()),
            "{found:?}"
        );
    }

    #[test]
    fn initial_without_states() {
        let text = format!(
            "{HEAD}  a: {{ initial: b, invoke: x, transitions: {{ done: done }} }}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        assert_eq!(
            problems(&text),
            ["a: `initial` without `states`: they are present together on compound states (V6)"]
        );
    }

    // V8: decision and sub-machine states cover their events.

    #[test]
    fn check_must_handle_yes_and_no() {
        let text = format!(
            "{HEAD}  a: {{ invoke: {{ check: {{ visits: a, less_than: 2 }} }}, transitions: {{ yes: done }} }}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        assert_eq!(
            problems(&text),
            ["a: a `check` state must handle `no`, itself or through an ancestor (V8)"]
        );
    }

    #[test]
    fn choose_needs_a_question_and_described_options() {
        let text = format!(
            "{HEAD}  a:\n    invoke: {{ choose: person, ask: tell }}\n    transitions:\n      \
             ship: {{ target: done, description: Ship it. }}\n      stop: done\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        assert_eq!(
            problems(&text),
            [
                "a: a `choose: person` state needs a `question`: what is being decided (V8)",
                "a: option `stop` needs a `description`: write it as `stop: { target: done, description: ... }` (V8)",
            ]
        );
    }

    #[test]
    fn choose_needs_two_options_and_min_confidence_needs_unsure() {
        let text = format!(
            "{HEAD}  a:\n    invoke: {{ choose: model, question: \"Go?\", min_confidence: 0.5 }}\n    \
             transitions:\n      go: {{ target: done, description: Go. }}\n      error: failed\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        assert_eq!(
            problems(&text),
            [
                "a: a `choose: model` state needs at least 2 options (transitions other than `unsure` and `error`), has 1 (V8)",
                "a: a `choose: model` state with `min_confidence` must handle `unsure`, itself or through an ancestor (V8)",
            ]
        );
    }

    #[test]
    fn machine_state_handles_every_final_state_of_the_child() {
        let child = "name: child\ndescription: d\ninitial: a\nstates:\n  \
                     a: { invoke: x, transitions: { ok: done, no: rejected } }\n  \
                     done: { final: true }\n  rejected: { final: true }\n  failed: { final: true }\n";
        let text = format!(
            "{HEAD}  a: {{ invoke: {{ machine: child }}, transitions: {{ done: done }} }}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        assert_eq!(
            problems_with(&text, &[("child", child)]),
            ["a: machine `child` can end in `rejected`, which this state does not handle, itself or through an ancestor (V8)"]
        );
    }

    // V9: input.

    #[test]
    fn input_names_a_script_state_and_matches_needs_a_script_before_it() {
        let text = format!(
            "{HEAD}  a: {{ invoke: {{ check: {{ matches: ok }} }}, transitions: {{ yes: b, no: b }} }}\n  \
             b: {{ invoke: {{ check: {{ matches: ok }}, input: a }}, transitions: {{ yes: done, no: done }} }}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        assert_eq!(
            problems(&text),
            [
                "a: `matches` without `input` reads the most recent script's output, but no script state comes before this state (V9)",
                "b: input `a` is not a state with a script invoke (V9)",
            ]
        );
    }

    #[test]
    fn matches_after_a_script_state_needs_no_input() {
        let text = format!(
            "{HEAD}  a: {{ invoke: x, transitions: {{ done: b }} }}\n  \
             b: {{ invoke: {{ check: {{ matches: ok }} }}, transitions: {{ yes: done, no: a }} }}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        assert!(problems(&text).is_empty(), "{:?}", problems(&text));
    }

    // V10: conditions.

    #[test]
    fn condition_rules() {
        let text = "name: m\ndescription: d\ndata:\n  max_rounds: { type: int, default: 2 }\n  \
                    mode: { type: string, default: fast }\ninitial: a\nstates:\n  \
                    a: { invoke: x, transitions: { done: b } }\n  \
                    b: { invoke: { check: { visits: w, less_than: { data: rounds } } }, transitions: { yes: c, no: c } }\n  \
                    c: { invoke: { check: { data: mode, less_than: b } }, transitions: { yes: d, no: d } }\n  \
                    d: { invoke: { check: { data: max_rounds, equals: fast } }, transitions: { yes: e, no: e } }\n  \
                    e: { invoke: { check: { matches: '(' } }, transitions: { yes: f, no: f } }\n  \
                    f: { invoke: { check: { visits: a, data: mode, equals: 1 } }, transitions: { yes: g, no: g } }\n  \
                    g: { invoke: { check: { visits: a } }, transitions: { yes: done, no: done } }\n  \
                    w:\n    initial: z\n    transitions: { done.state.w: done }\n    states:\n      z: { final: true }\n  \
                    done: { final: true }\n  failed: { final: true }\n";
        let found: Vec<String> = problems(text)
            .into_iter()
            .filter(|p| p.ends_with("(V10)"))
            .collect();
        assert_eq!(
            found,
            [
                "b: check: `visits` names `w`, which is not an atomic state (V10)",
                "b: check: unknown data `rounds` (V10)",
                "c: check: `less_than` compares ints only, not string (V10)",
                "d: check: `data` compares int with string (V10)",
                "e: check: `matches` '(' is not a regular expression: unclosed group (V10)",
                "f: check: a condition has exactly one subject, not `visits` and `data`; use two `check` states in a row (V10)",
                "g: check: `visits` needs one operator: `equals`, `not_equals`, `less_than`, `at_most`, `more_than` or `at_least` (V10)",
            ]
        );
    }

    // V11: reachability.

    #[test]
    fn unhandled_error_reaches_failed() {
        // a and b loop forever on done, but an invoke can fail, and an unhandled error
        // goes to `failed`, so neither stalls.
        let text = format!(
            "{HEAD}  a: {{ invoke: x, transitions: {{ done: b }} }}\n  \
             b: {{ invoke: x, transitions: {{ done: a }} }}\n  failed: {{ final: true }}\n"
        );
        assert!(problems(&text).is_empty(), "{:?}", problems(&text));
    }

    #[test]
    fn handled_error_that_loops_stalls() {
        let text = format!(
            "{HEAD}  a: {{ invoke: x, transitions: {{ done: a, error: a }} }}\n  \
             failed: {{ final: true }}\n"
        );
        assert_eq!(
            problems(&text),
            ["a: state cannot reach a root-level final state (V11)"]
        );
    }

    #[test]
    fn a_check_loop_without_a_way_out_stalls() {
        let text = format!(
            "{HEAD}  a: {{ invoke: {{ check: {{ visits: a, less_than: 3 }} }}, transitions: {{ yes: a, no: a }} }}\n  \
             failed: {{ final: true }}\n"
        );
        assert_eq!(
            problems(&text),
            ["a: state cannot reach a root-level final state (V11)"]
        );
    }

    #[test]
    fn unreachable_compound_is_reported_once() {
        let text = format!(
            "{HEAD}  a: {{ invoke: x, transitions: {{ done: done }} }}\n  \
             w:\n    initial: z\n    transitions: {{ done.state.w: done }}\n    states:\n      \
             z: {{ final: true }}\n  done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        assert_eq!(
            problems(&text),
            ["w: state is unreachable from the root `initial` (V11)"]
        );
    }

    // V15: done.state.<id>.

    #[test]
    fn nested_final_needs_a_done_state_handler() {
        let text = format!(
            "{HEAD}  a:\n    initial: b\n    transitions: {{ done.state.a: done }}\n    states:\n      \
             b: {{ transitions: {{ done: c }} }}\n      c: {{ final: true }}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        assert!(problems(&text).is_empty(), "{:?}", problems(&text));

        // Without a handler, `done.state.a` becomes `error`, so the run fails instead.
        let stalled = text.replace(
            "transitions: { done.state.a: done }",
            "transitions: { go: done }",
        );
        assert_eq!(
            problems(&stalled),
            [
                "done: state is unreachable from the root `initial` (V11)",
                "a: compound state has a final state, but nothing handles `done.state.a`, itself or through an ancestor (V15)",
            ]
        );
    }

    // V16: invokes.

    #[test]
    fn invoke_names_and_values() {
        let child =
            "name: child\ndescription: d\ndata:\n  n: { type: int, default: 1 }\ninitial: a\n\
                     states:\n  a: { invoke: x, transitions: { done: done } }\n  \
                     done: { final: true }\n  failed: { final: true }\n";
        let text = format!(
            "{HEAD}  a: {{ invoke: {{ machine: child, params: {{ n: two, k: 1 }} }}, transitions: {{ done: b }} }}\n  \
             b: {{ invoke: {{ machine: nobody }}, transitions: {{ done: c }} }}\n  \
             c:\n    invoke: {{ choose: model, question: \"Go?\", router: nobody, min_confidence: 1.5 }}\n    \
             max_attempts: 2\n    transitions:\n      \
             go: {{ target: done, description: Go. }}\n      stop: {{ target: done, description: Stop. }}\n      \
             unsure: done\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        let found: Vec<String> = problems_with(&text, &[("child", child)])
            .into_iter()
            .filter(|p| p.ends_with("(V16)"))
            .collect();
        assert_eq!(
            found,
            [
                "a: param `n` must be of type `int` (V16)",
                "a: unknown param `k`: machine `child` has no data `k` (V16)",
                "b: machine `nobody` does not exist (V16)",
                "c: router `nobody` is not a machine (V16)",
                "c: min_confidence 1.5 is not between 0 and 1 (V16)",
                "c: `max_attempts` is only allowed on states that invoke a script (V16)",
            ]
        );
    }

    #[test]
    fn choose_model_without_router_needs_a_machine_named_router() {
        let text = format!(
            "{HEAD}  a:\n    invoke: {{ choose: model, question: \"Go?\" }}\n    transitions:\n      \
             go: {{ target: done, description: Go. }}\n      stop: {{ target: done, description: Stop. }}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        assert!(problems(&text).is_empty(), "{:?}", problems(&text));
        assert_eq!(
            problems_with(&text, &[]),
            ["a: `choose: model` names no `router`, and there is no machine named `router` (V16)"]
        );
    }

    // V17: type: internal.

    #[test]
    fn internal_only_from_a_compound_state_to_a_descendant() {
        let text = format!(
            "{HEAD}  a: {{ invoke: x, transitions: {{ done: {{ target: done, type: internal }} }} }}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        assert_eq!(
            problems(&text),
            ["a: transition `done`: `type: internal` is only allowed from a compound state to one of its descendants (V17)"]
        );
    }

    // V18: event names.

    #[test]
    fn event_names_and_reserved_options() {
        let text = format!(
            "{HEAD}  a: {{ invoke: x, transitions: {{ Done: b, done: b }} }}\n  \
             b:\n    invoke: {{ choose: person, question: \"Go?\", ask: tell }}\n    transitions:\n      \
             go: {{ target: done, description: Go. }}\n      done.later: {{ target: done, description: Later. }}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        assert_eq!(
            problems(&text),
            [
                "a: event `Done` does not match ^[a-z][a-z0-9_]*(\\.[a-z0-9_]+)*$ (V18)",
                "b: option `done.later` is reserved: `done`, `error`, `unsure` and names starting with `done.` or `error.` cannot be options (V18)",
            ]
        );
    }

    // V19 inside an invoke.

    #[test]
    fn choose_keys_of_the_other_kind() {
        let text = format!(
            "{HEAD}  a:\n    invoke: {{ choose: person, question: \"Go?\", ask: tell, router: router, input: a }}\n    \
             transitions:\n      go: {{ target: done, description: Go. }}\n      \
             stop: {{ target: done, description: Stop. }}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        let found: Vec<String> = problems(&text)
            .into_iter()
            .filter(|p| p.ends_with("(V19)"))
            .collect();
        assert_eq!(
            found,
            ["a: `choose: person` does not take `router`, `input` (V19)"]
        );
    }

    // V20: invoke cycles.

    #[test]
    fn a_machine_that_invokes_itself() {
        let text = format!(
            "{HEAD}  a: {{ invoke: {{ machine: m }}, transitions: {{ done: done }} }}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        assert_eq!(
            problems(&text),
            ["a: machine `m` invokes itself: m -> m; a machine never invokes itself, directly or through others (V20)"]
        );
    }

    #[test]
    fn a_machine_that_invokes_itself_through_its_router() {
        let router = "name: router\ndescription: d\ninitial: a\nstates:\n  \
                      a: { invoke: { machine: m }, transitions: { done: done } }\n  \
                      done: { final: true }\n  failed: { final: true }\n";
        let text = format!(
            "{HEAD}  a:\n    invoke: {{ choose: model, question: \"Go?\" }}\n    transitions:\n      \
             go: {{ target: done, description: Go. }}\n      stop: {{ target: done, description: Stop. }}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        assert_eq!(
            problems_with(&text, &[("router", router)]),
            ["a: machine `m` invokes itself: m -> router -> m; a machine never invokes itself, directly or through others (V20)"]
        );
    }

    // V21: overlapping events in one state.

    #[test]
    fn overlapping_events_in_one_state() {
        let text = format!(
            "{HEAD}  a:\n    initial: b\n    transitions: {{ done: done, done.state.a: done }}\n    states:\n      \
             b: {{ invoke: x, transitions: {{ done: c, done_later: c }} }}\n      \
             c: {{ final: true }}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        assert_eq!(
            problems(&text),
            ["a: events `done` and `done.state.a` overlap: `done` also matches `done.state.a`, so at most one transition per state may match an event (V21)"]
        );
    }

    #[test]
    fn data_types() {
        use serde_norway::Value;
        assert!(DataType::Int.matches(&Value::from(3)));
        assert!(!DataType::Int.matches(&Value::from("3")));
        assert!(!DataType::Int.matches(&Value::from(1.5)));
        assert!(DataType::String.matches(&Value::from("on")));
        assert!(!DataType::String.matches(&Value::from(true)));
        assert!(DataType::Bool.matches(&Value::from(false)));
        assert!(!DataType::Bool.matches(&Value::from("no")));
    }

    #[test]
    fn script_problems_name_the_state_and_relative_path() {
        let text = format!(
            "{HEAD}  a: {{ invoke: x, transitions: {{ done: b }} }}\n  \
             b:\n    invoke: {{ choose: person, question: \"Go?\" }}\n    transitions:\n      \
             go: {{ target: done, description: Go. }}\n      stop: {{ target: done, description: Stop. }}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        let machines = BTreeMap::from([("m".to_string(), load("m", &text))]);
        let tmp = TempDir::new().unwrap();
        let ids = BTreeSet::new();
        let env = CheckEnv {
            decree_dir: tmp.path(),
            machine_ids: &ids,
            machines: &machines,
        };
        let found = machines["m"].validate(&text, &env);
        assert_eq!(
            found,
            [
                Problem {
                    at: "b".into(),
                    message: "a `choose: person` state needs an `ask` script, which tells someone how to reply (V12)".into(),
                },
                Problem {
                    at: "a".into(),
                    message: "script `x` not found; searched scripts/m, scripts (V12)".into(),
                },
            ]
        );
    }
}
