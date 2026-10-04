//! Machines: SCXML statecharts written as YAML (docs/reference/machines.md).
//!
//! `load_machines` reads `.decree/machines/*.yml` and flattens each machine into an arena of `Node`s. The interpreter, validator and graph
//! exporter work on the arena, never on the raw structs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

use crate::cond::Condition;
use crate::error::DecreeError;

pub(crate) mod validate;
/// Directory holding machine files, relative to `.decree/`.
pub const MACHINES_DIR: &str = "machines";

/// Root-level final state an unhandled `error` goes to (V5).
pub const FAILED: &str = "failed";

/// Events a `model` or `person` state handles beside its options (docs/reference/machines.md, Choices).
const NOT_OPTIONS: [&str; 2] = ["unsure", "error"];

/// A machine file: one SCXML document (`<scxml>`), written as YAML.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Machine {
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
struct State {
    #[serde(default, rename = "final")]
    pub is_final: bool,
    pub description: Option<String>,
    pub invoke: Option<Invoke>,
    #[serde(default)]
    pub onentry: Vec<String>,
    #[serde(default)]
    pub onexit: Vec<String>,
    pub initial: Option<String>,
    #[serde(default)]
    pub states: BTreeMap<String, State>,
    #[serde(default)]
    pub transitions: BTreeMap<EventName, Transition>,
    #[serde(default)]
    pub emits: Vec<String>,
}

/// A state's function, SCXML `<invoke type>` (docs/reference/machines.md, Invoke): a map with
/// exactly one key, which names the kind, as serde's externally tagged enums. A bare string is
/// short for `{ script: <name> }`.
#[derive(Debug, Clone, PartialEq)]
pub enum Invoke {
    /// `decree:script`
    Script(ScriptInvoke),
    /// `decree:check`, boxed: a condition is much larger than the other kinds.
    Check(Box<Condition>),
    /// `decree:model`
    Model(ModelInvoke),
    /// `decree:person`
    Person(PersonInvoke),
    /// SCXML's own type: a child state machine.
    Machine(MachineInvoke),
}

/// `{ script: { name, max_attempts?, timeout_s? } }`, or `{ script: <name> }`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptInvoke {
    pub name: String,
    pub max_attempts: Option<u32>,
    pub timeout_s: Option<u64>,
}

/// `{ model: { question, router?, min_confidence?, output? } }`. `question` is optional
/// here so that V8, not the parser, reports it missing.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelInvoke {
    pub question: Option<String>,
    pub router: Option<String>,
    pub min_confidence: Option<f64>,
    pub output: Option<String>,
}

/// `{ person: { question, ask, timeout_s? } }`. `question` and `ask` are optional here so
/// that V8 and V12, not the parser, report them missing.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonInvoke {
    pub question: Option<String>,
    pub ask: Option<String>,
    pub timeout_s: Option<u64>,
}

/// `{ machine: { name, params? } }`, or `{ machine: <name> }`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineInvoke {
    pub name: String,
    #[serde(default)]
    pub params: serde_norway::Mapping,
}

/// The kinds `invoke` names, in the order the reference lists them.
const INVOKE_KINDS: &str = "`script`, `check`, `model`, `person` or `machine`";

/// A `script` or `machine` invoke's value: a bare name is short for `{ name: <name> }`.
fn named<T: serde::de::DeserializeOwned>(value: serde_norway::Value) -> Result<T, String> {
    let value = match value {
        serde_norway::Value::String(name) => {
            let mut map = serde_norway::Mapping::new();
            map.insert("name".into(), name.into());
            serde_norway::Value::Mapping(map)
        }
        other => other,
    };
    serde_norway::from_value(value).map_err(|e| e.to_string())
}

