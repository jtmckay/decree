//! Property tests of the interpreter's invariants, through the binary (docs/reference/runs.md,
//! Step loop, Visits and events.jsonl; machines.md, Rules; scripts.md, Events from an invoke).
//! proptest generates small valid machines (up to 6 states, at most one compound state,
//! script and `check` invokes, `onentry`/`onexit` hooks, finals at both levels) and what
//! each script execution does (exit code, an event it names, declared or not). For
//! each case, `decree check` passes, `decree process` runs one message, and the run's
//! events, hooks and mirror must hold the invariants listed on `check_case`, and every
//! line of its `events.jsonl` must validate against `events.schema.json`, carry the run's
//! one `trace_id`, name a `span_id` no other event names, and agree with `traces.jsonl`
//! (docs/reference/observability.md, Traces).
//!
//! Termination: every transition goes forward in state order except a `check`'s `true`,
//! which may go back only when it tests `visits` of its own state with `less_than`, so
//! every loop passes a bound.

use assert_cmd::cargo::cargo_bin_cmd;
use proptest::prelude::*;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use tempfile::TempDir;

mod common;
use common::write_script;
#[path = "common/schema.rs"]
mod schema;

#[path = "common/traces.rs"]
mod traces;

/// `events.schema.json`, compiled once for every case.
fn events_validator() -> &'static jsonschema::Validator {
    static VALIDATOR: std::sync::OnceLock<jsonschema::Validator> = std::sync::OnceLock::new();
    VALIDATOR.get_or_init(schema::events_validator)
}

/// Every hook: appends `<phase> <state>` to `trace.log`.
const HOOK: &str = r#"#!/bin/sh
echo "$DECREE_PHASE $DECREE_STATE" >> "$DECREE_PROJECT_ROOT/trace.log"
"#;

/// Every script invoke: appends `invoke <state> <visits> <attempt>` to `trace.log`, then
/// replays execution `n` of its state from `plan/<state>/<n>` (`code`, `name`); with no
/// plan left it exits 0 and names no event.
const WORK: &str = r#"#!/bin/sh
d="$DECREE_PROJECT_ROOT/plan/$DECREE_STATE"
mkdir -p "$d"
n=$(( $(cat "$d/count" 2>/dev/null || echo 0) + 1 ))
echo "$n" > "$d/count"
echo "invoke $DECREE_STATE $DECREE_VISITS $DECREE_ATTEMPT" >> "$DECREE_PROJECT_ROOT/trace.log"
code=0
name=
[ -f "$d/$n" ] && . "$d/$n"
[ -n "$name" ] && echo "$name" > "$DECREE_EVENT_FILE"
exit "$code"
"#;

const NAMES: [Option<&str>; 4] = [None, Some("ev_a"), Some("ev_b"), Some("ev_z")];
const OPS: [&str; 6] = [
    "equals",
    "not_equals",
    "less_than",
    "at_most",
    "more_than",
    "at_least",
];

/// A transition target.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Atom(usize),
    Grp,
    GrpEnd,
    Done,
    Alt,
    Failed,
}

#[derive(Clone, Debug)]
enum Invoke {
    Script {
        attempts: u32,
        extra: Option<(&'static str, Target)>,
        error: Option<Target>,
    },
    Check {
        of: usize,
        op: &'static str,
        n: u32,
        on_true: Target,
    },
}

#[derive(Clone, Debug)]
struct Atom {
    invoke: Invoke,
    /// The `done` target of a script, the `false` target of a check.
    next: Target,
    onentry: bool,
    onexit: bool,
}

/// The compound state `grp`, around atoms `start..end`, with final child `grp_end`.
#[derive(Clone, Debug)]
struct Grp {
    start: usize,
    end: usize,
    onentry: bool,
    onexit: bool,
    end_onentry: bool,
    after: Target,
    error: Option<Target>,
    extra: Option<(&'static str, Target)>,
}

#[derive(Clone, Debug)]
struct Spec {
    atoms: Vec<Atom>,
    grp: Option<Grp>,
    root_onentry: bool,
    root_onexit: bool,
    /// `onentry` hooks on `done`, `alt` and `failed`.
    final_onentry: [bool; 3],
    /// Per atom: what each script execution does, `(exit code, named event)`.
    plans: Vec<Vec<(i64, Option<&'static str>)>>,
}

/// Draws numbers from the generated list; 0 once it runs out.
struct Draw<'a>(std::slice::Iter<'a, u32>);

impl Draw<'_> {
    fn below(&mut self, n: usize) -> usize {
        self.0.next().map_or(0, |v| *v as usize % n)
    }

