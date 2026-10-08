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

/// A project with machine `m`, whose one state runs `work`, and an inbox message for it.
fn project(work: &str) -> TempDir {
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
    write_script(&decree.join("scripts/work"), work);
    queue(&tmp, "a");
    tmp
}

fn queue(tmp: &TempDir, id: &str) {
    fs::write(
        tmp.path().join(format!(".decree/inbox/{id}.md")),
        format!("---\nid: {id}\nmachine: m\n---\nbody\n"),
    )
    .unwrap();
}

const PRINT_COMFY_URL: &str =
    "#!/usr/bin/env bash\necho \"$COMFY_URL\" >> \"$DECREE_PROJECT_ROOT/out.txt\"\n";

/// AC: `.decree/env` sets `COMFY_URL` for every script, and decree's own environment wins.
#[test]
fn dotenv_values_reach_scripts_and_the_process_environment_wins() {
    let tmp = project(PRINT_COMFY_URL);
    fs::write(
        tmp.path().join(".decree/env"),
        "# ComfyUI\nCOMFY_URL=\"http://box:8188\"\n",
    )
    .unwrap();
    cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .arg("process")
        .env_remove("COMFY_URL")
        .assert()
        .success();
    queue(&tmp, "b");
    cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .arg("process")
        .env("COMFY_URL", "http://other:8188")
        .assert()
        .success();
    let out = fs::read_to_string(tmp.path().join("out.txt")).unwrap();
    assert_eq!(out, "http://box:8188\nhttp://other:8188\n");
}

/// Quotes are removed, `export` and comments are allowed, double-quoted and plain values
/// are interpolated, single-quoted ones are not.
#[test]
fn dotenv_syntax_as_compose_reads_it() {
    let tmp = project(
        "#!/usr/bin/env bash\nprintf '%s|%s|%s|%s\\n' \"$A\" \"$B\" \"$C\" \"$D\" > \"$DECREE_PROJECT_ROOT/out.txt\"\n",
    );
    fs::write(
        tmp.path().join(".decree/env"),
        "# comment\n\nA='single $HOME'\nexport B=exported\nC=\"$HOME\"\nD=plain value\n",
    )
    .unwrap();
    cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .arg("process")
        .env("HOME", "/home/x")
        .env_remove("A")
        .env_remove("B")
        .env_remove("C")
        .env_remove("D")
        .assert()
        .success();
    let out = fs::read_to_string(tmp.path().join("out.txt")).unwrap();
    assert_eq!(out, "single $HOME|exported|/home/x|plain value\n");
}

/// AC: `HOST=box` and `URL=http://${HOST}:8188` print `http://box:8188`, then
/// `http://other:8188` with `HOST=other` in decree's environment.
#[test]
fn dotenv_values_are_interpolated_and_the_process_environment_wins() {
    let tmp = project("#!/usr/bin/env bash\necho \"$URL\" >> \"$DECREE_PROJECT_ROOT/out.txt\"\n");
    fs::write(
        tmp.path().join(".decree/env"),
        "HOST=box\nURL=http://${HOST}:8188\n",
    )
    .unwrap();
    cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .arg("process")
        .env_remove("HOST")
        .env_remove("URL")
        .assert()
        .success();
    queue(&tmp, "b");
    cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .arg("process")
        .env("HOST", "other")
        .env_remove("URL")
        .assert()
        .success();
    let out = fs::read_to_string(tmp.path().join("out.txt")).unwrap();
    assert_eq!(out, "http://box:8188\nhttp://other:8188\n");
}

/// AC: `A='${B}'` and `C=$$5` print `${B}` and `$5`.
#[test]
fn single_quotes_and_double_dollar_are_literal() {
    let tmp = project(
        "#!/usr/bin/env bash\nprintf '%s|%s\\n' \"$A\" \"$C\" > \"$DECREE_PROJECT_ROOT/out.txt\"\n",
    );
    fs::write(tmp.path().join(".decree/env"), "A='${B}'\nC=$$5\n").unwrap();
    cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .env("NO_COLOR", "1")
        .arg("check")
        .assert()
        .success();
    cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .arg("process")
        .env("B", "set")
        .env_remove("A")
        .env_remove("C")
        .assert()
        .success();
    let out = fs::read_to_string(tmp.path().join("out.txt")).unwrap();
    assert_eq!(out, "${B}|$5\n");
}

