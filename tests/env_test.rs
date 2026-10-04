//! A decree started inside another decree run (a test suite run by a gate script, say)
//! inherits that run's variables. Its scripts must see only the `DECREE_*` variables it
//! sets itself (docs/reference/scripts.md, Environment), not the outer run's.

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use tempfile::TempDir;

mod common;
use common::write_script;

#[test]
fn a_script_sees_only_the_decree_variables_its_own_run_sets() {
    let tmp = TempDir::new().unwrap();
    let decree = tmp.path().join(".decree");
    for dir in ["machines", "scripts", "inbox", "runs"] {
        fs::create_dir_all(decree.join(dir)).unwrap();
    }
    fs::write(
        decree.join("machines/m.yml"),
        "name: m\ndescription: One script.\ninitial: work\nstates:\n  work:\n    invoke: work\n    transitions: { done: done }\n  done: { final: true }\n  failed: { final: true }\n",
    )
    .unwrap();
    write_script(
        &decree.join("scripts/work"),
        "#!/usr/bin/env bash\nenv | grep -E '^(DECREE_|TRACESTATE=)' | sort > \"$DECREE_PROJECT_ROOT/env.txt\"\n",
    );
    fs::write(
        decree.join("inbox/a.md"),
        "---\nid: a\nmachine: m\n---\nbody\n",
    )
    .unwrap();
    cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .arg("process")
        // What an outer run's script environment holds.
        .env("DECREE_DATA_OUTER", "leak")
        .env("DECREE_PARENT", "outer-run")
        .env("DECREE_MESSAGE_ID", "outer-run")
        .env("DECREE_WAIT_ID", "outer-run.w3")
        .env("TRACESTATE", "outer=1")
        .assert()
        .success();
    let env = fs::read_to_string(tmp.path().join("env.txt")).unwrap();
    assert!(!env.contains("DECREE_DATA_OUTER"), "{env}");
    assert!(!env.contains("outer"), "{env}");
    assert!(env.contains("DECREE_MESSAGE_ID=a\n"), "{env}");
    assert!(env.contains("DECREE_PARENT=\n"), "{env}");
}