    fn flip(&mut self) -> bool {
        self.below(2) == 1
    }

    fn pick<T: Copy>(&mut self, from: &[T]) -> T {
        from[self.below(from.len())]
    }
}

impl Spec {
    fn build(raw: &[u32]) -> Spec {
        let mut d = Draw(raw.iter());
        let k = 1 + d.below(5);
        let grp = d.flip().then(|| {
            let start = d.below(k);
            let end = start + 1 + d.below(k - start);
            (start, end)
        });
        let inside = |i: usize| grp.is_some_and(|(s, e)| s <= i && i < e);
        // Forward targets of atom `i`: later atoms, `grp` from before it, `grp_end` from
        // inside it, and the root finals.
        let forward = |i: usize| {
            let mut t: Vec<Target> = (i + 1..k).map(Target::Atom).collect();
            if grp.is_some_and(|(s, _)| i < s) {
                t.push(Target::Grp);
            }
            if inside(i) {
                t.push(Target::GrpEnd);
            }
            t.extend([Target::Done, Target::Alt, Target::Failed]);
            t
        };
        let mut atoms = Vec::new();
        for i in 0..k {
            let next = match grp {
                Some((_, e)) if inside(i) && i + 1 == e => Target::GrpEnd,
                Some((s, _)) if i + 1 == s && d.flip() => Target::Grp,
                _ if i + 1 < k => Target::Atom(i + 1),
                _ => Target::Done,
            };
            let invoke = if d.flip() {
                if d.flip() {
                    // A bounded loop: back to this state or an earlier one.
                    let mut back: Vec<Target> = (0..=i).map(Target::Atom).collect();
                    if grp.is_some_and(|(s, _)| s <= i) {
                        back.push(Target::Grp);
                    }
                    Invoke::Check {
                        of: i,
                        op: "less_than",
                        n: 1 + d.below(3) as u32,
                        on_true: d.pick(&back),
                    }
                } else {
                    Invoke::Check {
                        of: d.below(k),
                        op: d.pick(&OPS),
                        n: d.below(3) as u32,
                        on_true: d.pick(&forward(i)),
                    }
                }
            } else {
                let extra = d
                    .flip()
                    .then(|| (d.pick(&["ev_a", "ev_b"]), d.pick(&forward(i))));
                let error = d.flip().then(|| d.pick(&forward(i)));
                Invoke::Script {
                    attempts: 1 + d.below(2) as u32,
                    extra,
                    error,
                }
            };
            atoms.push(Atom {
                invoke,
                next,
                onentry: d.flip(),
                onexit: d.flip(),
            });
        }
        let grp = grp.map(|(start, end)| {
            let mut beyond: Vec<Target> = (end..k).map(Target::Atom).collect();
            beyond.extend([Target::Done, Target::Alt, Target::Failed]);
            // Only a script inside can raise `grp`'s own events (V11 counts reachability
            // by the events each state can produce).
            let script = atoms[start..end]
                .iter()
                .any(|a: &Atom| matches!(a.invoke, Invoke::Script { .. }));
            Grp {
                start,
                end,
                onentry: d.flip(),
                onexit: d.flip(),
                end_onentry: d.flip(),
                after: if end < k {
                    Target::Atom(end)
                } else {
                    Target::Done
                },
                error: (d.flip() && script).then(|| d.pick(&beyond)),
                extra: (d.flip() && script).then(|| (d.pick(&["ev_a", "ev_b"]), d.pick(&beyond))),
            }
        });
        let mut plans = Vec::new();
        for _ in 0..k {
            let runs = d.below(4);
            plans.push(
                (0..runs)
                    .map(|_| (d.pick(&[0, 0, 1, 2]), d.pick(&NAMES)))
                    .collect(),
            );
        }
        Spec {
            atoms,
            grp,
            root_onentry: d.flip(),
            root_onexit: d.flip(),
            final_onentry: [d.flip(), d.flip(), d.flip()],
            plans,
        }
    }

