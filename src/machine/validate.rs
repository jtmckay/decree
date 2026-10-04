//! Validation (docs/reference/machines.md, Validation): rules V1–V21 on a machine's arena.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::{
    event_matches, is_event_name, is_ident, is_reserved_event, ChooseKind, DataType, Edge, Invoke,
    LoadedMachine, MachineInvoke, FAILED, ROUTER_MACHINE,
};
use crate::cond::{Condition, Operand, Subject, Test};
use crate::runtime::resolve::resolve_script;

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

impl LoadedMachine {
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