/// An unknown variable reads as empty; `decree check` warns about it, naming the line, and
/// passes. Set in decree's environment, there is no warning.
#[test]
fn an_unknown_variable_is_empty_and_check_warns() {
    let tmp = project("#!/usr/bin/env bash\necho \"[$URL]\" > \"$DECREE_PROJECT_ROOT/out.txt\"\n");
    fs::write(
        tmp.path().join(".decree/env"),
        "# hosts\n\nPORT=1\nURL=http://${DECREE_TEST_UNSET_HOST}:$PORT\n",
    )
    .unwrap();
    let out = cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .env("NO_COLOR", "1")
        .env_remove("DECREE_TEST_UNSET_HOST")
        .arg("check")
        .assert()
        .success()
        .get_output()
        .clone();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("warning: env: line 4: `${DECREE_TEST_UNSET_HOST}` is not set\n"),
        "{stderr}"
    );
    let out = cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .env("NO_COLOR", "1")
        .env("DECREE_TEST_UNSET_HOST", "box")
        .arg("check")
        .assert()
        .success()
        .get_output()
        .clone();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(!stderr.contains("is not set"), "{stderr}");
    cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .arg("process")
        .env_remove("DECREE_TEST_UNSET_HOST")
        .env_remove("URL")
        .env_remove("PORT")
        .assert()
        .success();
    let out = fs::read_to_string(tmp.path().join("out.txt")).unwrap();
    assert_eq!(out, "[http://:1]\n");
}

/// AC: a reserved key, a line that is not a pair, an unterminated `${` or a malformed
/// `${VAR:…}` is a `decree check` error naming the file and line, and `process` refuses to start with the same error.
#[test]
fn a_malformed_dotenv_fails_check_and_process() {
    for (text, line) in [
        ("A=1\nDECREE_X=1\n", 2),
        ("not a pair\n", 1),
        ("A=1\nURL=http://${HOST\n", 2),
        ("A=1\n\nURL=${HOST:?required}\n", 3),
    ] {
        let tmp = project(PRINT_COMFY_URL);
        fs::write(tmp.path().join(".decree/env"), text).unwrap();
        let out = cargo_bin_cmd!("decree")
            .current_dir(tmp.path())
            .env("NO_COLOR", "1")
            .arg("check")
            .assert()
            .code(1)
            .get_output()
            .clone();
        let stdout = String::from_utf8(out.stdout).unwrap();
        let expected = format!("env: line {line}: ");
        assert!(stdout.starts_with(&expected), "{stdout}");
        assert!(stdout.trim_end().ends_with("(E1)"), "{stdout}");

        let out = cargo_bin_cmd!("decree")
            .current_dir(tmp.path())
            .env("NO_COLOR", "1")
            .arg("process")
            .assert()
            .code(1)
            .get_output()
            .clone();
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(stderr.contains(stdout.trim_end()), "{stderr}");
        assert!(
            tmp.path().join(".decree/inbox/a.md").exists(),
            "nothing ran"
        );
        assert!(!tmp.path().join("out.txt").exists());
    }
}