    fn name(&self, t: Target) -> String {
        match t {
            Target::Atom(i) => format!("s{i}"),
            Target::Grp => "grp".into(),
            Target::GrpEnd => "grp_end".into(),
            Target::Done => "done".into(),
            Target::Alt => "alt".into(),
            Target::Failed => "failed".into(),
        }
    }

    fn atom_of(&self, state: &str) -> Option<usize> {
        state.strip_prefix('s').and_then(|n| n.parse().ok())
    }

    /// Whether `state` (an atom or `grp_end`) is inside `grp`.
    fn in_grp(&self, state: &str) -> bool {
        let Some(g) = &self.grp else { return false };
        state == "grp_end"
            || self
                .atom_of(state)
                .is_some_and(|i| g.start <= i && i < g.end)
    }

    /// The root finals: `done`, `failed`, and `alt` when a transition targets it (V11).
    fn finals(&self) -> Vec<&'static str> {
        let mut holders: Vec<String> = (0..self.atoms.len()).map(|i| format!("s{i}")).collect();
        if self.grp.is_some() {
            holders.push("grp".into());
        }
        let alt = holders
            .iter()
            .any(|h| self.transitions(h).iter().any(|(_, t)| *t == Target::Alt));
        ["done", "alt", "failed"]
            .into_iter()
            .filter(|f| alt || *f != "alt")
            .collect()
    }

    fn is_root_final(&self, state: &str) -> bool {
        ["done", "alt", "failed"].contains(&state)
    }

    /// The atomic states and finals a transition may enter.
    fn states(&self) -> Vec<String> {
        let mut s: Vec<String> = (0..self.atoms.len()).map(|i| format!("s{i}")).collect();
        if self.grp.is_some() {
            s.push("grp_end".into());
        }
        s.extend(self.finals().into_iter().map(String::from));
        s
    }

    /// The transitions declared on `state` (an atom, `grp` or `grp_end`).
    fn transitions(&self, state: &str) -> Vec<(String, Target)> {
        if state == "grp" {
            let g = self.grp.as_ref().unwrap();
            let mut t = vec![("done.state.grp".to_string(), g.after)];
            t.extend(g.error.map(|e| ("error".to_string(), e)));
            t.extend(g.extra.map(|(ev, e)| (ev.to_string(), e)));
            return t;
        }
        let Some(i) = self.atom_of(state) else {
            return Vec::new();
        };
        let atom = &self.atoms[i];
        match &atom.invoke {
            Invoke::Script { extra, error, .. } => {
                let mut t = vec![("done".to_string(), atom.next)];
                t.extend(extra.map(|(ev, e)| (ev.to_string(), e)));
                t.extend(error.map(|e| ("error".to_string(), e)));
                t
            }
            Invoke::Check { on_true, .. } => {
                vec![
                    ("true".to_string(), *on_true),
                    ("false".to_string(), atom.next),
                ]
            }
        }
    }

    /// The transition `event` selects from atomic `state`: the state's own, else `grp`'s;
    /// an unhandled `error` goes to `failed`. Returns its source and declared target.
    fn select(&self, state: &str, event: &str) -> Option<(String, Target)> {
        let mut holders = vec![state.to_string()];
        if self.in_grp(state) {
            holders.push("grp".into());
        }
        for h in holders {
            if let Some((_, t)) = self.transitions(&h).into_iter().find(|(e, _)| e == event) {
                return Some((h, t));
            }
        }
        (event == "error").then(|| ("_root".to_string(), Target::Failed))
    }

