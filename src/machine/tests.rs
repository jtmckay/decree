use super::validate::{CheckEnv, Problem};
use super::*;
use crate::cond::Operand;
use std::fs;
use tempfile::TempDir;

/// `examples/project`'s `hello`, `develop` and `deploy`, and from `tests/fixtures/feature/`: `ship`, which
/// invokes `feature` and `deploy`, `feature` (nesting, a check, a model, a person, `data`,
/// `emits`, `onentry` and `onexit`) and the router `feature` names by default.
const EXAMPLES: [&str; 6] = ["hello", "develop", "deploy", "ship", "feature", "router"];

fn fixture(name: &str) -> String {
    let project = if ["hello", "develop", "deploy"].contains(&name) {
        "examples/project"
    } else {
        "tests/fixtures/feature"
    };
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(project)
        .join(".decree/machines")
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
fn reference_examples_load() {
    let files: Vec<(&str, String)> = EXAMPLES.iter().map(|n| (*n, fixture(n))).collect();
    let refs: Vec<(&str, &str)> = files.iter().map(|(n, t)| (*n, t.as_str())).collect();
    let tmp = project(&refs);
    let machines = load_machines(&tmp.path().join(".decree")).unwrap();
    assert_eq!(
        machines.keys().map(String::as_str).collect::<Vec<_>>(),
        ["deploy", "develop", "feature", "hello", "router", "ship"]
    );
    for (id, m) in &machines {
        assert_eq!(&m.root().id, id);
        assert!(!m.description().is_empty());
    }
    assert_eq!(machines["hello"].nodes.len(), 1 + 3);
    assert_eq!(machines["develop"].nodes.len(), 1 + 4);
    assert_eq!(machines["deploy"].nodes.len(), 1 + 6);
    assert_eq!(machines["ship"].nodes.len(), 1 + 4);

    let develop = &machines["develop"];
    let implement = &develop.nodes[develop.find("implement").unwrap()];
    assert_eq!(
        implement.invoke,
        Some(Invoke::Script(ScriptInvoke {
            name: "implement".into(),
            attempts: Some(Attempts::Values(vec!["local".into(), "claude".into()])),
            timeout: None,
            env: BTreeMap::new(),
        }))
    );

    let deploy = &machines["deploy"];
    let approval = &deploy.nodes[deploy.find("approval").unwrap()];
    let Some(Invoke::Person(c)) = &approval.invoke else {
        panic!("{:?}", approval.invoke);
    };
    assert_eq!(c.question.as_deref(), Some("Ship this build?"));
    assert_eq!(c.ask.as_deref(), Some("ask_person"));
    assert_eq!(c.timeout, Some(Duration::from_secs(86400)));

    let ship = &machines["ship"];
    let build = &ship.nodes[ship.find("build").unwrap()];
    assert_eq!(
        build.invoke,
        Some(Invoke::Machine(MachineInvoke {
            name: "feature".into(),
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
        Some(Invoke::Script(ScriptInvoke {
            name: "verify".into(),
            attempts: None,
            timeout: None,
            env: BTreeMap::new(),
        }))
    );

    let rounds = &m.nodes[m.find("rounds_left").unwrap()];
    let Some(Invoke::Check(check)) = &rounds.invoke else {
        panic!("{:?}", rounds.invoke);
    };
    assert_eq!(check.visits.as_deref(), Some("implement"));
    assert_eq!(check.less_than, Some(Operand::Data("max_rounds".into())));
    // `true:` and `false:` are YAML booleans, read as the event names `true` and `false`.
    let events: Vec<&str> = rounds
        .transitions
        .iter()
        .map(|e| e.event.as_str())
        .collect();
    assert_eq!(events, ["false", "true"]);

    let triage = m.find("triage").unwrap();
    let Some(Invoke::Model(c)) = &m.nodes[triage].invoke else {
        panic!();
    };
    assert_eq!(c.min_confidence, Some(0.8));
    assert_eq!(c.output.as_deref(), Some("verify"));
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
    assert_eq!(m.attempt_count(m.find("implement").unwrap()), 2);
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
fn misspelled_script_setting_names_file_and_state_path() {
    let text = fixture("feature").replace("attempts: 2", "attempt: 2");
    let err = load_err("feature", &text);
    assert!(
        err.starts_with("machines/feature.yml: work.implement: unknown field `attempt`"),
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
        err.contains("greet: unknown invoke kind `run`: `invoke` names one of `script`, `check`, `model`, `person` or `machine`"),
        "{err}"
    );
    assert!(err.ends_with("(V19)"), "{err}");
    let text = fixture("hello").replace(
        "invoke: greet ",
        "invoke: { script: greet, check: { visits: greet, equals: 1 } }",
    );
    let err = load_err("hello", &text);
    assert!(
        err.contains("greet: `invoke` is a script name or a map with exactly one key, which names the kind: `script`, `check`, `model`, `person` or `machine`"),
        "{err}"
    );
}

#[test]
fn misspelled_root_key_names_line() {
    let text = fixture("hello").replace("description:", "descripton:");
    let err = load_err("hello", &text);
    assert!(
        err.starts_with("machines/hello.yml: line 4: unknown field `descripton`"),
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
    let text = "name: b\ndescription: A router state inside a compound state.\ninitial: work\n\
                states:\n  work:\n    initial: step\n    transitions: { done.state.work: done }\n    \
                states:\n      step:\n        invoke: work\n        router: llm\n        transitions:\n          \
                pass: { target: fin, description: The work is finished. }\n          \
                retry: { target: step, description: Run the work again. }\n      fin: { final: true }\n  \
                done: { final: true }\n  failed: { final: true }\n";
    assert_eq!(
        parse_machine(text).unwrap_err(),
        "work.step: router on a state is not supported: make the decision a state with invoke: { model: { question: ... } } (V19)"
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
fn reference_examples_validate() {
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
        found.contains(
            &"b: state id `b` is also used by `a.b`; state ids are unique across the machine (V2)"
                .to_string()
        ),
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
fn check_must_handle_true_and_false() {
    let text = format!(
        "{HEAD}  a: {{ invoke: {{ check: {{ visits: a, less_than: 2 }} }}, transitions: {{ true: done }} }}\n  \
         done: {{ final: true }}\n  failed: {{ final: true }}\n"
    );
    assert_eq!(
        problems(&text),
        ["a: a `check` state must handle `false`, itself or through an ancestor (V8)"]
    );
}

#[test]
fn check_events_true_and_false_are_written_without_quotes() {
    let text = format!(
        "{HEAD}  a:\n    invoke:\n      check: {{ visits: a, less_than: 2 }}\n    \
         transitions: {{ true: done, false: failed }}\n  \
         done: {{ final: true }}\n  failed: {{ final: true }}\n"
    );
    let m = load("m", &text);
    let a = &m.nodes[m.find("a").unwrap()];
    let events: Vec<(&str, &str)> = a
        .transitions
        .iter()
        .map(|e| (e.event.as_str(), e.target.as_str()))
        .collect();
    assert_eq!(events, [("false", "failed"), ("true", "done")]);
    assert!(problems(&text).is_empty(), "{:?}", problems(&text));
}

#[test]
fn person_needs_a_question_and_described_options() {
    let text = format!(
        "{HEAD}  a:\n    invoke:\n      person: {{ ask: tell }}\n    transitions:\n      \
         ship: {{ target: done, description: Ship it. }}\n      stop: done\n  \
         done: {{ final: true }}\n  failed: {{ final: true }}\n"
    );
    assert_eq!(
        problems(&text),
        [
            "a: a `person` state needs a `question`: what is being decided (V8)",
            "a: option `stop` needs a `description`: write it as `stop: { target: done, description: ... }` (V8)",
        ]
    );
}

#[test]
fn model_needs_two_options_and_min_confidence_needs_unsure() {
    let text = format!(
        "{HEAD}  a:\n    invoke:\n      model: {{ question: Go?, min_confidence: 0.5 }}\n    \
         transitions:\n      go: {{ target: done, description: Go. }}\n      error: failed\n  \
         done: {{ final: true }}\n  failed: {{ final: true }}\n"
    );
    assert_eq!(
        problems(&text),
        [
            "a: a `model` state needs at least 2 options (transitions other than `unsure` and `error`), has 1 (V8)",
            "a: a `model` state with `min_confidence` must handle `unsure`, itself or through an ancestor (V8)",
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

// V9: output.

#[test]
fn output_names_a_script_state() {
    let text = format!(
        "{HEAD}  a: {{ invoke: x, transitions: {{ done: b }} }}\n  \
         b:\n    invoke:\n      check: {{ output: a, matches: ok }}\n    transitions: {{ true: c, false: c }}\n  \
         c:\n    invoke:\n      model: {{ question: Go?, output: a }}\n    transitions:\n      \
         go: {{ target: done, description: Go. }}\n      stop: {{ target: done, description: Stop. }}\n  \
         done: {{ final: true }}\n  failed: {{ final: true }}\n"
    );
    assert!(problems(&text).is_empty(), "{:?}", problems(&text));
    let wrong = text
        .replace("output: a, matches", "output: b, matches")
        .replace("output: a }", "output: nowhere }");
    assert_eq!(
        problems(&wrong),
        [
            "b: output `b` is not a state with a script invoke (V9)",
            "c: output `nowhere` is not a state with a script invoke (V9)",
        ]
    );
}

// V10: conditions.

#[test]
fn condition_rules() {
    let text = "name: m\ndescription: d\ndata:\n  max_rounds: { type: int, default: 2 }\n  \
                mode: { type: string, default: fast }\ninitial: a\nstates:\n  \
                a: { invoke: x, transitions: { done: b } }\n  \
                b: { invoke: { check: { visits: w, less_than: { data: rounds } } }, transitions: { true: c, false: c } }\n  \
                c: { invoke: { check: { data: mode, less_than: b } }, transitions: { true: d, false: d } }\n  \
                d: { invoke: { check: { data: max_rounds, equals: fast } }, transitions: { true: e, false: e } }\n  \
                e: { invoke: { check: { output: a, matches: '(' } }, transitions: { true: f, false: f } }\n  \
                f: { invoke: { check: { visits: a, data: mode, equals: 1 } }, transitions: { true: g, false: g } }\n  \
                g: { invoke: { check: { visits: a } }, transitions: { true: h, false: h } }\n  \
                h: { invoke: { check: { output: a, equals: ok } }, transitions: { true: done, false: done } }\n  \
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
            "c: check: `less_than` compares ints and numbers only, not string (V10)",
            "d: check: `data` compares int with string (V10)",
            "e: check: `matches` '(' is not a regular expression: unclosed group (V10)",
            "f: check: a condition has exactly one subject, not `visits` and `data`; use two `check` states in a row (V10)",
            "g: check: `visits` needs one operator: `equals`, `not_equals`, `less_than`, `at_most`, `more_than` or `at_least` (V10)",
            "h: check: `output` takes the operator `matches`, not `equals` (V10)",
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
        "{HEAD}  a: {{ invoke: {{ check: {{ visits: a, less_than: 3 }} }}, transitions: {{ true: a, false: a }} }}\n  \
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
    let child = "name: child\ndescription: d\ndata:\n  n: { type: int, default: 1 }\ninitial: a\n\
                 states:\n  a: { invoke: x, transitions: { done: done } }\n  \
                 done: { final: true }\n  failed: { final: true }\n";
    let text = format!(
        "{HEAD}  a: {{ invoke: {{ machine: {{ name: child, params: {{ n: two, k: 1 }} }} }}, transitions: {{ done: b }} }}\n  \
         b: {{ invoke: {{ machine: nobody }}, transitions: {{ done: c }} }}\n  \
         c:\n    invoke:\n      model: {{ question: Go?, router: nobody, min_confidence: 1.5 }}\n    \
         transitions:\n      \
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
        ]
    );
}

#[test]
fn model_without_router_needs_a_machine_named_router() {
    let text = format!(
        "{HEAD}  a:\n    invoke:\n      model: {{ question: Go? }}\n    transitions:\n      \
         go: {{ target: done, description: Go. }}\n      stop: {{ target: done, description: Stop. }}\n  \
         done: {{ final: true }}\n  failed: {{ final: true }}\n"
    );
    assert!(problems(&text).is_empty(), "{:?}", problems(&text));
    assert_eq!(
        problems_with(&text, &[]),
        ["a: `model` names no `router`, and there is no machine named `router` (V16)"]
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
         b:\n    invoke:\n      person: {{ question: Go?, ask: tell }}\n    transitions:\n      \
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

// V19: old shapes fail with the shape that replaced them.

/// The parse error for state `a` written with `state`, the YAML of its keys after the
/// state's own line.
fn old_shape(state: &str) -> String {
    let text = format!(
        "{HEAD}  s: {{ invoke: s, transitions: {{ done: a }} }}\n  a:\n{state}  \
         done: {{ final: true }}\n  failed: {{ final: true }}\n"
    );
    parse_machine(&text).unwrap_err()
}

#[test]
fn choose_names_model_and_person() {
    assert_eq!(
        old_shape("    invoke: { choose: model, question: Go? }\n    transitions: { go: done, stop: done }\n"),
        "a: choose is not supported: write invoke: { model: { question: ... } } for a model, or invoke: { person: { question: ..., ask: <script> } } for a person (V19)"
    );
}

#[test]
fn input_names_output() {
    let message = "a: input is not supported: name the state whose output is read with output, in the condition ({ output: <state>, matches: ... }) or in the model ({ model: { ..., output: <state> } }) (V19)";
    assert_eq!(
        old_shape("    invoke: { check: { matches: ok }, input: s }\n    transitions: { true: done, false: done }\n"),
        message
    );
    assert_eq!(
        old_shape("    invoke:\n      model: { question: Go?, input: s }\n    transitions: { go: done, stop: done }\n"),
        message
    );
}

#[test]
fn bare_matches_names_output() {
    assert_eq!(
        old_shape("    invoke:\n      check: { matches: ok }\n    transitions: { true: done, false: done }\n"),
        "a: a bare matches is not supported: name the state it reads, { output: <state>, matches: ... } (V19)"
    );
}

#[test]
fn state_level_script_settings_name_the_script_invoke() {
    assert_eq!(
        old_shape("    invoke: s\n    max_attempts: 2\n    transitions: { done: done }\n"),
        "a: max_attempts is not supported: write `attempts: 2` or `attempts: [<value>, …]` inside the script invoke, invoke: { script: { name: <script>, attempts: … } } (V19)"
    );
    assert_eq!(
        old_shape("    invoke: s\n    timeout_s: 60\n    transitions: { done: done }\n"),
        "a: timeout_s on a state is not supported: write timeout: <n>s|m|h|d inside the invoke, invoke: { script: { name: <script>, timeout: <n>s|m|h|d } } (or person: { ..., timeout: <n>s|m|h|d }) (V19)"
    );
}

#[test]
fn old_retry_key_in_an_invoke_names_attempts_with_its_count() {
    assert_eq!(
        old_shape("    invoke: { script: { name: s, max_attempts: 2 } }\n    transitions: { done: done }\n"),
        "a: max_attempts is not supported: write `attempts: 2` or `attempts: [<value>, …]` (V19)"
    );
    assert_eq!(
        old_shape("    invoke: { model: { question: Q, max_attempts: x } }\n    transitions: { done: done }\n"),
        "a: max_attempts is not supported: write `attempts: <n>` or `attempts: [<value>, …]` (V19)"
    );
}

#[test]
fn attempts_is_a_count_or_a_list_of_values() {
    let text = format!(
        "{HEAD}  a:\n    invoke:\n      script: {{ name: s, attempts: [local, qwen3:8b, claude-opus-5-5, local] }}\n    \
         transitions: {{ done: done }}\n  done: {{ final: true }}\n  failed: {{ final: true }}\n"
    );
    let m = load("m", &text);
    let a = m.find("a").unwrap();
    assert_eq!(m.attempt_count(a), 4);
    let values: Vec<_> = (1..=5).map(|k| m.attempt_value(a, k)).collect();
    assert_eq!(
        values,
        [
            Some("local"),
            Some("qwen3:8b"),
            Some("claude-opus-5-5"),
            Some("local"),
            None
        ]
    );
    assert_eq!(m.attempt_value(a, 0), None);
}

#[test]
fn attempt_values_follow_the_pattern() {
    for ok in [
        "local",
        "claude-opus-5-5",
        "qwen3:8b",
        "a/b@c.d_e",
        "0",
        &"x".repeat(128),
    ] {
        assert!(is_attempt_value(ok), "{ok}");
    }
    for bad in ["", "-x", ".x", "a b", "é", &"x".repeat(129)] {
        assert!(!is_attempt_value(bad), "{bad}");
    }
}

#[test]
fn machine_with_params_beside_it_names_the_long_form() {
    assert_eq!(
        old_shape("    invoke: { machine: child, params: { n: 1 } }\n    transitions: { done: done }\n"),
        "a: { machine: <name>, params: ... } is not supported: write invoke: { machine: { name: <name>, params: ... } } (V19)"
    );
}

#[test]
fn script_and_machine_take_a_bare_name_or_the_long_form() {
    let text = format!(
        "{HEAD}  a:\n    invoke:\n      script: {{ name: s, attempts: 3, timeout: 9s }}\n    \
         transitions: {{ done: b }}\n  \
         b: {{ invoke: {{ script: s }}, transitions: {{ done: c }} }}\n  \
         c: {{ invoke: {{ machine: {{ name: child, params: {{ n: 1 }} }} }}, transitions: {{ done: done }} }}\n  \
         done: {{ final: true }}\n  failed: {{ final: true }}\n"
    );
    let m = load("m", &text);
    let a = m.find("a").unwrap();
    let Some(Invoke::Script(script)) = &m.nodes[a].invoke else {
        panic!("{:?}", m.nodes[a].invoke);
    };
    assert_eq!(
        (script.name.as_str(), &script.attempts, script.timeout),
        ("s", &Some(Attempts::Count(3)), Some(Duration::from_secs(9)))
    );
    assert_eq!(m.attempt_count(a), 3);
    assert_eq!(m.attempt_value(a, 1), None);
    let b = m.find("b").unwrap();
    assert_eq!(
        m.nodes[b].invoke,
        Some(Invoke::Script(ScriptInvoke {
            name: "s".into(),
            attempts: None,
            timeout: None,
            env: BTreeMap::new(),
        }))
    );
    assert_eq!(m.attempt_count(b), 1);
    let Some(Invoke::Machine(child)) = &m.nodes[m.find("c").unwrap()].invoke else {
        panic!();
    };
    assert_eq!(child.name, "child");
    assert_eq!(child.params.len(), 1);
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
        "{HEAD}  a:\n    invoke:\n      model: {{ question: Go? }}\n    transitions:\n      \
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
    assert!(DataType::Number.matches(&Value::from(1.5)));
    assert!(DataType::Number.matches(&Value::from(2)));
    assert!(!DataType::Number.matches(&Value::from("x")));
    assert!(!DataType::Number.matches(&Value::from(f64::NAN)));
    assert!(!DataType::Number.matches(&Value::from(f64::INFINITY)));
}

#[test]
fn param_problems_name_the_param_the_value_and_the_allowed_values() {
    use serde_norway::Value;
    let m = load_machine_text(
        "m",
        "name: m\ndescription: d\ndata:\n  need: { type: string, enum: [plan, fix], default: fix }\n  \
         megapixels: { type: number, default: 1.0 }\ninitial: x\nstates:\n  x: { final: true }\n",
    )
    .unwrap();
    let need = &m.data["need"];
    assert_eq!(need.param_problem("need", &Value::from("plan")), None);
    assert_eq!(
        need.param_problem("need", &Value::from("review"))
            .as_deref(),
        Some("param `need` is `review`, not one of the allowed values: plan, fix")
    );
    assert_eq!(
        need.param_problem("need", &Value::from(1)).as_deref(),
        Some("param `need` must be of type `string`")
    );
    let megapixels = &m.data["megapixels"];
    assert_eq!(megapixels.kind, DataType::Number);
    assert_eq!(
        megapixels.param_problem("megapixels", &Value::from(1.5)),
        None
    );
    assert_eq!(
        megapixels.param_problem("megapixels", &Value::from(2)),
        None
    );
    assert_eq!(
        megapixels
            .param_problem("megapixels", &Value::from("x"))
            .as_deref(),
        Some("param `megapixels` must be of type `number`")
    );
}

#[test]
fn script_problems_name_the_state_and_relative_path() {
    let text = format!(
        "{HEAD}  a: {{ invoke: x, transitions: {{ done: b }} }}\n  \
         b:\n    invoke:\n      person: {{ question: Go? }}\n    transitions:\n      \
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
                message:
                    "a `person` state needs an `ask` script, which tells someone how to reply (V12)"
                        .into(),
            },
            Problem {
                at: "a".into(),
                message: "script `x` not found; searched scripts/m, scripts (V12)".into(),
            },
        ]
    );
}

#[test]
fn attempts_of_another_shape_name_both_forms() {
    for (attempts, expected) in [
        ("-1", "`attempts` is a positive integer or a list of values, `attempts: [<value>, …]`, not -1"),
        ("local", "`attempts` is a positive integer or a list of values, `attempts: [<value>, …]`"),
        ("[local, 3]", "`attempts` is a positive integer or a list of values, `attempts: [<value>, …]`: each value is a string, not 3"),
    ] {
        let text = format!(
            "{HEAD}  a:\n    invoke: {{ script: {{ name: s, attempts: {attempts} }} }}\n    \
             transitions: {{ done: done }}\n  done: {{ final: true }}\n  failed: {{ final: true }}\n"
        );
        let err = load_err("m", &text);
        assert!(err.contains(expected), "{attempts}: {err}");
        assert!(err.contains(": a: "), "{attempts}: {err}");
    }
}

/// An invoke's `env` keeps every value as a string; a float or a list is a parse error.
#[test]
fn invoke_env_values_are_strings_ints_or_bools() {
    let text = "name: m\ndescription: d\ninitial: s\nstates:\n  s:\n    invoke: { script: { name: build, env: { METHOD: image_text, STEPS: 30, FAST: true } } }\n    transitions: { done: done }\n  done: { final: true }\n  failed: { final: true }\n";
    let m = load_machine_text("m", text).unwrap();
    let script = m.nodes[m.find("s").unwrap()]
        .invoke
        .as_ref()
        .unwrap()
        .script()
        .unwrap();
    let env: Vec<(&str, &str)> = script
        .env
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(
        env,
        vec![("FAST", "true"), ("METHOD", "image_text"), ("STEPS", "30")]
    );
    for bad in ["1.5", "[a]"] {
        let text = text.replace("30", bad);
        let err = load_machine_text("m", &text).unwrap_err().to_string();
        assert!(
            err.contains("`STEPS` is not a string, int or bool"),
            "{err}"
        );
    }
}