/// AC: `script: { name: build, env: { METHOD: image_text } }` gives that invoke
/// `METHOD=image_text`, over `.decree/env` and the process environment; another state
/// invoking `build` without `env` does not get it. Invoke `env:` values are not
/// interpolated: the YAML stays literal.
#[test]
fn invoke_env_wins_for_that_invoke_only() {
    let tmp = project("#!/usr/bin/env bash\n");
    let decree = tmp.path().join(".decree");
    fs::write(
        decree.join("machines/m.yml"),
        "name: m\ndescription: One script, two states.\ninitial: build_image_text\nstates:\n  \
         build_image_text:\n    invoke:\n      script: { name: build, env: { METHOD: image_text, STEPS: 30, RAW: \"${STEPS}$$\" } }\n    \
         transitions: { done: build_plain }\n  \
         build_plain:\n    invoke: build\n    transitions: { done: done }\n  \
         done: { final: true }\n  failed: { final: true }\n",
    )
    .unwrap();
    write_script(
        &decree.join("scripts/build"),
        "#!/usr/bin/env bash\necho \"$DECREE_STATE METHOD=${METHOD-unset} STEPS=${STEPS-unset} RAW=${RAW-unset}\" >> \"$DECREE_PROJECT_ROOT/out.txt\"\n",
    );
    fs::write(decree.join("env"), "STEPS=10\n").unwrap();
    cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .env("NO_COLOR", "1")
        .arg("check")
        .assert()
        .success();
    cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .arg("process")
        .env("STEPS", "20")
        .env_remove("METHOD")
        .env_remove("RAW")
        .assert()
        .success();
    let out = fs::read_to_string(tmp.path().join("out.txt")).unwrap();
    assert_eq!(
        out,
        "build_image_text METHOD=image_text STEPS=30 RAW=${STEPS}$$\nbuild_plain METHOD=unset STEPS=20 RAW=unset\n"
    );
}

/// AC: in a chain parent -> child -> grandchild of `machine` invokes, each script sees its
/// parent's run folder in `DECREE_PARENT_RUN_DIR` (empty at the top) and the top run's in
/// `DECREE_ROOT_RUN_DIR` (its own at the top).
#[test]
fn run_dir_variables_in_a_parent_a_child_and_a_grandchild() {
    let tmp = project("#!/usr/bin/env bash\n");
    let decree = tmp.path().join(".decree");
    let machine = |name: &str, child: Option<&str>| {
        let call = match child {
            Some(child) => format!(
                "  call:\n    invoke: {{ machine: {child} }}\n    transitions: {{ done: done }}\n"
            ),
            None => String::new(),
        };
        let after = if child.is_some() { "call" } else { "done" };
        fs::write(
            decree.join(format!("machines/{name}.yml")),
            format!(
                "name: {name}\ndescription: One link of a chain.\ninitial: work\nstates:\n  \
                 work:\n    invoke: dirs\n    transitions: {{ done: {after} }}\n{call}  \
                 done: {{ final: true }}\n  failed: {{ final: true }}\n"
            ),
        )
        .unwrap();
    };
    machine("m", Some("child"));
    machine("child", Some("grandchild"));
    machine("grandchild", None);
    write_script(
        &decree.join("scripts/dirs"),
        "#!/usr/bin/env bash\necho \"$DECREE_MACHINE|$DECREE_RUN_DIR|$DECREE_PARENT_RUN_DIR|$DECREE_ROOT_RUN_DIR\" >> \"$DECREE_PROJECT_ROOT/out.txt\"\n",
    );
    cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .env("NO_COLOR", "1")
        .arg("check")
        .assert()
        .success();
    cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .arg("process")
        .assert()
        .success();
    let out = fs::read_to_string(tmp.path().join("out.txt")).unwrap();
    let rows: Vec<Vec<&str>> = out.lines().map(|l| l.split('|').collect()).collect();
    assert_eq!(rows.len(), 3, "{out}");
    let (parent, child, grandchild) = (&rows[0], &rows[1], &rows[2]);
    assert_eq!(
        (parent[0], child[0], grandchild[0]),
        ("m", "child", "grandchild")
    );
    let runs = decree.join("runs");
    assert_eq!(parent[1], runs.join("a").to_str().unwrap());
    // The parent: no parent, and its own folder is the root.
    assert_eq!(parent[2], "");
    assert_eq!(parent[3], parent[1]);
    // The child: the parent's folder for both.
    assert_eq!(child[2], parent[1]);
    assert_eq!(child[3], parent[1]);
    // The grandchild: the child's folder, and the parent's as the root.
    assert_eq!(grandchild[2], child[1]);
    assert_eq!(grandchild[3], parent[1]);
    assert_ne!(child[1], parent[1]);
    assert_ne!(grandchild[1], child[1]);
}