    fn hooks(&self, state: &str) -> (bool, bool) {
        if state == "grp" {
            let g = self.grp.as_ref().unwrap();
            return (g.onentry, g.onexit);
        }
        if state == "grp_end" {
            return (self.grp.as_ref().unwrap().end_onentry, false);
        }
        if let Some(i) = ["done", "alt", "failed"].iter().position(|f| *f == state) {
            return (self.final_onentry[i], false);
        }
        let atom = &self.atoms[self.atom_of(state).unwrap()];
        (atom.onentry, atom.onexit)
    }

    fn yaml(&self) -> String {
        let mut y = String::from("name: prop\ndescription: A generated machine.\n");
        if self.root_onentry {
            y.push_str("onentry: [hook]\n");
        }
        if self.root_onexit {
            y.push_str("onexit: [hook]\n");
        }
        let initial = match &self.grp {
            Some(g) if g.start == 0 => "grp",
            _ => "s0",
        };
        y.push_str(&format!("initial: {initial}\nstates:\n"));
        let transitions = |state: &str| -> String {
            let t: Vec<String> = self
                .transitions(state)
                .into_iter()
                .map(|(e, t)| format!("{e}: {}", self.name(t)))
                .collect();
            format!("{{ {} }}", t.join(", "))
        };
        let atom = |i: usize, indent: &str| -> String {
            let a = &self.atoms[i];
            let mut s = format!("{indent}s{i}:\n");
            match &a.invoke {
                Invoke::Script { attempts: 1, .. } => {
                    s.push_str(&format!("{indent}  invoke: work\n"));
                }
                Invoke::Script { attempts, .. } => s.push_str(&format!(
                    "{indent}  invoke:\n{indent}    script: {{ name: work, max_attempts: {attempts} }}\n"
                )),
                Invoke::Check { of, op, n, .. } => s.push_str(&format!(
                    "{indent}  invoke:\n{indent}    check: {{ visits: s{of}, {op}: {n} }}\n"
                )),
            }
            if a.onentry {
                s.push_str(&format!("{indent}  onentry: [hook]\n"));
            }
            if a.onexit {
                s.push_str(&format!("{indent}  onexit: [hook]\n"));
            }
            s.push_str(&format!(
                "{indent}  transitions: {}\n",
                transitions(&format!("s{i}"))
            ));
            s
        };
        for i in 0..self.atoms.len() {
            match &self.grp {
                Some(g) if i == g.start => {
                    y.push_str("  grp:\n");
                    if g.onentry {
                        y.push_str("    onentry: [hook]\n");
                    }
                    if g.onexit {
                        y.push_str("    onexit: [hook]\n");
                    }
                    y.push_str(&format!("    initial: s{}\n", g.start));
                    y.push_str(&format!("    transitions: {}\n", transitions("grp")));
                    y.push_str("    states:\n");
                    for j in g.start..g.end {
                        y.push_str(&atom(j, "      "));
                    }
                    let hook = if g.end_onentry {
                        ", onentry: [hook]"
                    } else {
                        ""
                    };
                    y.push_str(&format!("      grp_end: {{ final: true{hook} }}\n"));
                }
                Some(g) if g.start < i && i < g.end => {}
                _ => y.push_str(&atom(i, "  ")),
            }
        }
        for (i, f) in ["done", "alt", "failed"].iter().enumerate() {
            if !self.finals().contains(f) {
                continue;
            }
            let hook = if self.final_onentry[i] {
                ", onentry: [hook]"
            } else {
                ""
            };
            y.push_str(&format!("  {f}: {{ final: true{hook} }}\n"));
        }
        y
    }
}

/// What one case produced.
struct Outcome {
    check_code: Option<i32>,
    check_stdout: String,
    process_code: Option<i32>,
    events: Vec<Value>,
    /// Every way a line of `events.jsonl` breaks `events.schema.json`.
    schema_errors: Vec<String>,
    /// How `traces.jsonl` disagrees with `events.jsonl`, if it does.
    traces_error: Option<String>,
    trace: Vec<String>,
    mirror: Option<String>,
}