impl<'de> Deserialize<'de> for Invoke {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_norway::Value::deserialize(deserializer)?;
        let map = match value {
            serde_norway::Value::String(name) => {
                return Ok(Invoke::Script(ScriptInvoke {
                    name,
                    max_attempts: None,
                    timeout_s: None,
                }))
            }
            serde_norway::Value::Mapping(map) if map.len() == 1 => map,
            _ => {
                return Err(D::Error::custom(format!(
                    "`invoke` is a script name or a map with exactly one key, which names the kind: {INVOKE_KINDS}"
                )))
            }
        };
        let (kind, value) = map.into_iter().next().unwrap_or_default();
        let result = match kind.as_str().unwrap_or_default() {
            "script" => named(value).map(Invoke::Script),
            "check" => serde_norway::from_value(value)
                .map(|c| Invoke::Check(Box::new(c)))
                .map_err(|e| e.to_string()),
            "model" => serde_norway::from_value(value)
                .map(Invoke::Model)
                .map_err(|e| e.to_string()),
            "person" => serde_norway::from_value(value)
                .map(Invoke::Person)
                .map_err(|e| e.to_string()),
            "machine" => named(value).map(Invoke::Machine),
            other => Err(format!(
                "unknown invoke kind `{other}`: `invoke` names one of {INVOKE_KINDS}"
            )),
        };
        result.map_err(D::Error::custom)
    }
}

impl Invoke {
    /// The script invoke, if this is one.
    pub fn script(&self) -> Option<&ScriptInvoke> {
        match self {
            Invoke::Script(script) => Some(script),
            _ => None,
        }
    }

    /// The state whose output a `check` or `model` reads (docs/reference/machines.md, Output).
    fn output(&self) -> Option<&str> {
        match self {
            Invoke::Check(c) => c.output.as_deref(),
            Invoke::Model(m) => m.output.as_deref(),
            _ => None,
        }
    }

    /// The `question` of a `model` or `person` invoke: the kind's name and the question.
    pub fn question(&self) -> Option<(&'static str, Option<&str>)> {
        match self {
            Invoke::Model(m) => Some(("model", m.question.as_deref())),
            Invoke::Person(p) => Some(("person", p.question.as_deref())),
            _ => None,
        }
    }
}

/// A transition's event name. YAML 1.2 reads `true:` and `false:` as booleans; in
/// `transitions` they are the event names `true` and `false`, the events of a `check`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct EventName(String);

impl<'de> Deserialize<'de> for EventName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match serde_norway::Value::deserialize(deserializer)? {
            serde_norway::Value::String(name) => Ok(EventName(name)),
            serde_norway::Value::Bool(b) => Ok(EventName(b.to_string())),
            other => Err(D::Error::custom(format!(
                "an event name is a string, not {}",
                serde_norway::to_string(&other)
                    .unwrap_or_default()
                    .trim_end()
            ))),
        }
    }
}

/// An SCXML `<transition event target type>`: `event: target` or the long form.
#[derive(Debug)]
enum Transition {
    Short(String),
    Long(LongTransition),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LongTransition {
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
enum TransitionType {
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
fn flatten(id: &str, machine: Machine) -> LoadedMachine {
    let root = Node {
        id: machine.name,
        parent: None,
        depth: 0,
        children: Vec::new(),
        is_final: false,
        description: Some(machine.description),
        invoke: None,
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
            onentry: state.onentry,
            onexit: state.onexit,
            initial: state.initial,
            transitions: state
                .transitions
                .into_iter()
                .map(|(event, t)| Edge::new(event.0, t))
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
        let machine = load_machine_text(&id, &std::fs::read_to_string(path)?)?;
        machines.insert(id, machine);
    }
    Ok(machines)
}

/// The file each machine id loads from: every `*.yml` file in `machines/`, by stem.
pub fn machine_paths(decree_dir: &Path) -> Result<BTreeMap<String, PathBuf>, DecreeError> {
    let entries = match std::fs::read_dir(decree_dir.join(MACHINES_DIR)) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(e.into()),
    };
    let mut paths = BTreeMap::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "yml") && path.is_file() {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                paths.insert(stem.to_string(), path);
            }
        }
    }
    Ok(paths)
}

