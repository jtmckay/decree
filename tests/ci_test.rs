//! `.github/workflows/ci.yml` parses as YAML, its `test` job runs exactly the commands of
//! the gate script (`.decree/scripts/develop/gate.sh`), so the two cannot drift apart,
//! and its `decree-check` job checks every project with `--format sarif` and uploads the
//! results. Reads this repository's files only; writes nothing.

use serde_norway::Value;
use std::path::PathBuf;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workflow() -> Value {
    let path = repo().join(".github/workflows/ci.yml");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    serde_norway::from_str(&text).unwrap_or_else(|e| panic!("{path:?} is not YAML: {e}"))
}

fn job<'a>(workflow: &'a Value, name: &str) -> &'a Value {
    let job = &workflow["jobs"][name];
    assert!(job.is_mapping(), "ci.yml has no `{name}` job");
    job
}

fn steps(job: &Value) -> &[Value] {
    job["steps"]
        .as_sequence()
        .expect("a job's `steps` is a list")
}

/// The `run` of each step that has one, in order.
fn runs(job: &Value) -> Vec<&str> {
    steps(job)
        .iter()
        .filter_map(|s| s["run"].as_str())
        .collect()
}

/// The `uses` of each step that has one, in order.
fn uses(job: &Value) -> Vec<&str> {
    steps(job)
        .iter()
        .filter_map(|s| s["uses"].as_str())
        .collect()
}

/// The commands the gate script runs: each `cargo` line, without its trailing `&&`.
fn gate_commands() -> Vec<String> {
    let path = repo().join(".decree/scripts/develop/gate.sh");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    text.lines()
        .map(|line| line.trim().trim_end_matches("&&").trim())
        .filter(|line| line.starts_with("cargo "))
        .map(str::to_string)
        .collect()
}

#[test]
fn the_workflow_parses_and_runs_on_pushes_and_pull_requests() {
    let wf = workflow();
    assert_eq!(wf["name"].as_str(), Some("CI"));
    let branches: Vec<&str> = wf["on"]["push"]["branches"]
        .as_sequence()
        .expect("`on.push.branches` is a list")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(branches, ["main", "v0.5"]);
    assert!(
        wf["on"].get("pull_request").is_some(),
        "ci.yml runs on every pull_request"
    );
    assert_eq!(wf["permissions"]["contents"].as_str(), Some("read"));
}

#[test]
fn the_test_job_runs_exactly_the_gate_commands() {
    let gate = gate_commands();
    assert_eq!(
        gate,
        [
            "cargo fmt --check",
            "cargo clippy --all-targets -- -D warnings",
            "cargo test"
        ],
        "gate.sh's commands changed; this test reads them by line"
    );
    let wf = workflow();
    let test = job(&wf, "test");
    assert_eq!(test["runs-on"].as_str(), Some("ubuntu-latest"));
    assert_eq!(runs(test), gate, "the `test` job runs the gate's commands");
    assert_eq!(
        uses(test),
        [
            "actions/checkout@v4",
            "dtolnay/rust-toolchain@stable",
            "Swatinem/rust-cache@v2"
        ]
    );
    let toolchain = steps(test)
        .iter()
        .find(|s| s["uses"].as_str() == Some("dtolnay/rust-toolchain@stable"))
        .unwrap();
    let components = toolchain["with"]["components"].as_str().unwrap_or_default();
    for component in ["rustfmt", "clippy"] {
        assert!(
            components.contains(component),
            "toolchain lacks {component}"
        );
    }
}

#[test]
fn the_decree_check_job_checks_every_project_as_sarif_and_uploads_it() {
    let wf = workflow();
    let check = job(&wf, "decree-check");
    assert_eq!(check["runs-on"].as_str(), Some("ubuntu-latest"));
    assert_eq!(
        check["permissions"]["security-events"].as_str(),
        Some("write")
    );
    assert_eq!(check["permissions"]["contents"].as_str(), Some("read"));

    let runs = runs(check);
    assert!(runs.contains(&"cargo build --release"));
    let script = runs
        .iter()
        .find(|r| r.contains("check --format sarif"))
        .expect("a step runs `decree check --format sarif`");
    // The projects come from a glob, so a new example is checked without editing ci.yml.
    assert!(script.contains("for project in . examples/*/"), "{script}");
    assert!(script.contains(".automationDetails.id"), "{script}");
    assert!(script.contains("artifactLocation.uri"), "{script}");

    // One upload of the folder; each file's category is its automationDetails.id.
    let uploads: Vec<&Value> = steps(check)
        .iter()
        .filter(|s| s["uses"].as_str() == Some("github/codeql-action/upload-sarif@v4"))
        .collect();
    assert_eq!(uploads.len(), 1, "one upload-sarif step");
    assert_eq!(uploads[0]["with"]["sarif_file"].as_str(), Some("sarif"));
    assert!(
        uploads[0]["with"].get("category").is_none(),
        "the category comes from each file's automationDetails.id"
    );

    // The job fails at the end, after the upload, if any check failed.
    let last = steps(check).last().unwrap();
    assert!(last["if"]
        .as_str()
        .unwrap_or_default()
        .contains("steps.check.outputs.failed"));
    assert!(last["run"].as_str().unwrap_or_default().contains("exit 1"));
}