fn run_case(spec: &Spec) -> Outcome {
    let tmp = TempDir::new().unwrap();
    let decree = tmp.path().join(".decree");
    for dir in ["machines", "scripts", "migrations", "inbox", "runs"] {
        fs::create_dir_all(decree.join(dir)).unwrap();
    }
    fs::write(decree.join("processed.md"), "").unwrap();
    fs::write(decree.join("machines/prop.yml"), spec.yaml()).unwrap();
    write_script(&decree.join("scripts/hook"), HOOK);
    write_script(&decree.join("scripts/work"), WORK);
    for (i, plan) in spec.plans.iter().enumerate() {
        let dir = tmp.path().join("plan").join(format!("s{i}"));
        fs::create_dir_all(&dir).unwrap();
        for (n, (code, name)) in plan.iter().enumerate() {
            let text = format!("code={code}\nname={}\n", name.unwrap_or_default());
            fs::write(dir.join((n + 1).to_string()), text).unwrap();
        }
    }
    fs::write(
        decree.join("inbox/a.md"),
        "---\nid: run-a\nmachine: prop\n---\n",
    )
    .unwrap();

    let decree_cmd = |arg: &str| {
        cargo_bin_cmd!("decree")
            .current_dir(tmp.path())
            .env("NO_COLOR", "1")
            .arg(arg)
            .output()
            .unwrap()
    };
    let check = decree_cmd("check");
    let process = decree_cmd("process");
    let run = decree.join("runs/run-a");
    let log = fs::read_to_string(run.join("events.jsonl")).unwrap_or_default();
    let schema_errors = schema::events_errors(events_validator(), &log);
    let traces_error = std::panic::catch_unwind(|| traces::agree_with_events(&run))
        .err()
        .map(|e| {
            e.downcast_ref::<String>()
                .cloned()
                .unwrap_or_else(|| "traces.jsonl disagrees with events.jsonl".to_string())
        });
    let events = log
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let trace = fs::read_to_string(tmp.path().join("trace.log"))
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect();
    let mirror = fs::read_to_string(run.join("message.md"))
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix("state: ").map(String::from));
    Outcome {
        check_code: check.status.code(),
        check_stdout: String::from_utf8_lossy(&check.stdout).into_owned(),
        process_code: process.status.code(),
        events,
        schema_errors,
        traces_error,
        trace,
        mirror,
    }
}

fn compare(op: &str, left: i64, right: i64) -> bool {
    match op {
        "equals" => left == right,
        "not_equals" => left != right,
        "less_than" => left < right,
        "at_most" => left <= right,
        "more_than" => left > right,
        "at_least" => left >= right,
        _ => unreachable!("{op}"),
    }
}