/// Parse and flatten machine `id`, whose file contents are `text`.
pub fn load_machine_text(id: &str, text: &str) -> Result<LoadedMachine, DecreeError> {
    let machine = parse_machine(text)
        .map_err(|e| DecreeError::Other(format!("{MACHINES_DIR}/{id}.yml: {e}")))?;
    Ok(flatten(id, machine))
}

/// Keys outside the SCXML subset that a state may be written with, and what to use instead
/// (V19). SCXML elements are named as such; the rest are decree 0.5 drafts that the
/// decision invokes replaced, and script settings that moved inside the script invoke.
const UNSUPPORTED_STATE_KEYS: [(&str, &str); 15] = [
    ("router", "router on a state is not supported: make the decision a state with invoke: { model: { question: ... } }"),
    ("default", "default on a state is not supported: a model state takes unsure, or error, instead"),
    ("max_attempts", "max_attempts on a state is not supported: write it inside the script invoke, invoke: { script: { name: <script>, max_attempts: <n> } }"),
    ("timeout_s", "timeout_s on a state is not supported: write it inside the invoke, invoke: { script: { name: <script>, timeout_s: <n> } } (or person: { ..., timeout_s: <n> })"),
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

/// Old `invoke` shapes and their replacements (V19): `choose`, `input`, `{ machine, params }`
/// and a bare `matches`. Nothing old is read: each fails with the shape to write instead.
const CHOOSE: &str = "choose is not supported: write invoke: { model: { question: ... } } for a model, or invoke: { person: { question: ..., ask: <script> } } for a person";
const INPUT: &str = "input is not supported: name the state whose output is read with output, in the condition ({ output: <state>, matches: ... }) or in the model ({ model: { ..., output: <state> } })";
const MACHINE_PARAMS: &str = "{ machine: <name>, params: ... } is not supported: write invoke: { machine: { name: <name>, params: ... } }";
const BARE_MATCHES: &str =
    "a bare matches is not supported: name the state it reads, { output: <state>, matches: ... }";

/// The first old shape in a state's `invoke`, if any (V19).
fn old_invoke_shape(invoke: &serde_norway::Value) -> Option<&'static str> {
    let map = invoke.as_mapping()?;
    let has = |key: &str| map.contains_key(key);
    if has("choose") {
        return Some(CHOOSE);
    }
    let model_input = invoke
        .get("model")
        .is_some_and(|m| m.get("input").is_some());
    if has("input") || model_input {
        return Some(INPUT);
    }
    if has("machine") && map.len() > 1 {
        return Some(MACHINE_PARAMS);
    }
    let bare_matches = invoke.get("check").is_some_and(|c| {
        c.get("matches").is_some()
            && !["output", "data", "visits", "confidence"]
                .iter()
                .any(|subject| c.get(subject).is_some())
    });
    bare_matches.then_some(BARE_MATCHES)
}

/// The docs/reference/machines.md message for `cond` on a transition (V19).
const COND_ON_TRANSITION: &str =
    "cond on a transition is not supported: make the decision a state with invoke: { check: ... }";

