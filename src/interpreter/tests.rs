use super::child::continue_run;
use super::decide::{Reply, CHOICES_FILE};
use super::recover::{recover, reject, run_status, Recovery, RunStatus};
use super::*;
use crate::events::EventLog;
use crate::layout::INBOX_DIR;
use crate::machine::load_machine_text;
use crate::machine::validate::CheckEnv;
use crate::message::{create_run_dir, lock_state, LockState};
use crate::runtime::executor_tests::install;
use serde_json::Map;
use std::collections::{BTreeSet, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tempfile::TempDir;

const RUN_ID: &str = "20261001T143005Z-3fa9c1";
const BODY: &str = "# Task\r\nDo the thing.\n";

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A temp project holding fixture machine `tests/fixtures/machines/step/<name>.yml` (or
/// several machines, the first of which the run uses), a run folder with its
/// `message.md`, and every script the machines name: `record.sh` installed under that
/// name, unless `scripts` maps the name to another fixture.
struct Project {
    tmp: TempDir,
    name: String,
    machines: BTreeMap<String, LoadedMachine>,
    shutdown: Arc<AtomicBool>,
}

/// Every script machine `m` names: invokes, `ask` scripts, `onentry` and `onexit`.
fn script_names(m: &LoadedMachine) -> Vec<&str> {
    let mut names = Vec::new();
    for node in &m.nodes {
        match &node.invoke {
            Some(Invoke::Script(name)) => names.push(name.as_str()),
            Some(Invoke::Choose(c)) => names.extend(c.ask.as_deref()),
            _ => {}
        }
        names.extend(node.onentry.iter().chain(&node.onexit).map(String::as_str));
    }
    names
}

impl Project {
    fn new(name: &str, scripts: &[(&str, &str)]) -> Self {
        let fixture = repo().join(format!("tests/fixtures/machines/step/{name}.yml"));
        let text = fs::read_to_string(&fixture).unwrap();
        Self::from_text(name, &text, scripts)
    }

    /// The same, for machine `name` written as `text`.
    fn from_text(name: &str, text: &str, scripts: &[(&str, &str)]) -> Self {
        Self::from_texts(&[(name, text)], scripts)
    }

    /// The same, for several machines: the run uses the first.
    fn from_texts(machines: &[(&str, &str)], scripts: &[(&str, &str)]) -> Self {
        let tmp = TempDir::new().unwrap();
        let decree = tmp.path().join(DECREE_DIR);
        let loaded: BTreeMap<String, LoadedMachine> = machines
            .iter()
            .map(|(name, text)| {
                let m = load_machine_text(name, text).unwrap();
                (name.to_string(), m)
            })
            .collect();

        let script_dir = decree.join("scripts");
        fs::create_dir_all(&script_dir).unwrap();
        let fixtures = repo().join("tests/fixtures/scripts");
        for script in loaded.values().flat_map(script_names) {
            let file = scripts
                .iter()
                .find(|(s, _)| *s == script)
                .map_or("record", |(_, f)| f);
            install(
                &fixtures.join(format!("{file}.sh")),
                &script_dir.join(format!("{script}.sh")),
            );
        }
        // Every fixture machine passes `decree check`.
        let ids: BTreeSet<String> = loaded.keys().cloned().collect();
        let env = CheckEnv {
            decree_dir: &decree,
            machine_ids: &ids,
            machines: &loaded,
        };
        for (name, text) in machines {
            let problems = loaded[*name].validate(text, &env);
            assert!(problems.is_empty(), "{name}: {problems:?}");
        }

        let name = machines[0].0;
        let project = Project {
            tmp,
            name: name.to_string(),
            machines: loaded,
            shutdown: Arc::new(AtomicBool::new(false)),
        };
        fs::create_dir_all(project.run_dir()).unwrap();
        let message = format!("---\nid: {RUN_ID}\nmachine: {name}\ntrigger: inbox\n---\n{BODY}");
        fs::write(project.run_dir().join(MESSAGE_FILE), message).unwrap();
        project
    }

    /// The machine the run uses.
    fn machine(&self) -> &LoadedMachine {
        &self.machines[&self.name]
    }

    fn ctx(&self) -> Context<'_> {
        Context {
            project_root: self.root(),
            machines: &self.machines,
            shutdown: Arc::clone(&self.shutdown),
        }
    }

    fn root(&self) -> PathBuf {
        self.tmp.path().to_path_buf()
    }

    fn run_dir(&self) -> PathBuf {
        self.root().join(".decree/runs").join(RUN_ID)
    }

    fn executor(&self, trigger: &str, params: &serde_norway::Mapping) -> Executor {
        self.ctx()
            .executor(self.machine(), RUN_ID, trigger, params, None)
            .unwrap()
    }

    fn run(&self) -> Outcome {
        self.run_with("inbox", "inbox.md", &serde_norway::Mapping::new())
    }

    fn run_with(&self, trigger: &str, file: &str, params: &serde_norway::Mapping) -> Outcome {
        self.start(trigger, file, params).unwrap()
    }

    fn start(
        &self,
        trigger: &str,
        file: &str,
        params: &serde_norway::Mapping,
    ) -> Result<Outcome, InterpreterError> {
        let input = RunInput {
            params: params.clone(),
            message_body: BODY.to_string(),
            file: Some(file.to_string()),
            depth: 0,
        };
        let ctx = self.ctx();
        Interpreter::new(&ctx, self.machine(), self.executor(trigger, params), input)?.start()
    }

    /// The log of the first `script` event of `script`.
    fn log_of(&self, script: &str) -> String {
        let event = self
            .events_of("script")
            .into_iter()
            .find(|e| e["script"] == script)
            .unwrap_or_else(|| panic!("no script event for {script}"));
        fs::read_to_string(self.run_dir().join(event["log"].as_str().unwrap())).unwrap()
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

/// The input of an inbox message `inbox.md` with no params.
fn inbox_input() -> RunInput {
    RunInput {
        message_body: BODY.to_string(),
        file: Some("inbox.md".to_string()),
        ..RunInput::default()
    }
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
// Exit and entry order (docs/reference/runs.md, Step loop)
// ---------------------------------------------------------------

#[test]
fn order_normal_path_to_a_final_state() {
    let p = Project::new("step_normal", &[]);
    assert_eq!(p.run(), Outcome::Finished("done".into()));
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
    assert_eq!(run_status(p.machine(), &events, false), RunStatus::Finished);
}

#[test]
fn order_self_transition_exits_and_reenters_the_state() {
    let p = Project::new("step_self", &[]);
    assert_eq!(p.run(), Outcome::Finished("done".into()));
    assert_eq!(
        p.order(),
        [
            "root_entry",
            "a_entry",
            "a_exit",
            "a_entry",
            "a_exit",
            "done_entry",
            "root_exit"
        ]
    );
    assert_eq!(
        p.transitions(),
        ["- claimed a claim", "a yes a check", "a no done check"]
    );
    assert_eq!(visits(&p.events())["a"], 2);
}

#[test]
fn order_entering_and_leaving_a_compound_state() {
    let p = Project::new("step_compound", &[]);
    assert_eq!(p.run(), Outcome::Finished("done".into()));
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
    assert_eq!(p.run(), Outcome::Finished("failed".into()));
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
fn order_onentry_failure_stops_its_own_block_and_error_is_selected_from_the_atomic_state() {
    let p = Project::new("step_entry_fail", &[]);
    assert_eq!(p.run(), Outcome::Finished("done".into()));
    // `outer_entry_fail` skips `outer_entry` only: `inner` is still entered, then
    // `error` is selected from `inner` (not `outer`'s `error: failed`), and `work`
    // never runs.
    assert_eq!(
        p.order(),
        [
            "root_entry",
            "outer_entry_fail",
            "inner_entry",
            "inner_exit",
            "outer_exit",
            "cleanup_entry",
            "done_entry",
            "root_exit"
        ]
    );
    assert_eq!(
        p.transitions(),
        [
            "- claimed start claim",
            "start done inner exit_code",
            "inner error cleanup exit_code",
            "cleanup done done exit_code"
        ]
    );
    // No invoke ran, so no exit code.
    assert_eq!(p.events_of("transition")[2]["exit_code"], Value::Null);
}

#[test]
fn order_root_onentry_failure_is_error_selected_from_the_atomic_state() {
    let p = Project::new("step_root_entry_fail", &[]);
    assert_eq!(p.run(), Outcome::Finished("done".into()));
    // The root block stops; `a` is still entered, and its `error` transition is taken
    // without running its invoke.
    assert_eq!(
        p.order(),
        [
            "root_entry_fail",
            "a_entry",
            "a_exit",
            "cleanup_entry",
            "done_entry",
            "root_exit"
        ]
    );
    assert_eq!(
        p.transitions(),
        [
            "- claimed a claim",
            "a error cleanup exit_code",
            "cleanup done done exit_code"
        ]
    );
    assert_eq!(p.events_of("transition")[1]["exit_code"], Value::Null);
}

#[test]
fn order_root_onentry_failure_unhandled_goes_to_failed() {
    let p = Project::new("step_root_entry_fail_unhandled", &[]);
    assert_eq!(p.run(), Outcome::Finished("failed".into()));
    assert_eq!(
        p.order(),
        [
            "root_entry_fail",
            "a_entry",
            "a_exit",
            "failed_entry",
            "root_exit"
        ]
    );
    assert_eq!(
        p.transitions(),
        ["- claimed a claim", "a error failed exit_code"]
    );
    assert_eq!(mirrored_state(&p), "failed");
}

#[test]
fn order_onexit_failure_is_recorded_and_changes_nothing() {
    let p = Project::new("step_exit_fail", &[]);
    assert_eq!(p.run(), Outcome::Finished("done".into()));
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
    assert_eq!(p.run(), Outcome::Finished("failed".into()));
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
    assert_eq!(p.run(), Outcome::Finished("failed".into()));
    assert_eq!(
        p.order(),
        // docs/reference/scripts.md: the remaining `onentry` scripts are skipped; the run still ends.
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
// Migrations: the ledger line (docs/reference/messages.md, rule 5; docs/reference/runs.md, step 7)
// ---------------------------------------------------------------

#[test]
fn migration_ledger_line_is_written_before_final_onentry() {
    let p = Project::new("step_normal", &[("done_entry", "copy_ledger")]);
    fs::write(p.root().join(".decree/processed.md"), "44-prev.md").unwrap();
    let outcome = p.run_with("migration", "45-next.md", &serde_norway::Mapping::new());
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
    let outcome = p.run_with("migration", "45-next.md", &serde_norway::Mapping::new());
    assert_eq!(outcome, Outcome::Finished("failed".into()));
    assert_eq!(p.processed(), "44-prev.md\n");
}

#[test]
fn failed_migration_writes_no_ledger_line() {
    let p = Project::new("step_error", &[]);
    p.run_with("migration", "45-next.md", &serde_norway::Mapping::new());
    assert_eq!(p.processed(), "");
}

// ---------------------------------------------------------------
// Attempts and visits
// ---------------------------------------------------------------

#[test]
fn invoke_failing_twice_then_succeeding_takes_done_after_two_attempts() {
    let p = Project::new("step_attempts", &[("fail_until_final", "fail_until_final")]);
    assert_eq!(p.run(), Outcome::Finished("done".into()));
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
fn visits_check_ends_a_retry_loop_after_two_visits() {
    let p = Project::new(
        "step_retry_loop",
        &[
            ("fail_until_final", "fail_until_final"),
            ("verify", "exit_zero"),
        ],
    );
    assert_eq!(p.run(), Outcome::Finished("done".into()));
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
            "verify done rounds_left exit_code",
            "rounds_left yes implement check",
            "implement error implement attempt",
            "implement done verify exit_code",
            "verify done rounds_left exit_code",
            "rounds_left no done check"
        ]
    );
    let decisions = p.events_of("decision");
    assert_eq!(decisions.len(), 2);
    assert_eq!(decisions[1]["event"], "no");
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
// Check (docs/reference/runs.md, Check)
// ---------------------------------------------------------------

/// Machine `step_check`: `work` runs `script` (a fixture name), then `decide` checks
/// `condition` (YAML flow mapping), with `input: work` if `input` is set.
fn check_project(condition: &str, input: bool, script: &str) -> Project {
    let input = if input { ", input: work" } else { "" };
    let text = format!(
        "name: step_check\ndescription: Run a script, then check a condition.\n\
         data:\n  max_rounds: {{ type: int, default: 2 }}\n  limit: {{ type: int, default: 1 }}\n  \
         mode: {{ type: string, default: fast }}\n  strict: {{ type: bool, default: true }}\n\
         initial: work\nstates:\n  \
         work: {{ invoke: work, transitions: {{ done: decide }} }}\n  \
         decide:\n    invoke: {{ check: {condition}{input} }}\n    transitions: {{ yes: passed, no: refused }}\n  \
         passed: {{ final: true }}\n  refused: {{ final: true }}\n  failed: {{ final: true }}\n"
    );
    Project::from_text("step_check", &text, &[("work", script)])
}

/// The event a check produces, from its `decision` event, after a clean run.
fn check_event(condition: &str, params_yaml: &str) -> String {
    let p = check_project(condition, false, "exit_zero");
    let outcome = p.run_with("inbox", "inbox.md", &params(params_yaml));
    let decision = &p.events_of("decision")[0];
    let event = decision["event"].as_str().unwrap().to_string();
    let expected = if event == "yes" { "passed" } else { "refused" };
    assert_eq!(outcome, Outcome::Finished(expected.into()), "{condition}");
    event
}

#[test]
fn check_each_operator_on_visits() {
    // `work` has been entered once when `decide` runs.
    for (op, yes, no) in [
        ("equals", 1, 2),
        ("not_equals", 2, 1),
        ("less_than", 2, 1),
        ("at_most", 1, 0),
        ("more_than", 0, 1),
        ("at_least", 1, 2),
    ] {
        let cond = |n: i32| format!("{{ visits: work, {op}: {n} }}");
        assert_eq!(check_event(&cond(yes), "{}"), "yes", "{op} {yes}");
        assert_eq!(check_event(&cond(no), "{}"), "no", "{op} {no}");
    }
}

#[test]
fn check_each_operator_on_data() {
    for (cond, yes) in [
        ("{ data: mode, equals: fast }", true),
        ("{ data: mode, not_equals: fast }", false),
        ("{ data: strict, equals: true }", true),
        ("{ data: strict, not_equals: true }", false),
        ("{ data: max_rounds, less_than: 3 }", true),
        ("{ data: max_rounds, at_most: 1 }", false),
        ("{ data: max_rounds, more_than: 1 }", true),
        ("{ data: max_rounds, at_least: 3 }", false),
    ] {
        let want = if yes { "yes" } else { "no" };
        assert_eq!(check_event(cond, "{}"), want, "{cond}");
    }
}

#[test]
fn check_compares_with_a_data_value_set_by_params() {
    let cond = "{ visits: work, less_than: { data: max_rounds } }";
    assert_eq!(check_event(cond, "{}"), "yes");
    assert_eq!(check_event(cond, "max_rounds: 1"), "no");
    let cond = "{ data: limit, equals: { data: max_rounds } }";
    assert_eq!(check_event(cond, "{}"), "no");
    assert_eq!(check_event(cond, "limit: 2"), "yes");
}

#[test]
fn check_matches_reads_the_input_states_output() {
    // `exit_zero` prints `hello`; `stderr` writes `[stderr] to stderr` to its log.
    for (script, cond, want) in [
        ("exit_zero", "{ matches: '(?m)^hello$' }", "yes"),
        ("exit_zero", "{ matches: hel+o }", "yes"),
        ("exit_zero", "{ matches: '(?m)^bye$' }", "no"),
        (
            "stderr",
            "{ matches: '(?m)^\\[stderr\\] to stderr$' }",
            "yes",
        ),
    ] {
        let p = check_project(cond, true, script);
        p.run();
        let decision = &p.events_of("decision")[0];
        assert_eq!(decision["event"], want, "{script} {cond}");
    }
}

#[test]
fn check_matches_without_input_reads_the_most_recent_invoke() {
    let p = check_project("{ matches: '^hello' }", false, "exit_zero");
    assert_eq!(p.run(), Outcome::Finished("passed".into()));
}

#[test]
fn check_data_matches_tests_the_string_value_set_by_params() {
    let text = "name: step_file\ndescription: Check a file name.\n\
                data:\n  file: { type: string, default: \"\" }\ninitial: decide\nstates:\n  \
                decide:\n    invoke: { check: { data: file, matches: '\\.md$' } }\n    \
                transitions: { yes: passed, no: refused }\n  \
                passed: { final: true }\n  refused: { final: true }\n  failed: { final: true }\n";
    for (file, want, end) in [
        ("notes/a.md", "yes", "passed"),
        ("notes/a.txt", "no", "refused"),
    ] {
        let p = Project::from_text("step_file", text, &[]);
        let outcome = p.run_with("inbox", "inbox.md", &params(&format!("file: {file}")));
        assert_eq!(outcome, Outcome::Finished(end.into()), "{file}");
        let decision = &p.events_of("decision")[0];
        assert_eq!(decision["event"], want, "{file}");
        assert_eq!(
            decision["condition"],
            json!({ "data": "file", "matches": "\\.md$" })
        );
    }
}

/// Machine `step_confidence`: `worth_asking` checks `big_model`'s confidence. `gate`
/// skips `big_model`, which only exists so the condition names a `choose: model` state;
/// instead the run's `events.jsonl` starts with `decision`, a decision event of it.
fn confidence_project(decision: Value) -> Project {
    let text = "name: step_confidence\ndescription: Check a model's confidence.\n\
                initial: gate\nstates:\n  \
                gate:\n    invoke: { check: { visits: big_model, equals: 0 } }\n    \
                transitions: { yes: worth_asking, no: big_model }\n  \
                worth_asking:\n    invoke: { check: { confidence: big_model, at_least: 0.4 } }\n    \
                transitions: { yes: ask_person, no: set_aside }\n  \
                big_model:\n    invoke: { choose: model, router: router, question: \"Which kind?\", min_confidence: 0.7 }\n    \
                transitions:\n      \
                invoice: { target: ask_person, description: A bill. }\n      \
                receipt: { target: set_aside, description: A paid bill. }\n      \
                unsure: { target: worth_asking }\n  \
                ask_person: { final: true }\n  set_aside: { final: true }\n  failed: { final: true }\n";
    let router = "name: router\ndescription: A router.\ninitial: ask\nstates:\n  \
                  ask: { invoke: ask, transitions: { done: done } }\n  \
                  done: { final: true }\n  failed: { final: true }\n";
    let p = Project::from_texts(&[("step_confidence", text), ("router", router)], &[]);
    let mut event = json!({
        "v": 1, "seq": 1, "ts": "2026-10-01T17:04:12.000Z", "type": "decision",
        "run_id": RUN_ID, "machine": "step_confidence", "trigger": "inbox",
        "state": "big_model", "kind": "model", "event": "unsure",
        "options": ["invoice", "receipt"], "router": "router",
    });
    event
        .as_object_mut()
        .unwrap()
        .extend(decision.as_object().unwrap().clone());
    fs::write(p.run_dir().join(EVENTS_FILE), format!("{event}\n")).unwrap();
    p
}

#[test]
fn check_confidence_reads_the_latest_decision_of_the_state() {
    for (decision, want, end) in [
        (
            json!({ "pick": "invoice", "confidence": 0.55 }),
            "yes",
            "ask_person",
        ),
        (json!({}), "no", "set_aside"),
    ] {
        let p = confidence_project(decision.clone());
        assert_eq!(p.run(), Outcome::Finished(end.into()), "{decision}");
        let checks: Vec<_> = p
            .events_of("decision")
            .into_iter()
            .filter(|d| d["state"] == "worth_asking")
            .collect();
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0]["event"], want, "{decision}");
        assert_eq!(
            checks[0]["condition"],
            json!({ "confidence": "big_model", "at_least": 0.4 })
        );
    }
}

#[test]
fn check_appends_a_decision_before_its_transition() {
    let p = check_project(
        "{ visits: work, less_than: { data: max_rounds } }",
        false,
        "exit_zero",
    );
    assert_eq!(p.run(), Outcome::Finished("passed".into()));
    let events = p.events();
    let i = events.iter().position(|e| e["type"] == "decision").unwrap();
    let d = &events[i];
    assert_eq!(d["state"], "decide");
    assert_eq!(d["kind"], "check");
    assert_eq!(d["event"], "yes");
    assert_eq!(
        d["condition"],
        json!({ "visits": "work", "less_than": { "data": "max_rounds" } })
    );
    let t = &events[i + 1];
    assert_eq!(t["type"], "transition");
    assert_eq!(
        (
            &t["from"],
            &t["event"],
            &t["to"],
            &t["source"],
            &t["exit_code"]
        ),
        (
            &json!("decide"),
            &json!("yes"),
            &json!("passed"),
            &json!("check"),
            &Value::Null
        )
    );
    // No script ran for the check.
    assert!(p.events_of("script").iter().all(|e| e["state"] != "decide"));
}

// ---------------------------------------------------------------
// Other events
// ---------------------------------------------------------------

#[test]
fn undeclared_printed_event_becomes_error_with_invalid_event() {
    let p = Project::new("step_normal", &[("a_invoke", "print_undeclared")]);
    assert_eq!(p.run(), Outcome::Finished("failed".into()));
    let t = &p.events_of("transition")[1];
    assert_eq!(t["event"], "error");
    assert_eq!(t["source"], "stdout");
    assert_eq!(t["invalid_event"], "nope");
    assert_eq!(t["to"], "failed");
}

#[test]
fn timed_out_invoke_gives_error() {
    let p = Project::new("step_timeout", &[("sleep_long", "sleep_long")]);
    assert_eq!(p.run(), Outcome::Finished("failed".into()));
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
    assert_eq!(p.run(), Outcome::Interrupted("a".into()));
    let events = p.events();
    let last = events.last().unwrap();
    assert_eq!(last["type"], "interrupted");
    assert_eq!(last["state"], "a");
    assert_eq!(last["cause"], "signal");
    assert_eq!(last["script"], "root_entry");
    assert!(p.order().is_empty());
    assert_eq!(
        run_status(p.machine(), &events, false),
        RunStatus::Interrupted
    );
}

// ---------------------------------------------------------------
// Interrupts, run status and the run lock
// ---------------------------------------------------------------

fn append(p: &Project, kind: &str, fields: Value) {
    let mut log = EventLog::open(&p.run_dir(), RUN_ID, &p.name, "inbox").unwrap();
    log.append(kind, fields).unwrap();
}

/// The `transition` event `decree retry` writes (docs/reference/cli.md) back into `state`.
fn retry(p: &Project, state: &str) {
    let fields = json!({
        "from": state, "event": "retry", "to": state, "source": "retry", "exit_code": null
    });
    append(p, "transition", fields);
}

fn status_of(p: &Project) -> RunStatus {
    let alive = matches!(lock_state(&p.run_dir()).unwrap(), LockState::Live(_));
    p.ctx().status(p.machine(), &p.events(), alive)
}

/// A run of `step_compound` interrupted by a signal in `inner`, below `outer`.
fn interrupted_compound() -> Project {
    let p = Project::new("step_compound", &[]);
    p.shutdown.store(true, Ordering::SeqCst);
    assert_eq!(p.run(), Outcome::Interrupted("inner".into()));
    p.shutdown.store(false, Ordering::SeqCst);
    p
}

#[test]
fn lock_is_deleted_when_the_run_finishes_or_is_interrupted() {
    let p = Project::new("step_normal", &[]);
    assert_eq!(p.run(), Outcome::Finished("done".into()));
    assert!(!p.run_dir().join(LOCK_FILE).exists());
    // Interrupted by a signal: deleted too.
    let p = interrupted_compound();
    assert!(!p.run_dir().join(LOCK_FILE).exists());
}

#[test]
fn lock_of_a_waiting_run_is_released() {
    let (p, _) = person_project(&[]);
    assert!(matches!(p.run(), Outcome::Waiting { .. }));
    assert!(!p.run_dir().join(LOCK_FILE).exists());
    assert_eq!(status_of(&p), RunStatus::Waiting);
}

#[test]
fn retry_reruns_root_and_state_onentry_then_continues_at_the_recorded_state() {
    let p = interrupted_compound();
    assert_eq!(status_of(&p), RunStatus::Interrupted);
    retry(&p, "inner");
    assert_eq!(status_of(&p), RunStatus::Pending);
    let outcome = continue_run(&p.ctx(), RUN_ID).unwrap();
    assert_eq!(outcome, Outcome::Finished("done".into()));
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
    assert_eq!(
        p.transitions(),
        [
            "- claimed inner claim",
            "inner retry inner retry",
            "inner done after exit_code",
            "after done done exit_code"
        ]
    );
    assert!(!p.run_dir().join(LOCK_FILE).exists());
}

#[test]
fn retried_run_whose_onentry_fails_takes_error() {
    let p = interrupted_compound();
    retry(&p, "inner");
    install(
        &repo().join("tests/fixtures/scripts/exit_three.sh"),
        &p.root().join(".decree/scripts/inner_entry.sh"),
    );
    let outcome = continue_run(&p.ctx(), RUN_ID).unwrap();
    assert_eq!(outcome, Outcome::Finished("failed".into()));
    assert_eq!(
        p.transitions().last().unwrap(),
        "inner error failed exit_code"
    );
}

#[test]
fn recover_marks_a_crashed_run_once_and_never_continues_it() {
    let p = Project::new("step_normal", &[]);
    // A crash after the claim event: a stale lock, or none.
    append(
        &p,
        "transition",
        json!({"from": null, "event": "claimed", "to": "a", "source": "claim", "exit_code": null}),
    );
    fs::write(p.run_dir().join(LOCK_FILE), "999999999").unwrap();
    let found = recover(&p.ctx()).unwrap();
    assert_eq!(found.crashed, [(RUN_ID.to_string(), "a".to_string())]);
    assert!(found.pending.is_empty());
    let events = p.events();
    let last = events.last().unwrap();
    assert_eq!(last["type"], "interrupted");
    assert_eq!(last["cause"], "crash");
    assert_eq!(last["state"], "a");
    assert_eq!(last["seq"], 2);
    assert!(p.order().is_empty());

    // Already marked: nothing more is appended.
    assert_eq!(recover(&p.ctx()).unwrap(), Recovery::default());
    assert_eq!(p.events().len(), 2);

    // `decree retry` makes it pending; the stale lock does not stop it continuing.
    retry(&p, "a");
    let found = recover(&p.ctx()).unwrap();
    assert_eq!(found.pending, [RUN_ID]);
    assert_eq!(
        continue_run(&p.ctx(), RUN_ID).unwrap(),
        Outcome::Finished("done".into())
    );
    assert_eq!(p.order()[..2], ["root_entry", "a_entry"]);
}

#[test]
fn recover_leaves_an_active_run_alone_and_resume_refuses_it() {
    let p = Project::new("step_normal", &[]);
    append(
        &p,
        "transition",
        json!({"from": null, "event": "claimed", "to": "a", "source": "claim", "exit_code": null}),
    );
    let mut holder = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    fs::write(p.run_dir().join(LOCK_FILE), holder.id().to_string()).unwrap();
    assert_eq!(status_of(&p), RunStatus::Active);
    assert_eq!(recover(&p.ctx()).unwrap(), Recovery::default());
    assert_eq!(p.events().len(), 1);

    retry(&p, "a");
    assert_eq!(recover(&p.ctx()).unwrap(), Recovery::default());
    let err = continue_run(&p.ctx(), RUN_ID).unwrap_err();
    assert!(
        matches!(&err, InterpreterError::Active(id) if id == RUN_ID),
        "{err}"
    );
    assert!(p.order().is_empty());
    assert_eq!(
        fs::read_to_string(p.run_dir().join(LOCK_FILE)).unwrap(),
        holder.id().to_string()
    );
    holder.kill().unwrap();
    holder.wait().unwrap();
}

#[test]
fn recover_does_not_mark_a_waiting_run_without_a_lock() {
    let (p, _) = person_project(&[]);
    assert!(matches!(p.run(), Outcome::Waiting { .. }));
    let before = p.events().len();
    assert_eq!(recover(&p.ctx()).unwrap(), Recovery::default());
    assert_eq!(p.events().len(), before);
    assert_eq!(status_of(&p), RunStatus::Waiting);
}

#[test]
fn recover_and_continue_rewrite_a_mirror_that_disagrees_with_the_events() {
    let p = interrupted_compound();
    let path = p.run_dir().join(MESSAGE_FILE);
    mirror_state(&path, "after").unwrap();
    recover(&p.ctx()).unwrap();
    assert_eq!(mirrored_state(&p), "inner");
    assert!(p.message().ends_with(BODY));

    // A crash between the `transition` event and the mirror write.
    retry(&p, "inner");
    mirror_state(&path, "after").unwrap();
    let ctx = Context {
        shutdown: Arc::new(AtomicBool::new(true)),
        ..p.ctx()
    };
    assert_eq!(
        continue_run(&ctx, RUN_ID).unwrap(),
        Outcome::Interrupted("inner".into())
    );
    assert_eq!(mirrored_state(&p), "inner");
}

#[test]
fn repair_mirror_leaves_an_unparsable_message_unchanged() {
    let p = interrupted_compound();
    let path = p.run_dir().join(MESSAGE_FILE);
    fs::write(
        &path,
        "---
machine: [
",
    )
    .unwrap();
    assert!(!repair_mirror(&p.run_dir(), &p.events()).unwrap());
    assert_eq!(fs::read_to_string(&path).unwrap(), "---\nmachine: [\n");
}

// ---------------------------------------------------------------
// Composition: bubbling, `type: internal`, nested final states
// ---------------------------------------------------------------

#[test]
fn composition_unhandled_event_is_taken_by_the_nearest_ancestor() {
    let p = Project::new("step_bubble", &[("a_invoke", "print_pass")]);
    assert_eq!(p.run(), Outcome::Finished("done".into()));
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
    assert_eq!(p.run(), Outcome::Finished("done".into()));
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
    assert_eq!(p.run(), Outcome::Finished("done".into()));
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
    assert_eq!(p.run(), Outcome::Finished("done".into()));
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
        run_status(p.machine(), &events[..=at + 1], false),
        RunStatus::Interrupted
    );
    assert_eq!(visits(&events)["finished"], 1);
}

#[test]
fn nested_final_state_writes_no_ledger_line() {
    let p = Project::new("step_nested_final", &[("after_invoke", "copy_ledger")]);
    fs::write(p.root().join(".decree/processed.md"), "45-prev.md\n").unwrap();
    let outcome = p.run_with("migration", "46-next.md", &serde_norway::Mapping::new());
    assert_eq!(outcome, Outcome::Finished("done".into()));
    // `copy_ledger` ran after `finished` was entered, before the root `done`.
    let seen = fs::read_to_string(p.root().join("ledger.txt")).unwrap();
    assert_eq!(seen, "45-prev.md\n");
    assert_eq!(p.processed(), "45-prev.md\n46-next.md\n");
}

#[test]
fn nested_final_onentry_failure_is_error_resolved_from_that_state() {
    let p = Project::new("step_nested_final", &[("finished_entry", "exit_three")]);
    assert_eq!(p.run(), Outcome::Finished("recovered".into()));
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
// Choose: person, interpreter side (docs/reference/messages.md, Replies; docs/reference/runs.md)
// ---------------------------------------------------------------

/// The `step_person` project, run until it waits. `ask_person` prints its environment.
fn person_project(scripts: &[(&str, &str)]) -> (Project, String) {
    let mut all = vec![("ask_person", "print_env")];
    all.extend_from_slice(scripts);
    let p = Project::new("step_person", &all);
    let outcome = p.run();
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

/// Append a `received` event, as reply delivery does, then continue the run.
fn receive(p: &Project, fields: Value) -> Outcome {
    let mut executor = p.executor("inbox", &serde_norway::Mapping::new());
    executor.events().append("received", fields).unwrap();
    let ctx = p.ctx();
    Interpreter::new(&ctx, p.machine(), executor, inbox_input())
        .unwrap()
        .resume()
        .unwrap()
}

#[test]
fn person_ask_script_sees_the_wait_id_and_choices_then_the_run_waits() {
    let before = Utc::now();
    let (p, wait_id) = person_project(&[]);
    assert_eq!(
        p.order(),
        ["root_entry", "build_invoke", "gate_entry", "approval_entry"]
    );
    let log = p.log_of("ask_person");
    let choices = p.run_dir().join(CHOICES_FILE);
    for line in [
        format!("DECREE_WAIT_ID={wait_id}"),
        format!("DECREE_CHOICES={}", choices.display()),
        "DECREE_QUESTION=Ship this build?".to_string(),
        "DECREE_EVENTS=approve reject".to_string(),
        "DECREE_STATE=approval".to_string(),
        "DECREE_PHASE=invoke".to_string(),
    ] {
        assert!(log.lines().any(|l| l == line), "{line}\n{log}");
    }
    let written: Value = serde_json::from_str(&fs::read_to_string(&choices).unwrap()).unwrap();
    assert_eq!(
        written,
        json!({ "approve": "Ship this build.", "reject": "Do not ship." })
    );
    let script = p
        .events_of("script")
        .into_iter()
        .find(|e| e["script"] == "ask_person")
        .unwrap();
    assert_eq!(script["phase"], "invoke");

    let events = p.events();
    let last = events.last().unwrap();
    assert_eq!(last["type"], "waiting");
    assert_eq!(last["state"], "approval");
    assert_eq!(last["wait_id"], json!(wait_id));
    assert_eq!(last["options"], json!(["approve", "reject"]));
    let timeout_at = DateTime::parse_from_rfc3339(last["timeout_at"].as_str().unwrap())
        .unwrap()
        .with_timezone(&Utc);
    let ahead = (timeout_at - before).num_seconds();
    assert!((59..=61).contains(&ahead), "{ahead}");
    assert!(p.events_of("run_finished").is_empty());
    assert_eq!(run_status(p.machine(), &events, false), RunStatus::Waiting);
    assert_eq!(mirrored_state(&p), "approval");
}

#[test]
fn person_without_timeout_has_null_timeout_at() {
    let text = fs::read_to_string(repo().join("tests/fixtures/machines/step/step_person.yml"))
        .unwrap()
        .replace(", timeout_s: 60", "");
    let p = Project::from_text("step_person", &text, &[]);
    assert!(matches!(p.run(), Outcome::Waiting { .. }));
    assert_eq!(p.events_of("waiting")[0]["timeout_at"], Value::Null);
}

#[test]
fn person_reply_continues_with_source_person_and_reruns_nothing() {
    let (p, wait_id) = person_project(&[("ship_invoke", "print_env")]);
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
            "approval_entry",
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
    // A `decision` event, then the transition it causes.
    let events = p.events();
    let i = events.iter().position(|e| e["type"] == "decision").unwrap();
    assert_eq!(events[i - 1]["type"], "received");
    let d = &events[i];
    assert_eq!(d["state"], "approval");
    assert_eq!(d["kind"], "person");
    assert_eq!(d["event"], "approve");
    assert_eq!(d["options"], json!(["approve", "reject"]));
    assert_eq!(d["reply"], "reply.md");
    // Then the `onexit` scripts, then the transition (docs/reference/runs.md, steps 5 and 6).
    let taken = events[i..]
        .iter()
        .find(|e| e["type"] == "transition")
        .unwrap();
    assert_eq!(taken["from"], "approval");
    assert_eq!(taken["event"], "approve");
    assert_eq!(taken["to"], "ship");
    assert_eq!(taken["source"], "person");
    assert_eq!(taken["exit_code"], Value::Null);
    // Later scripts see the reply, and no wait.
    let log = p.log_of("ship_invoke");
    let reply = p.run_dir().join("received/reply.md");
    assert!(
        log.contains(&format!("DECREE_RECEIVED={}\n", reply.display())),
        "{log}"
    );
    assert!(log.contains("DECREE_WAIT_ID=\n"), "{log}");
    assert!(log.contains("DECREE_CHOICES=\n"), "{log}");
    assert_eq!(run_status(p.machine(), &events, false), RunStatus::Finished);
    // Log numbers continue after the logs written before the wait.
    let logs: Vec<&str> = scripts.iter().map(|e| e["log"].as_str().unwrap()).collect();
    let unique: HashSet<&&str> = logs.iter().collect();
    assert_eq!(unique.len(), logs.len(), "{logs:?}");
}

#[test]
fn person_timeout_error_goes_to_failed_with_source_timeout() {
    let (p, wait_id) = person_project(&[]);
    let outcome = receive(
        &p,
        json!({ "wait_id": wait_id, "event": "error", "timed_out": true }),
    );
    assert_eq!(outcome, Outcome::Finished("failed".into()));
    assert!(p
        .transitions()
        .contains(&"approval error failed timeout".to_string()));
    assert!(p.events_of("decision").is_empty());
    assert_eq!(
        &p.order()[4..],
        ["approval_exit", "gate_exit", "failed_entry", "root_exit"]
    );
}

#[test]
fn person_ask_script_failure_is_error_and_does_not_wait() {
    let p = Project::new("step_person", &[("ask_person", "exit_three")]);
    assert_eq!(p.run(), Outcome::Finished("failed".into()));
    assert!(p.events_of("waiting").is_empty());
    let t = p.events_of("transition");
    let taken = t.iter().find(|e| e["from"] == "approval").unwrap();
    assert_eq!(taken["event"], "error");
    assert_eq!(taken["source"], "exit_code");
    assert_eq!(taken["exit_code"], 3);
}

#[test]
fn person_onentry_failure_is_error_and_does_not_ask() {
    let p = Project::new("step_person", &[("approval_entry", "exit_three")]);
    assert_eq!(p.run(), Outcome::Finished("failed".into()));
    assert!(p.events_of("waiting").is_empty());
    assert!(p
        .events_of("script")
        .iter()
        .all(|e| e["script"] != "ask_person"));
}

#[test]
fn resume_refuses_a_run_that_has_not_received_an_event() {
    let (p, _) = person_project(&[]);
    let params = serde_norway::Mapping::new();
    let ctx = p.ctx();
    let err = Interpreter::new(
        &ctx,
        p.machine(),
        p.executor("inbox", &params),
        inbox_input(),
    )
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
fn every_decision_waiting_and_received_field_appears() {
    let (p, wait_id) = person_project(&[]);
    receive(
        &p,
        json!({ "wait_id": wait_id, "event": "approve", "file": "reply.md" }),
    );
    let (q, wait_id) = person_project(&[]);
    receive(
        &q,
        json!({ "wait_id": wait_id, "event": "error", "timed_out": true }),
    );
    let c = check_project("{ visits: work, equals: 1 }", false, "exit_zero");
    c.run();
    let common = ["v", "seq", "ts", "type", "run_id", "machine", "trigger"];
    let keys = |kind: &str| -> HashSet<String> {
        p.events_of(kind)
            .into_iter()
            .chain(q.events_of(kind))
            .chain(c.events_of(kind))
            .flat_map(|e| e.keys().cloned().collect::<Vec<_>>())
            .collect()
    };
    for (kind, fields) in [
        (
            "waiting",
            &["state", "wait_id", "options", "timeout_at"][..],
        ),
        ("received", &["wait_id", "event", "file", "timed_out"][..]),
        (
            "decision",
            &["state", "kind", "event", "condition", "options", "reply"][..],
        ),
    ] {
        let want: HashSet<String> = common.iter().chain(fields).map(|s| s.to_string()).collect();
        assert_eq!(keys(kind), want, "{kind}");
    }
}

// ---------------------------------------------------------------
// Sub-machines (docs/reference/runs.md)
// ---------------------------------------------------------------

fn fixture(name: &str) -> String {
    fs::read_to_string(repo().join(format!("tests/fixtures/machines/step/{name}.yml"))).unwrap()
}

/// `step_parent` invoking `step_child`, whose `child_work` script is `child_work`.
fn parent_project(child_work: &str) -> Project {
    Project::from_texts(
        &[
            ("step_parent", &fixture("step_parent")),
            ("step_child", &fixture("step_child")),
        ],
        &[("child_work", child_work)],
    )
}

impl Project {
    fn child_dir(&self, id: &str) -> PathBuf {
        self.root().join(".decree/runs").join(id)
    }

    fn child_events(&self, id: &str) -> Vec<Map<String, Value>> {
        read_events(&self.child_dir(id)).unwrap()
    }

    /// The child run id named by the first `waiting` event.
    fn child_id(&self) -> String {
        self.events_of("waiting")[0]["child"]
            .as_str()
            .unwrap()
            .to_string()
    }
}

fn is_run_id(id: &str) -> bool {
    let (stamp, hex) = id.split_once('-').unwrap();
    stamp.len() == 16
        && DateTime::parse_from_str(&format!("{stamp}+0000"), "%Y%m%dT%H%M%SZ%z").is_ok()
        && hex.len() == 6
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[test]
fn machine_invoke_runs_a_child_run_in_its_own_folder() {
    let p = parent_project("print_env");
    assert_eq!(p.run(), Outcome::Finished("done".into()));
    let child = p.child_id();
    assert!(is_run_id(&child), "{child}");

    // The child's message.md: machine, id, parent, depth, trigger, params, the body.
    let message = fs::read_to_string(p.child_dir(&child).join(MESSAGE_FILE)).unwrap();
    assert_eq!(
        message,
        format!(
            "---\nmachine: step_child\nid: {child}\nparent: {RUN_ID}\ndepth: 1\n\
             trigger: invoke\nparams:\n  label: release\nstate: done\n---\n{BODY}"
        )
    );
    // An ordinary run with its own events and logs.
    let events = p.child_events(&child);
    let claim = &events[0];
    assert_eq!(claim["machine"], "step_child");
    assert_eq!(claim["run_id"], json!(child));
    assert_eq!(claim["trigger"], "invoke");
    assert_eq!(claim["file"], Value::Null);
    assert_eq!(events.last().unwrap()["type"], "run_finished");
    assert_eq!(events.last().unwrap()["state"], "done");
    let script = events.iter().find(|e| e["type"] == "script").unwrap();
    let log =
        fs::read_to_string(p.child_dir(&child).join(script["log"].as_str().unwrap())).unwrap();
    for line in [
        format!("DECREE_PARENT={RUN_ID}"),
        format!("DECREE_MESSAGE_ID={child}"),
        "DECREE_MACHINE=step_child".to_string(),
        "DECREE_TRIGGER=invoke".to_string(),
        "DECREE_DATA_LABEL=release".to_string(),
        "DECREE_REQUEST=".to_string(),
        "DECREE_REPLY=".to_string(),
    ] {
        assert!(log.lines().any(|l| l == line), "{line}\n{log}");
    }

    // The parent: waiting for the child, received its final state, then the transition.
    let kinds: Vec<String> = p
        .events()
        .iter()
        .map(|e| e["type"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        kinds,
        [
            "transition",
            "waiting",
            "received",
            "script",
            "transition",
            "run_finished"
        ]
    );
    let waiting = &p.events_of("waiting")[0];
    assert_eq!(waiting["state"], "build");
    assert!(waiting.get("wait_id").is_none());
    let received = &p.events_of("received")[0];
    assert_eq!(received["event"], "done");
    assert_eq!(received["child"], json!(child));
    assert!(received.get("wait_id").is_none());
    let taken = &p.events_of("transition")[1];
    assert_eq!(p.transitions()[1], "build done done machine");
    assert_eq!(taken["exit_code"], Value::Null);
    assert_eq!(p.order(), ["build_exit"]);
}

#[test]
fn machine_invoke_takes_the_childs_final_state_and_failed_as_error() {
    let p = parent_project("print_reject");
    assert_eq!(p.run(), Outcome::Finished("rejected".into()));
    assert_eq!(p.events_of("received")[0]["event"], "rejected");
    assert_eq!(p.transitions()[1], "build rejected rejected machine");

    let p = parent_project("exit_three");
    assert_eq!(p.run(), Outcome::Finished("failed".into()));
    let child = p.child_id();
    assert_eq!(p.child_events(&child).last().unwrap()["state"], "failed");
    assert_eq!(p.events_of("received")[0]["event"], "error");
    assert_eq!(p.transitions()[1], "build error failed machine");
}

#[test]
fn machine_invoke_past_max_depth_starts_no_child_and_is_error() {
    let p = parent_project("print_env");
    let input = RunInput {
        depth: 10,
        ..inbox_input()
    };
    let ctx = p.ctx();
    let empty = serde_norway::Mapping::new();
    let outcome = Interpreter::new(&ctx, p.machine(), p.executor("emit", &empty), input)
        .unwrap()
        .start()
        .unwrap();
    assert_eq!(outcome, Outcome::Finished("failed".into()));
    assert!(p.events_of("waiting").is_empty());
    let taken = &p.events_of("transition")[1];
    assert_eq!(p.transitions()[1], "build error failed machine");
    assert_eq!(taken["error"], "max_depth 10 reached");
    let runs: Vec<_> = fs::read_dir(p.root().join(".decree/runs"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(runs, [RUN_ID]);
}

/// `step_parent` invoking `step_person`, run until the child waits for a reply.
fn parent_of_person() -> (Project, String, String) {
    let parent = fixture("step_parent").replace(
        "machine: step_child, params: { label: release }",
        "machine: step_person",
    );
    let p = Project::from_texts(
        &[
            ("step_parent", &parent),
            ("step_person", &fixture("step_person")),
        ],
        &[],
    );
    let outcome = p.run();
    let child = p.child_id();
    let entered = p
        .child_events(&child)
        .into_iter()
        .find(|e| e["type"] == "transition" && e["to"] == "approval")
        .unwrap();
    let wait_id = format!("{child}.w{}", entered["seq"]);
    assert_eq!(
        outcome,
        Outcome::Child {
            state: "build".into(),
            child: child.clone(),
            outcome: Box::new(Outcome::Waiting {
                state: "approval".into(),
                wait_id: wait_id.clone()
            })
        }
    );
    (p, child, wait_id)
}

/// Append a `received` event to run `id`, as reply delivery does.
fn deliver(p: &Project, id: &str, fields: Value) {
    let m = &p.machines[p.child_events(id)[0]["machine"].as_str().unwrap()];
    let mut events = EventLog::open(&p.child_dir(id), id, &m.id, "invoke").unwrap();
    events.append("received", fields).unwrap();
}

#[test]
fn child_waiting_for_a_person_leaves_the_parent_waiting_until_the_reply_finishes_both() {
    let (p, child, wait_id) = parent_of_person();
    let ctx = p.ctx();
    let events = p.events();
    assert_eq!(events.last().unwrap()["type"], "waiting");
    assert_eq!(events.last().unwrap()["child"], json!(child));
    assert_eq!(ctx.status(p.machine(), &events, false), RunStatus::Waiting);
    let person = &p.machines["step_person"];
    assert_eq!(
        run_status(person, &p.child_events(&child), false),
        RunStatus::Waiting
    );

    // The reply finishes the child, then the parent.
    deliver(
        &p,
        &child,
        json!({ "wait_id": wait_id, "event": "approve", "file": "reply.md" }),
    );
    assert_eq!(
        continue_run(&ctx, &child).unwrap(),
        Outcome::Finished("done".into())
    );
    let child_events = p.child_events(&child);
    assert_eq!(child_events.last().unwrap()["type"], "run_finished");
    assert_eq!(child_events.last().unwrap()["state"], "done");
    let received = &p.events_of("received")[0];
    assert_eq!(received["child"], json!(child));
    assert_eq!(received["event"], "done");
    assert_eq!(
        p.transitions(),
        ["- claimed build claim", "build done done machine"]
    );
    let events = p.events();
    assert_eq!(events.last().unwrap()["type"], "run_finished");
    assert_eq!(ctx.status(p.machine(), &events, false), RunStatus::Finished);
    // The child's scripts ran in the child; the parent's onexit after it finished.
    assert_eq!(p.order().last().unwrap(), "build_exit");
    assert_eq!(mirrored_state(&p), "done");
}

#[test]
fn parent_of_a_finished_child_is_pending_and_continues() {
    let (p, child, wait_id) = parent_of_person();
    let ctx = p.ctx();
    deliver(
        &p,
        &child,
        json!({ "wait_id": wait_id, "event": "reject", "file": "reply.md" }),
    );
    // Continue only the child, as if decree stopped before continuing the parent.
    let person = &p.machines["step_person"];
    let executor = ctx
        .executor(
            person,
            &child,
            "invoke",
            &serde_norway::Mapping::new(),
            Some(RUN_ID),
        )
        .unwrap();
    let input = RunInput {
        depth: 1,
        ..inbox_input()
    };
    let outcome = Interpreter::new(&ctx, person, executor, input)
        .unwrap()
        .resume()
        .unwrap();
    assert_eq!(outcome, Outcome::Finished("rejected".into()));
    let events = p.events();
    assert_eq!(run_status(p.machine(), &events, false), RunStatus::Waiting);
    assert_eq!(ctx.status(p.machine(), &events, false), RunStatus::Pending);

    assert_eq!(
        continue_run(&ctx, RUN_ID).unwrap(),
        Outcome::Finished("rejected".into())
    );
    assert_eq!(p.transitions()[1], "build rejected rejected machine");
}

#[test]
fn parent_of_an_unfinished_child_does_not_continue() {
    let (p, child, _) = parent_of_person();
    let err = continue_run(&p.ctx(), RUN_ID).unwrap_err();
    assert_eq!(
        err.to_string(),
        format!("cannot continue the run: child run `{child}` has not finished")
    );
    assert_eq!(p.events().last().unwrap()["type"], "waiting");
}

#[test]
fn interrupted_child_leaves_the_parent_waiting() {
    let p = parent_project("print_env");
    p.shutdown.store(true, Ordering::SeqCst);
    let outcome = p.run();
    let child = p.child_id();
    assert_eq!(
        outcome,
        Outcome::Child {
            state: "build".into(),
            child: child.clone(),
            outcome: Box::new(Outcome::Interrupted("work".into()))
        }
    );
    assert_eq!(
        p.child_events(&child).last().unwrap()["type"],
        "interrupted"
    );
    let events = p.events();
    assert_eq!(events.last().unwrap()["type"], "waiting");
    assert_eq!(
        p.ctx().status(p.machine(), &events, false),
        RunStatus::Waiting
    );
}

// ---------------------------------------------------------------
// Choose: model (docs/reference/runs.md)
// ---------------------------------------------------------------

/// `step_model`, whose `triage` state asks the machine named `router` (fixture
/// `step_router`, renamed);
/// the router's `ask` script is `ask`, and `work` prints 60 lines. With `reply`, the
/// project root holds it as `reply.json`, for the `router_reply` script.
fn router_fixture() -> String {
    fixture("step_router").replace("name: step_router", "name: router")
}

fn model_project(ask: &str, reply: Option<&str>) -> Project {
    let p = Project::from_texts(
        &[
            ("step_model", &fixture("step_model")),
            ("router", &router_fixture()),
        ],
        &[("work", "print_lines"), ("ask", ask)],
    );
    if let Some(reply) = reply {
        fs::write(p.root().join("reply.json"), reply).unwrap();
    }
    p
}

/// The `decision` event of `triage`.
fn model_decision_of(p: &Project) -> Map<String, Value> {
    let decisions = p.events_of("decision");
    assert_eq!(decisions.len(), 1, "{decisions:?}");
    decisions.into_iter().next().unwrap()
}

#[test]
fn model_router_reply_is_the_event_and_the_request_matches_the_reference() {
    let p = model_project("router_retry", None);
    assert_eq!(p.run(), Outcome::Finished("retried".into()));
    assert_eq!(
        p.transitions(),
        [
            "- claimed work claim",
            "work done triage exit_code",
            "triage retry retried model",
        ]
    );
    let child = p.child_id();
    let child_dir = p.child_dir(&child);

    // The router is an ordinary child run, with the request and reply in its folder.
    let message = fs::read_to_string(child_dir.join(MESSAGE_FILE)).unwrap();
    assert!(
        message.starts_with(&format!(
            "---\nmachine: router\nid: {child}\nparent: {RUN_ID}\ndepth: 1\n\
             trigger: invoke\n"
        )),
        "{message}"
    );
    let request = fs::read_to_string(child_dir.join(REQUEST_FILE)).unwrap();
    let copied = fs::read_to_string(child_dir.join("request_copy.json")).unwrap();
    assert_eq!(copied, request);
    let input: String = (1..=60).map(|i| format!("line {i}\n")).collect::<String>() + "\n\n";
    let want = json!({
        "v": 1,
        "machine": "step_model",
        "machine_description": "Ask a router machine whether to implement again or split the work.",
        "state": "triage",
        "state_description": "The tests failed. Decide what to do next.",
        "question": "Should we implement again or split the work?",
        "options": [
            {"event": "retry", "description": "The failures look fixable; implement again."},
            {"event": "split", "description": "The scope is too large; split it."},
        ],
        "min_confidence": 0.8,
        "input": input,
        "message_body": BODY,
        "history": ["work: done"],
    });
    assert_eq!(serde_json::from_str::<Value>(&copied).unwrap(), want);
    // Keys in docs/reference/runs.md's order.
    let keys: Vec<usize> = [
        "\"v\"",
        "\"machine\"",
        "\"machine_description\"",
        "\"state\"",
        "\"state_description\"",
        "\"question\"",
        "\"options\"",
        "\"min_confidence\"",
        "\"input\"",
        "\"message_body\"",
        "\"history\"",
    ]
    .iter()
    .map(|k| copied.find(k).unwrap())
    .collect();
    assert!(keys.windows(2).all(|w| w[0] < w[1]), "{copied}");

    // The router's script saw DECREE_REQUEST and DECREE_REPLY in its own folder.
    assert!(child_dir.join(REPLY_FILE).is_file());
    let decision = model_decision_of(&p);
    let mut want = json!({
        "state": "triage", "kind": "model", "event": "retry",
        "options": ["retry", "split"], "router": "router",
        "child_run": child, "pick": "retry", "confidence": 0.9,
    });
    for (key, value) in want.as_object_mut().unwrap() {
        assert_eq!(&decision[key], value, "{key}");
    }
    assert!(decision["duration_ms"].is_u64());
    assert!(decision.get("router_error").is_none());
    // `waiting` for the child comes before the decision, which comes before the transition.
    let kinds: Vec<_> = p
        .events()
        .iter()
        .skip(3)
        .map(|e| e["type"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        kinds,
        ["waiting", "decision", "transition", "run_finished"],
        "{:?}",
        p.events()
    );
}

#[test]
fn model_request_omits_min_confidence_when_unset_and_router_overrides_the_default() {
    let text = fixture("step_model")
        .replace(", min_confidence: 0.8", ", router: other_router")
        .replace("      unsure: asked_person\n", "")
        .replace("  asked_person: { final: true }\n", "");
    let router = fixture("step_router").replace("name: step_router", "name: other_router");
    let p = Project::from_texts(
        &[("step_model", &text), ("other_router", &router)],
        &[("work", "print_lines"), ("ask", "router_retry")],
    );
    assert_eq!(p.run(), Outcome::Finished("retried".into()));
    let child = p.child_id();
    let request: Value =
        serde_json::from_str(&fs::read_to_string(p.child_dir(&child).join(REQUEST_FILE)).unwrap())
            .unwrap();
    assert!(request.get("min_confidence").is_none(), "{request}");
    assert_eq!(model_decision_of(&p)["router"], "other_router");
    assert_eq!(p.child_events(&child)[0]["machine"], "other_router");
}

#[test]
fn model_confidence_below_min_confidence_is_unsure_with_the_pick_recorded() {
    for (reply, event, end) in [
        (
            r#"{"event":"retry","reason":"One test fails.","confidence":0.5}"#,
            "unsure",
            "asked_person",
        ),
        // No confidence reported, with min_confidence set: unsure too.
        (r#"{"event":"split"}"#, "unsure", "asked_person"),
        (r#"{"event":"split","confidence":0.8}"#, "split", "split_up"),
    ] {
        let p = model_project("router_reply", Some(reply));
        assert_eq!(p.run(), Outcome::Finished(end.into()), "{reply}");
        let decision = model_decision_of(&p);
        let sent: Value = serde_json::from_str(reply).unwrap();
        assert_eq!(decision["event"], event, "{reply}");
        assert_eq!(decision["pick"], sent["event"], "{reply}");
        for key in ["reason", "confidence"] {
            assert_eq!(decision.get(key), sent.get(key), "{reply}: {key}");
        }
        let taken = p.transitions().last().unwrap().clone();
        assert_eq!(taken, format!("triage {event} {end} model"));
    }
}

#[test]
fn model_reply_probabilities_and_reason_are_recorded() {
    let reply = r#"{"event":"retry","reason":"Fixable.","confidence":0.86,"probabilities":{"retry":0.86,"split":0.14}}"#;
    let p = model_project("router_reply", Some(reply));
    assert_eq!(p.run(), Outcome::Finished("retried".into()));
    let decision = model_decision_of(&p);
    assert_eq!(decision["reason"], "Fixable.");
    assert_eq!(
        decision["probabilities"],
        json!({ "retry": 0.86, "split": 0.14 })
    );
}

#[test]
fn model_invalid_reply_or_failed_router_is_error_with_router_error() {
    let cases = [
        (
            "router_reply",
            Some(r#"{"event":"merge","confidence":0.99}"#),
            "reply.json: `merge` is not one of the options: retry, split",
        ),
        // Never fuzzy-matched.
        (
            "router_reply",
            Some(r#"{"event":"Retry"}"#),
            "reply.json: `Retry` is not one of the options: retry, split",
        ),
        (
            "router_reply",
            Some(r#"{"reason":"no pick"}"#),
            "reply.json: no `event`",
        ),
        (
            "router_reply",
            Some("Retry, I think."),
            "reply.json: not JSON: expected value at line 1 column 1",
        ),
        (
            "router_reply",
            Some(r#"{"event":"retry","confidence":1.5}"#),
            "reply.json: `confidence` 1.5 is not a number from 0 to 1",
        ),
        ("exit_zero", None, "wrote no reply.json"),
        ("exit_three", None, "ended in `failed`"),
    ];
    for (ask, reply, want) in cases {
        let p = model_project(ask, reply);
        assert_eq!(p.run(), Outcome::Finished("failed".into()), "{want}");
        let child = p.child_id();
        let decision = model_decision_of(&p);
        assert_eq!(decision["event"], "error", "{want}");
        assert_eq!(decision["child_run"], json!(child));
        let error = decision["router_error"].as_str().unwrap();
        assert!(error.ends_with(want), "{error} / {want}");
        assert!(decision.get("pick").is_none(), "{want}");
        assert_eq!(p.transitions()[2], "triage error failed model", "{want}");
    }
}

#[test]
fn model_past_max_depth_starts_no_router_and_is_error() {
    let p = model_project("router_retry", None);
    let input = RunInput {
        depth: 10,
        ..inbox_input()
    };
    let ctx = p.ctx();
    let empty = serde_norway::Mapping::new();
    let outcome = Interpreter::new(&ctx, p.machine(), p.executor("emit", &empty), input)
        .unwrap()
        .start()
        .unwrap();
    assert_eq!(outcome, Outcome::Finished("failed".into()));
    assert!(p.events_of("waiting").is_empty());
    let decision = model_decision_of(&p);
    assert_eq!(decision["router_error"], "max_depth 10 reached");
    assert_eq!(decision["router"], "router");
    assert!(decision.get("child_run").is_none());
    assert_eq!(p.transitions()[2], "triage error failed model");
}

#[test]
fn model_router_finished_after_a_crash_is_validated_when_the_parent_continues() {
    let p = model_project("router_retry", None);
    assert_eq!(p.run(), Outcome::Finished("retried".into()));
    // Cut the parent back to its `waiting` event, as if decree stopped there.
    let path = p.run_dir().join(EVENTS_FILE);
    let text = fs::read_to_string(&path).unwrap();
    let kept: String = text
        .lines()
        .take_while(|l| !l.contains("\"type\":\"decision\""))
        .map(|l| format!("{l}\n"))
        .collect();
    fs::write(&path, kept).unwrap();
    let events = p.events();
    assert_eq!(events.last().unwrap()["type"], "waiting");
    assert_eq!(
        p.ctx().status(p.machine(), &events, false),
        RunStatus::Pending
    );
    assert_eq!(
        continue_run(&p.ctx(), RUN_ID).unwrap(),
        Outcome::Finished("retried".into())
    );
    let decision = model_decision_of(&p);
    assert_eq!(decision["event"], "retry");
    assert_eq!(decision["confidence"], 0.9);
}

#[test]
fn reply_parse_validates_each_field() {
    let options = ["retry".to_string(), "split".to_string()];
    let parse = |text: &str| Reply::parse(text.as_bytes(), &options);
    assert_eq!(
        parse(r#"{"event":"retry","reason":null,"confidence":null}"#),
        Ok(Reply::Pick {
            event: "retry".into(),
            reason: None,
            confidence: None,
            probabilities: None,
        })
    );
    for (text, want) in [
        ("[]", "reply.json: not a JSON object"),
        (r#"{"event":3}"#, "reply.json: `event` is not a string"),
        (
            r#"{"event":"retry","reason":1}"#,
            "reply.json: `reason` is not a string",
        ),
        (
            r#"{"event":"retry","confidence":"high"}"#,
            "reply.json: `confidence` \"high\" is not a number from 0 to 1",
        ),
        (
            r#"{"event":"retry","probabilities":{"retry":"most"}}"#,
            "reply.json: `probabilities` is not a map of option to number",
        ),
    ] {
        assert_eq!(parse(text), Err(want.to_string()), "{text}");
    }
}

#[test]
fn run_ids_are_unique_and_skip_ids_taken_in_the_inbox() {
    let tmp = TempDir::new().unwrap();
    let decree = tmp.path().join(DECREE_DIR);
    let mut ids = BTreeSet::new();
    for _ in 0..50 {
        let (id, dir) = create_run_dir(&decree).unwrap();
        assert!(is_run_id(&id), "{id}");
        assert!(dir.is_dir());
        assert!(ids.insert(id));
    }
    // Every next id of this second is queued in the inbox: the run takes another.
    fs::create_dir_all(decree.join(INBOX_DIR)).unwrap();
    let (id, _) = create_run_dir(&decree).unwrap();
    let stamp = id.split_once('-').unwrap().0.to_string();
    let low = u32::from_str_radix(id.split_once('-').unwrap().1, 16).unwrap();
    for k in 1..=3 {
        let next = format!("{stamp}-{:06x}", (low + k) & 0xff_ffff);
        fs::write(decree.join(INBOX_DIR).join(format!("{next}.md")), "").unwrap();
    }
    let _ = id;
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
        let m = load_machine_text(&stem, &text).unwrap();
        let p = Project::new("step_normal", &[]);
        let params = serde_norway::Mapping::new();
        let input = RunInput {
            file: Some("irp.md".to_string()),
            ..RunInput::default()
        };
        let ctx = p.ctx();
        let outcome = Interpreter::new(&ctx, &m, p.executor("inbox", &params), input)
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
// events.jsonl: every docs/reference/runs.md field of the four types
// ---------------------------------------------------------------

#[test]
fn every_transition_script_and_run_finished_field_appears() {
    let mut seen: BTreeMap<String, HashSet<String>> = BTreeMap::new();
    let mut collect = |p: &Project| {
        for e in p.events() {
            let kind = e["type"].as_str().unwrap().to_string();
            seen.entry(kind).or_default().extend(e.keys().cloned());
        }
    };

    let p = Project::new("step_exit_fail", &[]);
    p.run();
    collect(&p);
    let p = Project::new("step_normal", &[("a_invoke", "print_undeclared")]);
    p.run();
    collect(&p);
    let p = Project::new("step_timeout", &[("sleep_long", "sleep_long")]);
    p.run();
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
    let expected: [(&str, &[&str]); 3] = [
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
        ("run_finished", &["state", "duration_ms"]),
    ];
    for (kind, fields) in expected {
        let keys = &seen[kind];
        let want: HashSet<String> = common.iter().chain(fields).map(|s| s.to_string()).collect();
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
    assert_eq!(events.len(), 2);
    assert_eq!(events[1]["type"], "run_finished");
    assert_eq!(events[1]["state"], "failed");
    let t = &events[0];
    assert_eq!(t["from"], Value::Null);
    assert_eq!(t["to"], "failed");
    assert_eq!(t["source"], "invalid_message");
    assert_eq!(t["error"], "params: unknown name `x`");
    assert_eq!(t["file"], "bad.md");
    assert_eq!(mirrored_state(&p), "failed");
    assert_eq!(run_status(p.machine(), &events, false), RunStatus::Finished);
    assert!(p.order().is_empty());
}

// ---------------------------------------------------------------
// Run status (docs/reference/messages.md)
// ---------------------------------------------------------------

#[test]
fn run_status_follows_the_reference_order() {
    let p = Project::new("step_normal", &[]);
    let m = p.machine();
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
    let out =
        mirror(b"---\nzeta: 1\nid: x\nstate: old\ncustom: [a, b]\n---\r\nbody\r\nmore\n").unwrap();
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
        err.contains("message.md: line 1: frontmatter has an opening `---` but no closing"),
        "{err}"
    );
    let err = mirror(b"---\nid: x\nid: y\n---\n").unwrap_err().to_string();
    assert!(err.contains("message.md: line 3: duplicate"), "{err}");
}