/// The invariants, checked against an oracle that walks the events:
/// - `decree check` passes, and the run ends in a root-level final state;
/// - every line validates against `events.schema.json`;
/// - every event has the run's one `trace_id`, every `span_id` is unique within the run, and
///   `traces.jsonl` has the spans the events name, at their times;
/// - `seq` is 1, 2, 3, … with no gaps;
/// - every `transition`'s `to` is a state of the machine and its `from` is the previous
///   transition's `to`; its event follows from the invoke's result (scripts.md, Events from
///   an invoke; attempts) or the check, and its `to` from the selected transition
///   (machines.md, Rules), following `initial` into `grp`;
/// - `DECREE_VISITS` and every visits check equal the number of transitions into the
///   state so far, attempts excluded (runs.md, Visits);
/// - the `state:` mirror equals the last transition's `to`;
/// - `onentry`/`onexit` hooks and invokes run in the documented order (runs.md, Step
///   loop; machines.md, exit and entry order).
fn check_case(spec: &Spec, out: &Outcome) -> Result<(), TestCaseError> {
    prop_assert_eq!(
        out.check_code,
        Some(0),
        "decree check: {}",
        out.check_stdout
    );
    let events = &out.events;
    prop_assert!(!events.is_empty(), "no events");
    prop_assert!(
        out.schema_errors.is_empty(),
        "events.schema.json: {:?}",
        out.schema_errors
    );
    prop_assert!(out.traces_error.is_none(), "{:?}", out.traces_error);
    let trace_id = &events[0]["trace_id"];
    prop_assert!(trace_id.is_string(), "no trace_id: {}", events[0]);
    let mut span_ids = std::collections::BTreeSet::new();
    for e in events {
        prop_assert_eq!(&e["trace_id"], trace_id, "{}", e);
        if let Some(span) = e["span_id"].as_str() {
            prop_assert!(span_ids.insert(span), "span_id {} twice", span);
        }
        if let Some(parent) = e["parent_span_id"].as_str() {
            prop_assert!(
                !span_ids.contains(parent),
                "parent {} is in the run",
                parent
            );
        }
    }

    let mut visits: BTreeMap<String, i64> = BTreeMap::new();
    let mut execs: BTreeMap<String, usize> = BTreeMap::new();
    let mut trace: Vec<String> = Vec::new();
    let mut prev: Option<String> = None;
    // The result of the latest invoke or check, for the transition that follows.
    let mut invoked: Option<(i64, Option<&str>, u64)> = None;
    let mut decided: Option<String> = None;
    let states = spec.states();
    let enter = |trace: &mut Vec<String>, state: &str| {
        if spec.hooks(state).0 {
            trace.push(format!("onentry {state}"));
        }
    };
    let exit = |trace: &mut Vec<String>, state: &str| {
        if spec.hooks(state).1 {
            trace.push(format!("onexit {state}"));
        }
    };

    for (n, e) in events.iter().enumerate() {
        prop_assert_eq!(&e["seq"], &Value::from(n + 1), "seq of {}", e);
        let kind = e["type"].as_str().unwrap();
        match kind {
            "transition" => {
                let to = e["to"].as_str().unwrap().to_string();
                let event = e["event"].as_str().unwrap();
                prop_assert!(states.contains(&to), "unknown state {}", to);
                match &prev {
                    None => {
                        prop_assert_eq!(&e["from"], &Value::Null);
                        prop_assert_eq!(event, "claimed");
                        prop_assert_eq!(&to, "s0");
                        if spec.root_onentry {
                            trace.push("onentry _root".into());
                        }
                        if spec.in_grp(&to) {
                            enter(&mut trace, "grp");
                        }
                        enter(&mut trace, &to);
                    }
                    Some(from) => {
                        prop_assert_eq!(e["from"].as_str(), Some(from.as_str()), "{}", e);
                        let source = e["source"].as_str().unwrap();
                        if let Some((code, name, attempt)) = invoked.take() {
                            prop_assert_eq!(&e["exit_code"], &Value::from(code));
                            let Invoke::Script { attempts, .. } =
                                spec.atoms[spec.atom_of(from).unwrap()].invoke
                            else {
                                unreachable!("invoked from a script state");
                            };
                            if code != 0 && attempt < u64::from(attempts) {
                                prop_assert_eq!(source, "attempt");
                                prop_assert_eq!(event, "error");
                                prop_assert_eq!(&to, from);
                                prev = Some(to);
                                continue;
                            }
                            let declared = name.filter(|p| spec.select(from, p).is_some());
                            let want = match (code, name) {
                                (0, Some(_)) => declared.unwrap_or("error"),
                                (0, None) => "done",
                                _ => "error",
                            };
                            prop_assert_eq!(event, want, "{}", e);
                            let invalid =
                                (code == 0 && declared.is_none()).then_some(name).flatten();
                            prop_assert_eq!(e["invalid_event"].as_str(), invalid, "{}", e);
                        } else if let Some(d) = decided.take() {
                            prop_assert_eq!(event, d.as_str());
                            prop_assert_eq!(source, "check");
                        } else {
                            prop_assert_eq!(from.as_str(), "grp_end");
                            prop_assert_eq!(event, "done.state.grp");
                            prop_assert_eq!(source, "internal");
                        }
                        let selected = spec.select(from, event);
                        prop_assert!(
                            selected.is_some(),
                            "nothing selects {} from {}",
                            event,
                            from
                        );
                        let (holder, target) = selected.unwrap();
                        let entered = match target {
                            Target::Grp => format!("s{}", spec.grp.as_ref().unwrap().start),
                            t => spec.name(t),
                        };
                        prop_assert_eq!(&to, &entered, "{}", e);
                        // The domain is `grp` when it is a proper ancestor of the
                        // transition's source and its target; otherwise the root.
                        let in_grp_domain = holder != "grp"
                            && spec.in_grp(&holder)
                            && target != Target::Grp
                            && spec.in_grp(&spec.name(target));
                        exit(&mut trace, from);
                        if spec.in_grp(from) && !in_grp_domain {
                            exit(&mut trace, "grp");
                        }
                        if spec.in_grp(&to) && !in_grp_domain {
                            enter(&mut trace, "grp");
                        }
                        enter(&mut trace, &to);
                        if spec.is_root_final(&to) && spec.root_onexit {
                            trace.push("onexit _root".into());
                        }
                    }
                }
                *visits.entry(to.clone()).or_default() += 1;
                prev = Some(to);
            }
            "script" if e["phase"] == "invoke" => {
                let state = e["state"].as_str().unwrap();
                prop_assert_eq!(Some(state), prev.as_deref());
                let i = spec.atom_of(state).unwrap();
                let n = execs.entry(state.to_string()).or_default();
                *n += 1;
                let (code, name) = spec.plans[i].get(*n - 1).copied().unwrap_or((0, None));
                prop_assert_eq!(&e["exit_code"], &Value::from(code), "{}", e);
                let attempt = e["attempt"].as_u64().unwrap();
                trace.push(format!("invoke {state} {} {attempt}", visits[state]));
                invoked = Some((code, name, attempt));
            }
            "script" => {}
            "decision" => {
                let state = e["state"].as_str().unwrap();
                prop_assert_eq!(Some(state), prev.as_deref());
                let Invoke::Check { of, op, n, .. } =
                    spec.atoms[spec.atom_of(state).unwrap()].invoke
                else {
                    unreachable!("a decision from a check state");
                };
                let count = visits.get(&format!("s{of}")).copied().unwrap_or(0);
                let want = if compare(op, count, i64::from(n)) {
                    "true"
                } else {
                    "false"
                };
                prop_assert_eq!(e["kind"].as_str(), Some("check"));
                prop_assert_eq!(e["event"].as_str(), Some(want), "{}", e);
                decided = Some(want.to_string());
            }
            "run_finished" => {
                prop_assert_eq!(n + 1, events.len(), "run_finished is the last event");
                prop_assert_eq!(e["state"].as_str(), prev.as_deref());
            }
            other => prop_assert!(false, "unexpected event type {}", other),
        }
    }
    let last = prev.unwrap();
    prop_assert!(spec.is_root_final(&last), "ended in {}", last);
    prop_assert_eq!(
        events.last().unwrap()["type"].as_str(),
        Some("run_finished")
    );
    let want_code = if last == "failed" { 1 } else { 0 };
    prop_assert_eq!(out.process_code, Some(want_code));
    prop_assert_eq!(out.mirror.as_deref(), Some(last.as_str()));
    prop_assert_eq!(&out.trace, &trace);
    Ok(())
}

proptest! {
    // The default case count; a failure prints its machine and plans, so none is saved.
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn generated_machines_keep_the_interpreter_invariants(
        raw in proptest::collection::vec(any::<u32>(), 64)
    ) {
        let spec = Spec::build(&raw);
        let out = run_case(&spec);
        check_case(&spec, &out).map_err(|e| {
            TestCaseError::fail(format!("{e}\n{}\nplans: {:?}", spec.yaml(), spec.plans))
        })?;
    }
}