/// Parse machine YAML. On failure the error starts with the dotted path of the state that
/// fails to deserialize (`work.implement: unknown field ...`), or with `line <n>` when the
/// problem is in the YAML syntax or at the root. A key outside the SCXML subset is reported
/// with its decree alternative, as is an old shape with the one that replaced it, and every
/// unknown key is tagged `(V19)`.
fn parse_machine(text: &str) -> Result<Machine, String> {
    let at_line = |e: &serde_norway::Error, msg: String| match e.location() {
        Some(loc) => format!("line {}: {msg}", loc.line()),
        None => msg,
    };
    let value: serde_norway::Value = match serde_norway::from_str(text) {
        Ok(value) => value,
        Err(syntax) => return Err(at_line(&syntax, syntax.to_string())),
    };
    // Before parsing: an old shape may also parse as a new one (a bare `matches`).
    if let Some(found) = value
        .get("states")
        .and_then(|states| unsupported_key(states, ""))
    {
        return Err(found);
    }
    let err = match serde_norway::from_str::<Machine>(text) {
        Ok(machine) => return Ok(machine),
        Err(e) => e,
    };
    if let Some(states) = value.get("states") {
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

/// An unknown key, or an unknown invoke kind, is outside the SCXML subset (V19).
fn tag_unknown(msg: String) -> String {
    if msg.contains("unknown field") || msg.contains("unknown invoke kind") {
        format!("{msg} (V19)")
    } else {
        msg
    }
}

/// The first state key, transition `cond` or old invoke shape outside the SCXML subset:
/// `<path>: <message>`.
fn unsupported_key(states: &serde_norway::Value, prefix: &str) -> Option<String> {
    for (key, state) in states.as_mapping()? {
        let path = join_path(prefix, key);
        for (name, message) in UNSUPPORTED_STATE_KEYS {
            if state.get(name).is_some() {
                return Some(format!("{path}: {message} (V19)"));
            }
        }
        if let Some(message) = state.get("invoke").and_then(old_invoke_shape) {
            return Some(format!("{path}: {message} (V19)"));
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

/// The router of a `model` invoke that names none (docs/reference/runs.md, The default router).
pub const ROUTER_MACHINE: &str = "router";

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

/// Events an invoke may not print and a `model` or `person` option may not be (docs/reference/machines.md, Rules).
pub fn is_reserved_event(event: &str) -> bool {
    matches!(event, "done" | "error" | "unsure")
        || event.starts_with("done.")
        || event.starts_with("error.")
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

    /// Attempts allowed for state `i`: its script invoke's `max_attempts`, default 1
    /// (docs/reference/scripts.md, Execution, Attempts).
    pub fn max_attempts(&self, i: usize) -> u32 {
        self.nodes[i]
            .invoke
            .as_ref()
            .and_then(Invoke::script)
            .and_then(|s| s.max_attempts)
            .unwrap_or(1)
            .max(1)
    }

    /// A final state whose parent is the root: entering it ends the run.
    pub fn is_root_final(&self, i: usize) -> bool {
        self.nodes[i].is_final && self.nodes[i].parent == Some(0)
    }

    /// The transition domain (docs/reference/machines.md, Rules): the deepest compound state,
    /// or the root, that is a proper ancestor of both the state declaring the transition and
    /// its target. For a `type: internal` transition from a compound state to one of its
    /// descendants, it is the source itself, so the source is neither exited nor re-entered.
    pub fn transition_domain(&self, source: usize, target: usize, internal: bool) -> usize {
        if internal && self.is_compound(source) && self.chain(target).skip(1).any(|a| a == source) {
            return source;
        }
        self.chain(source)
            .skip(1)
            .find(|&a| self.chain(target).skip(1).any(|b| b == a))
            .unwrap_or(0)
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

    /// The options of `model` or `person` state `i` (docs/reference/machines.md, Choices): its own transitions except
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
    fn final_events(&self) -> Vec<&str> {
        self.nodes[0]
            .children
            .iter()
            .map(|&c| &self.nodes[c])
            .filter(|n| n.is_final && n.id != FAILED)
            .map(|n| n.id.as_str())
            .collect()
    }

    /// `(state, machine)` for every machine this one runs as a child: `machine` invokes, and
    /// the router of each `model` invoke (its `router`, else [`ROUTER_MACHINE`]).
    pub fn invoked_machines(&self) -> Vec<(usize, &str)> {
        let mut out = Vec::new();
        for (i, node) in self.nodes.iter().enumerate() {
            let child = match &node.invoke {
                Some(Invoke::Machine(m)) => Some(m.name.as_str()),
                Some(Invoke::Model(c)) => Some(c.router.as_deref().unwrap_or(ROUTER_MACHINE)),
                _ => None,
            };
            out.extend(child.map(|c| (i, c)));
        }
        out
    }
}

#[cfg(test)]
mod tests;
