//! Machines: SCXML statecharts written as YAML (spec section 5).
//!
//! `load_machines` reads `machines/*.yml` project-local first, then from `shared_source`,
//! and flattens each machine into an arena of `Node`s. The interpreter, validator and graph
//! exporter work on the arena, never on the raw structs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::cond::{self, is_ident};
use crate::error::DecreeError;
use crate::runtime::resolve_script;

/// Directory holding machine files, relative to `.decree/` or `shared_source`.
pub const MACHINES_DIR: &str = "machines";

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
    pub invoke: Option<String>,
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
    pub router: Option<RouterKind>,
    pub default: Option<String>,
    #[serde(default)]
    pub emits: Vec<String>,
}

/// Extension: `router: llm` makes a router state (section 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RouterKind {
    Llm,
}

/// An SCXML `<transition event target cond type>`: `event: target` or the long form.
#[derive(Debug, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Transition {
    Short(String),
    Long {
        target: String,
        description: Option<String>,
        cond: Option<String>,
        /// Only `internal`; omitted means external.
        #[serde(rename = "type")]
        kind: Option<TransitionType>,
    },
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
    pub cond: Option<String>,
    pub internal: bool,
}

impl Edge {
    fn new(event: String, transition: Transition) -> Self {
        match transition {
            Transition::Short(target) => Edge {
                event,
                target,
                description: None,
                cond: None,
                internal: false,
            },
            Transition::Long {
                target,
                description,
                cond,
                kind,
            } => Edge {
                event,
                target,
                description,
                cond,
                internal: kind == Some(TransitionType::Internal),
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
    pub invoke: Option<String>,
    pub max_attempts: Option<u32>,
    pub timeout_s: Option<u64>,
    pub onentry: Vec<String>,
    pub onexit: Vec<String>,
    pub initial: Option<String>,
    /// In event-name order.
    pub transitions: Vec<Edge>,
    pub router: Option<RouterKind>,
    pub default: Option<String>,
    pub emits: Vec<String>,
}

/// A loaded machine: its `data` and its arena.
#[derive(Debug)]
pub struct LoadedMachine {
    /// The file stem, which V1 requires to equal `name`.
    pub id: String,
    /// The file it was read from.
    pub path: PathBuf,
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
pub fn flatten(id: &str, path: PathBuf, machine: Machine) -> LoadedMachine {
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
        router: None,
        default: None,
        emits: Vec::new(),
    };
    let mut nodes = vec![root];
    push_children(&mut nodes, 0, machine.states);
    LoadedMachine {
        id: id.to_string(),
        path,
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
            router: state.router,
            default: state.default,
            emits: state.emits,
        });
        push_children(nodes, index, state.states);
    }
}

/// Load every machine: `<decree_dir>/machines/*.yml`, then `<shared_source>/machines/*.yml`.
/// A project-local machine hides a shared one with the same id (section 3); the hidden file
/// is not read. A missing `machines/` directory holds no machines.
pub fn load_machines(
    decree_dir: &Path,
    shared_source: Option<&Path>,
) -> Result<BTreeMap<String, LoadedMachine>, DecreeError> {
    let mut machines = BTreeMap::new();
    for (id, path) in machine_paths(decree_dir, shared_source)? {
        let machine = load_machine_file(&id, &path)?;
        machines.insert(id, machine);
    }
    Ok(machines)
}

/// The file each machine id loads from: project-local first, then `shared_source`.
pub fn machine_paths(
    decree_dir: &Path,
    shared_source: Option<&Path>,
) -> Result<BTreeMap<String, PathBuf>, DecreeError> {
    let mut paths = BTreeMap::new();
    for base in std::iter::once(decree_dir).chain(shared_source) {
        for (id, path) in machine_files(&base.join(MACHINES_DIR))? {
            paths.entry(id).or_insert(path);
        }
    }
    Ok(paths)
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
    load_machine_text(id, path, &text)
}

/// Parse and flatten machine `id`, read from `path`, whose contents are `text`.
pub fn load_machine_text(id: &str, path: &Path, text: &str) -> Result<LoadedMachine, DecreeError> {
    let machine = parse_machine(text)
        .map_err(|e| DecreeError::Other(format!("{MACHINES_DIR}/{id}.yml: {e}")))?;
    Ok(flatten(id, path.to_path_buf(), machine))
}

/// Parse machine YAML. On failure the error starts with the dotted path of the state that
/// fails to deserialize (`work.implement: unknown field ...`), or with `line <n>` when the
/// problem is in the YAML syntax or at the root.
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
        if let Some((path, msg)) = locate_state_error(states, "") {
            return Err(format!("{path}: {msg}"));
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
    Err(at_line(&err, msg))
}

/// Find the outermost state whose own keys fail to deserialize, checking each state with its
/// child `states` removed, then descending. Returns its dotted path and the serde message.
fn locate_state_error(states: &serde_norway::Value, prefix: &str) -> Option<(String, String)> {
    let map = states.as_mapping()?;
    for (key, value) in map {
        let id = match key.as_str() {
            Some(id) => id.to_string(),
            None => serde_norway::to_string(key)
                .unwrap_or_default()
                .trim_end()
                .to_string(),
        };
        let path = if prefix.is_empty() {
            id
        } else {
            format!("{prefix}.{id}")
        };
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
// Validation (section 5, Validation): rules V1–V14 on the arena
// =================================================================

/// Everything outside the machine file that validation reads.
pub struct CheckEnv<'a> {
    pub decree_dir: &'a Path,
    pub shared_source: Option<&'a Path>,
    /// Ids of every machine file, including ones that fail to load (V13).
    pub machine_ids: &'a BTreeSet<String>,
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

/// SCXML event matching: descriptor `d` matches `event` if they are equal, or `event`
/// extends `d` after a `.` (`done.state` matches `done.state.work`).
pub fn event_matches(descriptor: &str, event: &str) -> bool {
    event
        .strip_prefix(descriptor)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

/// Root-level state that an unhandled `error` goes to (V5).
const FAILED: &str = "failed";

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

    /// Whether `error` can be raised while `i` is entered or active: its invoke fails,
    /// a waiting state times out, or an `onentry` script on the way in fails.
    fn can_error(&self, i: usize) -> bool {
        let node = &self.nodes[i];
        node.invoke.is_some()
            || node.timeout_s.is_some()
            || self.chain(i).any(|n| !self.nodes[n].onentry.is_empty())
    }

    /// Atomic and final states the run can move to from atomic or final state `i`
    /// (section 5, Rules): any transition of `i` or an ancestor, `done.state.<parent>` from
    /// a nested final state, and the implicit `failed` for an unhandled `error`.
    fn successors(&self, i: usize) -> Vec<usize> {
        let node = &self.nodes[i];
        let mut targets: Vec<&str> = Vec::new();
        let raise_from = if node.is_final {
            match node.parent {
                Some(p) if p != 0 => {
                    let event = format!("done.state.{}", self.nodes[p].id);
                    for n in self.chain(p) {
                        targets.extend(
                            self.nodes[n]
                                .transitions
                                .iter()
                                .filter(|e| event_matches(&e.event, &event))
                                .map(|e| e.target.as_str()),
                        );
                    }
                    !node.onentry.is_empty()
                }
                _ => return Vec::new(),
            }
        } else {
            // Which events the state can see (section 5, Kinds of state): a router picks
            // among its own events; a pass-through takes `done`; an invoke may print, and a
            // waiting state may receive, any event. `error` is resolved like any event.
            let pass_through =
                node.invoke.is_none() && node.router.is_none() && self.handles(i, "done");
            for n in self.chain(i) {
                targets.extend(
                    self.nodes[n]
                        .transitions
                        .iter()
                        .filter(|e| {
                            if node.router.is_some() {
                                n == i || event_matches(&e.event, "error")
                            } else if pass_through {
                                event_matches(&e.event, "done") || event_matches(&e.event, "error")
                            } else {
                                true
                            }
                        })
                        .map(|e| e.target.as_str()),
                );
            }
            self.can_error(i)
        };
        let mut out: Vec<usize> = targets
            .into_iter()
            .filter_map(|t| self.find(t))
            .filter_map(|t| self.enter(t))
            .collect();
        if raise_from && !self.handles(i, "error") {
            out.extend(self.failed_state());
        }
        out
    }

    /// Run V1–V14 on this machine. `text` is the machine file, for root-level line numbers.
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
        v.v8_done();
        v.v9_router();
        v.v10_cond();
        v.v11_reachable();
        v.v12_scripts();
        v.v13_emits();
        v.v14_data();
        v.problems
    }
}

struct Validator<'a> {
    m: &'a LoadedMachine,
    text: &'a str,
    env: &'a CheckEnv<'a>,
    problems: Vec<Problem>,
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
            let node = &self.m.nodes[i];
            for e in &node.transitions {
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
            if let Some(default) = &node.default {
                if !node.transitions.iter().any(|e| e.event == *default) {
                    self.push(
                        self.m.state_path(i),
                        format!("default `{default}` is not one of this state's transitions (V4)"),
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
                let mut found = Vec::new();
                if node.initial.is_none() {
                    self.push(at.clone(), "compound state has no `initial` (V6)".into());
                }
                if node.invoke.is_some() {
                    found.push("invoke");
                }
                if node.router.is_some() {
                    found.push("router");
                }
                if node.default.is_some() {
                    found.push("default");
                }
                if !found.is_empty() {
                    self.push(
                        at,
                        format!(
                            "compound state may not have {} (V6)",
                            found
                                .iter()
                                .map(|k| format!("`{k}`"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    );
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
                ("router", node.router.is_some()),
                ("default", node.default.is_some()),
            ]
            .into_iter()
            .filter_map(|(key, present)| present.then_some(key))
            .collect();
            if !found.is_empty() {
                self.push(
                    self.m.state_path(i),
                    format!(
                        "final state may only have `final`, `description`, `onentry` and `emits`, not {} (V7)",
                        found
                            .iter()
                            .map(|k| format!("`{k}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                );
            }
        }
    }

    fn v8_done(&mut self) {
        for i in self.states() {
            let node = &self.m.nodes[i];
            if node.is_final
                || self.m.is_compound(i)
                || node.invoke.is_none()
                || node.router.is_some()
            {
                continue;
            }
            if !self.m.handles(i, "done") {
                self.push(
                    self.m.state_path(i),
                    "invoke state does not handle `done`, itself or through an ancestor (V8)"
                        .into(),
                );
            }
        }
    }

    fn v9_router(&mut self) {
        for i in self.states() {
            let node = &self.m.nodes[i];
            let at = self.m.state_path(i);
            if node.router.is_none() {
                if node.default.is_some() && !node.is_final && !self.m.is_compound(i) {
                    self.push(at, "`default` is only allowed on router states (V9)".into());
                }
                continue;
            }
            if node.transitions.iter().any(|e| e.event == "done") {
                self.push(
                    at.clone(),
                    "router state may not declare `done` (V9)".into(),
                );
            }
            let options: Vec<&Edge> = node
                .transitions
                .iter()
                .filter(|e| e.event != "error")
                .collect();
            if options.len() < 2 {
                self.push(
                    at.clone(),
                    format!(
                        "router state needs at least 2 events other than `error`, has {} (V9)",
                        options.len()
                    ),
                );
            }
            for e in options {
                if e.description.is_none() {
                    self.push(
                        at.clone(),
                        format!(
                            "router event `{}` needs the long form with a `description` (V9)",
                            e.event
                        ),
                    );
                }
            }
            if node.default.is_none() {
                self.push(
                    at.clone(),
                    "router state needs a `default` event (V9)".into(),
                );
            }
            if node.description.is_none() {
                self.push(at, "router state needs a `description` (V9)".into());
            }
        }
    }

    fn v10_cond(&mut self) {
        for i in self.states() {
            let node = &self.m.nodes[i];
            for e in &node.transitions {
                let Some(text) = &e.cond else {
                    continue;
                };
                let at = self.m.state_path(i);
                if node.router.is_none() {
                    self.push(
                        at,
                        format!(
                            "transition `{}`: `cond` is only allowed on router states (V10)",
                            e.event
                        ),
                    );
                    continue;
                }
                if node.default.as_deref() == Some(e.event.as_str()) {
                    self.push(
                        at.clone(),
                        format!("default event `{}` may not have a `cond` (V10)", e.event),
                    );
                }
                let cond = match cond::parse(text) {
                    Ok(cond) => cond,
                    Err(err) => {
                        self.push(
                            at,
                            format!("transition `{}`: cond `{text}`: {err} (V10)", e.event),
                        );
                        continue;
                    }
                };
                for name in cond.data_refs() {
                    if !self.m.data.contains_key(name) {
                        self.push(
                            at.clone(),
                            format!(
                                "transition `{}`: cond `{text}` reads unknown data `{name}` (V10)",
                                e.event
                            ),
                        );
                    }
                }
                for state in cond.visits_refs() {
                    let atomic = self.m.find(state).is_some_and(|s| !self.m.is_compound(s));
                    if !atomic {
                        self.push(
                            at.clone(),
                            format!(
                                "transition `{}`: cond `{text}` reads `visits.{state}`, but `{state}` is not an atomic state (V10)",
                                e.event
                            ),
                        );
                    }
                }
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
            let names = node.invoke.iter().chain(&node.onentry).chain(&node.onexit);
            for name in names {
                uses.push((self.m.state_path(i), name));
            }
        }
        let prefix = format!("{}/", self.env.decree_dir.display());
        for (at, name) in uses {
            if let Err(e) = resolve_script(
                self.env.decree_dir,
                self.env.shared_source,
                &self.m.id,
                name,
            ) {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    const EXAMPLES: [&str; 3] = ["hello", "deploy", "feature"];

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
        load_machines(&tmp.path().join(".decree"), None)
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn section5_examples_load() {
        let files: Vec<(&str, String)> = EXAMPLES.iter().map(|n| (*n, fixture(n))).collect();
        let refs: Vec<(&str, &str)> = files.iter().map(|(n, t)| (*n, t.as_str())).collect();
        let tmp = project(&refs);
        let machines = load_machines(&tmp.path().join(".decree"), None).unwrap();
        assert_eq!(
            machines.keys().map(String::as_str).collect::<Vec<_>>(),
            ["deploy", "feature", "hello"]
        );
        for (id, m) in &machines {
            assert_eq!(&m.root().id, id);
            assert!(!m.description().is_empty());
        }
        assert_eq!(machines["hello"].nodes.len(), 1 + 3);
        assert_eq!(machines["deploy"].nodes.len(), 1 + 6);
    }

    #[test]
    fn feature_arena_has_nine_states_plus_root() {
        let tmp = project(&[("feature", &fixture("feature"))]);
        let machines = load_machines(&tmp.path().join(".decree"), None).unwrap();
        let m = &machines["feature"];
        assert_eq!(m.nodes.len(), 9 + 1);

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
                "verified",
                "verify",
            ]
        );

        let work = m.find("work").unwrap();
        assert_eq!(m.nodes[work].parent, Some(0));
        assert_eq!(m.nodes[work].depth, 1);
        assert_eq!(m.nodes[work].initial.as_deref(), Some("implement"));
        assert_eq!(m.nodes[work].children.len(), 4);
        assert_eq!(m.nodes[work].transitions[0].event, "done.state.work");

        let verify = m.find("verify").unwrap();
        let node = &m.nodes[verify];
        assert_eq!(node.parent, Some(work));
        assert_eq!(node.depth, 2);
        assert_eq!(m.state_path(verify), "work.verify");
        assert_eq!(node.router, Some(RouterKind::Llm));
        assert_eq!(node.default.as_deref(), Some("ask"));
        let retry = node
            .transitions
            .iter()
            .find(|e| e.event == "retry")
            .unwrap();
        assert_eq!(retry.target, "implement");
        assert_eq!(
            retry.cond.as_deref(),
            Some("visits.implement < data.max_rounds")
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
        assert_eq!(m.nodes[m.find("review").unwrap()].timeout_s, Some(172800));
    }

    #[test]
    fn misspelled_state_key_names_file_and_state_path() {
        let text = fixture("feature").replace("max_attempts: 2", "max_attempt: 2");
        let err = load_err("feature", &text);
        assert!(
            err.starts_with("machines/feature.yml: work.implement: unknown field `max_attempt`"),
            "{err}"
        );
    }

    #[test]
    fn misspelled_top_level_state_key() {
        let text = fixture("hello").replace("invoke: greet", "invokes: greet");
        let err = load_err("hello", &text);
        assert!(
            err.starts_with("machines/hello.yml: greet: unknown field `invokes`"),
            "{err}"
        );
    }

    #[test]
    fn misspelled_transition_key_names_its_state() {
        let text = fixture("feature").replace("cond: \"visits", "condition: \"visits");
        let err = load_err("feature", &text);
        assert!(
            err.starts_with("machines/feature.yml: work.verify: "),
            "{err}"
        );
    }

    #[test]
    fn misspelled_root_key_names_line() {
        let text = fixture("hello").replace("description:", "descripton:");
        let err = load_err("hello", &text);
        assert!(
            err.starts_with("machines/hello.yml: line 2: unknown field `descripton`"),
            "{err}"
        );
    }

    #[test]
    fn yaml_syntax_error_names_line() {
        let err = load_err("bad", "name: bad\nstates: [\n");
        assert!(err.starts_with("machines/bad.yml: line "), "{err}");
    }

    #[test]
    fn project_local_hides_shared() {
        let tmp = project(&[("hello", &fixture("hello"))]);
        let shared = tmp.path().join("shared");
        let shared_hello = fixture("hello").replace("Run one script.", "Shared hello.");
        write_machines(
            &shared,
            &[("hello", &shared_hello), ("deploy", &fixture("deploy"))],
        );

        let machines = load_machines(&tmp.path().join(".decree"), Some(&shared)).unwrap();
        assert_eq!(machines.len(), 2);
        assert_eq!(machines["hello"].description(), "Run one script.");
        assert!(machines["hello"]
            .path
            .starts_with(tmp.path().join(".decree")));
        assert!(machines["deploy"].path.starts_with(&shared));
    }

    #[test]
    fn hidden_shared_machine_is_not_read() {
        let tmp = project(&[("hello", &fixture("hello"))]);
        let shared = tmp.path().join("shared");
        write_machines(&shared, &[("hello", "not: [valid")]);
        assert!(load_machines(&tmp.path().join(".decree"), Some(&shared)).is_ok());
    }

    #[test]
    fn shared_machine_error_uses_relative_path() {
        let tmp = project(&[]);
        let shared = tmp.path().join("shared");
        let text = fixture("hello").replace("final: true }    ", "fnal: true }    ");
        write_machines(&shared, &[("hello", &text)]);
        let err = load_machines(&tmp.path().join(".decree"), Some(&shared))
            .unwrap_err()
            .to_string();
        assert!(
            err.starts_with("machines/hello.yml: failed: unknown field `fnal`"),
            "{err}"
        );
    }

    #[test]
    fn missing_dirs_load_nothing() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("nowhere");
        let machines = load_machines(&tmp.path().join(".decree"), Some(&missing)).unwrap();
        assert!(machines.is_empty());
    }

    #[test]
    fn only_yml_files_load() {
        let tmp = project(&[("hello", &fixture("hello"))]);
        let dir = tmp.path().join(".decree").join(MACHINES_DIR);
        fs::write(dir.join("notes.md"), "not a machine").unwrap();
        fs::write(dir.join("other.yaml"), "not: [valid").unwrap();
        let machines = load_machines(&tmp.path().join(".decree"), None).unwrap();
        assert_eq!(machines.keys().collect::<Vec<_>>(), ["hello"]);
    }

    #[test]
    fn internal_transition_type() {
        let m = parse_machine(
            "name: m\ndescription: d\ninitial: a\nstates:\n  a:\n    initial: b\n    \
             transitions: { go: { target: b, type: internal } }\n    states:\n      \
             b: { final: true }\n  failed: { final: true }\n",
        )
        .unwrap();
        let m = flatten("m", PathBuf::from("m.yml"), m);
        let a = &m.nodes[m.find("a").unwrap()];
        assert!(a.transitions[0].internal);
        assert_eq!(m.state_path(m.find("b").unwrap()), "a.b");
    }

    #[test]
    fn yaml_1_1_booleans_are_strings() {
        // The Norway problem: YAML 1.1 reads `on` and `no` as booleans; machines are YAML 1.2.
        let m = parse_machine(
            "name: m\ndescription: d\ninitial: a\nstates:\n  a:\n    \
             transitions: { on: no }\n  no: { final: true }\n  failed: { final: true }\n",
        )
        .unwrap();
        let m = flatten("m", PathBuf::from("m.yml"), m);
        let a = &m.nodes[m.find("a").unwrap()];
        assert_eq!(a.transitions[0].event, "on");
        assert_eq!(a.transitions[0].target, "no");
        assert!(m.find("no").is_some());
    }

    /// Problems for machine `m` given as `text`, other than V12 (no scripts exist here).
    fn problems(text: &str) -> Vec<String> {
        let m = flatten("m", PathBuf::from("m.yml"), parse_machine(text).unwrap());
        let tmp = TempDir::new().unwrap();
        let ids = BTreeSet::from(["m".to_string()]);
        let env = CheckEnv {
            decree_dir: tmp.path(),
            shared_source: None,
            machine_ids: &ids,
        };
        m.validate(text, &env)
            .into_iter()
            .map(|p| format!("{}: {}", p.at, p.message))
            .filter(|p| !p.ends_with("(V12)"))
            .collect()
    }

    const HEAD: &str = "name: m\ndescription: d\ninitial: a\nstates:\n";

    #[test]
    fn section5_examples_validate() {
        for name in EXAMPLES {
            let text = fixture(name);
            let m = flatten(name, PathBuf::from("x"), parse_machine(&text).unwrap());
            let tmp = TempDir::new().unwrap();
            let ids = BTreeSet::from(EXAMPLES.map(String::from));
            let env = CheckEnv {
                decree_dir: tmp.path(),
                shared_source: None,
                machine_ids: &ids,
            };
            let found: Vec<Problem> = m
                .validate(&text, &env)
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
             b:\n        initial: c\n        states:\n          c: {{ final: true }}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
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

    #[test]
    fn default_only_on_router_states() {
        let text = format!(
            "{HEAD}  a: {{ invoke: x, default: done, transitions: {{ done: done }} }}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        assert_eq!(
            problems(&text),
            ["a: `default` is only allowed on router states (V9)"]
        );
    }

    #[test]
    fn router_rules() {
        let text = format!(
            "{HEAD}  a:\n    router: llm\n    invoke: x\n    transitions:\n      \
             done: {{ target: done, description: d }}\n      error: failed\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        assert_eq!(
            problems(&text),
            [
                "a: router state may not declare `done` (V9)",
                "a: router state needs at least 2 events other than `error`, has 1 (V9)",
                "a: router state needs a `default` event (V9)",
                "a: router state needs a `description` (V9)",
            ]
        );
    }

    #[test]
    fn cond_rules() {
        let router = "    description: d\n    router: llm\n    invoke: x\n    default: pass\n";
        let text = format!(
            "{HEAD}  a:\n{router}    transitions:\n      \
             pass: {{ target: done, description: d, cond: \"visits.a > 1\" }}\n      \
             again: {{ target: a, description: d, cond: \"visits.w < 2\" }}\n      \
             other: {{ target: b, description: d, cond: \"a && b\" }}\n  \
             b:\n    invoke: x\n    transitions: {{ done: {{ target: done, cond: \"1 == 1\" }} }}\n  \
             w:\n    initial: z\n    states:\n      z: {{ final: true }}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        let found: Vec<String> = problems(&text)
            .into_iter()
            .filter(|p| p.ends_with("(V10)"))
            .collect();
        assert_eq!(found.len(), 4, "{found:?}");
        assert!(found[0].starts_with("a: transition `again`: cond `visits.w < 2` reads `visits.w`, but `w` is not an atomic state"));
        assert!(found[1].starts_with("a: transition `other`: cond `a && b`: "));
        assert_eq!(
            found[2],
            "a: default event `pass` may not have a `cond` (V10)"
        );
        assert_eq!(
            found[3],
            "b: transition `done`: `cond` is only allowed on router states (V10)"
        );
    }

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
    fn nested_final_continues_through_done_state() {
        let text = format!(
            "{HEAD}  a:\n    initial: b\n    transitions: {{ done.state.a: done }}\n    states:\n      \
             b: {{ transitions: {{ done: c }} }}\n      c: {{ final: true }}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        assert!(problems(&text).is_empty(), "{:?}", problems(&text));

        // Without a handler for done.state.a, the pass-through b can never finish: it only
        // takes `done`, never a's `go`, so `done` is unreachable too.
        let stalled = text.replace(
            "transitions: { done.state.a: done }",
            "transitions: { go: done }",
        );
        assert_eq!(
            problems(&stalled),
            [
                "done: state is unreachable from the root `initial` (V11)",
                "a.b: state cannot reach a root-level final state (V11)",
            ]
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
            "{HEAD}  a: {{ invoke: x, transitions: {{ done: done }} }}\n  \
             done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        let m = flatten("m", PathBuf::from("m.yml"), parse_machine(&text).unwrap());
        let tmp = TempDir::new().unwrap();
        let ids = BTreeSet::new();
        let env = CheckEnv {
            decree_dir: tmp.path(),
            shared_source: None,
            machine_ids: &ids,
        };
        let found = m.validate(&text, &env);
        assert_eq!(
            found,
            [Problem {
                at: "a".into(),
                message: "script `x` not found; searched scripts/m, scripts (V12)".into(),
            }]
        );
    }
}
