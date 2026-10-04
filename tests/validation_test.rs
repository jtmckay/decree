//! `decree check` for each validation rule of docs/reference/machines.md (Validation), V1–V21
//! and M1–M3, as a table. Each case is a rule, a name, the files of `.decree/` inline, and the
//! exact stdout of `decree check` (or `PASSES`: exit 0 and no output). The helper writes the
//! case into a temp project and adds a stub executable for every script the machines
//! reference, except where the case places a script itself or says it is missing.

use assert_cmd::cargo::cargo_bin_cmd;
use serde_norway::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use tempfile::TempDir;

/// `decree check` exits 0 and prints nothing.
const PASSES: &str = "";

const STUB: &str = "#!/usr/bin/env bash\nexit 0\n";

/// A script the case places itself, instead of the default `scripts/<name>` stub.
enum Script {
    /// No file at all.
    Missing(&'static str),
    /// An executable stub at `scripts/<path>`.
    At(&'static str),
    /// A stub at `scripts/<path>` without the execute bit.
    NotExecutable(&'static str),
}

impl Script {
    /// The script name this stands for: the file stem of its path.
    fn name(&self) -> &str {
        let (Script::Missing(path) | Script::At(path) | Script::NotExecutable(path)) = self;
        let file = path.rsplit('/').next().unwrap();
        file.split('.').next().unwrap()
    }
}

struct Case {
    rule: &'static str,
    name: &'static str,
    /// Files under `.decree/`: (path, text).
    files: &'static [(&'static str, &'static str)],
    scripts: &'static [Script],
    expected: &'static str,
}

/// A machine with one script state.
const BASE: &str = "\
name: m
description: Base machine.
initial: work
states:
  work:
    invoke: work
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

/// A machine with one datum of each type.
const TYPED: &str = "\
name: m
description: Typed data.
data:
  max_rounds: { type: int, default: 2 }
  mode: { type: string, default: fast }
  strict: { type: bool, default: true }
initial: work
states:
  work:
    invoke: work
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

const CASES: &[Case] = &[
    // V1
    Case {
        rule: "V1",
        name: "name equals the file stem",
        files: &[("machines/m.yml", BASE)],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V1",
        name: "name differs from the file stem",
        files: &[(
            "machines/m.yml",
            "\
name: other
description: Base machine.
initial: work
states:
  work:
    invoke: work
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: "machines/m.yml: line 1: name `other` does not equal the file stem `m` (V1)\n",
    },
    // V2
    Case {
        rule: "V2",
        name: "lowercase state ids",
        files: &[("machines/m.yml", BASE)],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V2",
        name: "a state id with a capital",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: Base machine.
initial: Work
states:
  Work:
    invoke: work
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: "machines/m.yml: Work: state id `Work` does not match ^[a-z][a-z0-9_]*$ (V2)\n",
    },
    // V3
    Case {
        rule: "V3",
        name: "initial names a child",
        files: &[("machines/m.yml", BASE)],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V3",
        name: "initial names no child",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: Base machine.
initial: nowhere
states:
  work:
    invoke: work
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: "machines/m.yml: line 3: initial `nowhere` is not a direct child state (V3)\n",
    },
    // V4
    Case {
        rule: "V4",
        name: "every target exists",
        files: &[("machines/m.yml", BASE)],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V4",
        name: "a target that does not exist",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: Base machine.
initial: work
states:
  work:
    invoke: work
    transitions: { done: done, skip: nowhere }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: "machines/m.yml: work: transition `skip` targets unknown state `nowhere` (V4)\n",
    },
    // V5
    Case {
        rule: "V5",
        name: "a root final state named failed",
        files: &[("machines/m.yml", BASE)],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V5",
        name: "no state named failed",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: Base machine.
initial: work
states:
  work:
    invoke: work
    transitions: { done: done }
  done: { final: true }
",
        )],
        scripts: &[],
        expected: "machines/m.yml: line 4: no root-level final state `failed`: every machine needs one for unhandled errors (V5)\n",
    },
    // V6
    Case {
        rule: "V6",
        name: "a compound state without invoke",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: A compound state with its own final state.
initial: work
states:
  work:
    initial: step
    transitions: { done.state.work: done }
    states:
      step:
        invoke: work
        transitions: { done: fin }
      fin: { final: true }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V6",
        name: "a compound state with invoke",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: A compound state with its own final state.
initial: work
states:
  work:
    invoke: work
    initial: step
    transitions: { done.state.work: done }
    states:
      step:
        invoke: work
        transitions: { done: fin }
      fin: { final: true }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: "machines/m.yml: work: compound state may not have `invoke` (V6)\n",
    },
    // V7
    Case {
        rule: "V7",
        name: "final states with only final",
        files: &[("machines/m.yml", BASE)],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V7",
        name: "a final state with transitions",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: Base machine.
initial: work
states:
  work:
    invoke: work
    transitions: { done: done }
  done: { final: true, transitions: { again: work } }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: "machines/m.yml: done: final state may only have `final`, `description`, `onentry` and `emits`, not `transitions` (V7)\n",
    },
    // V8
    Case {
        rule: "V8",
        name: "a check handles true and false, a person is asked a question",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: A check, then a person picks one option.
initial: work
states:
  work:
    invoke: work
    transitions: { done: again }
  again:
    invoke:
      check: { visits: work, less_than: 2 }
    transitions: { true: work, false: approval }
  approval:
    invoke:
      person:
        question: Ship it?
        ask: ask
    transitions:
      approve: { target: done, description: Ship it. }
      reject:  { target: failed, description: Do not ship. }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V8",
        name: "a check misses false, a person is asked nothing",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: A check that misses false, and a person asked nothing.
initial: work
states:
  work:
    invoke: work
    transitions: { done: again }
  again:
    invoke:
      check: { visits: work, less_than: 2 }
    transitions: { true: approval }
  approval:
    invoke:
      person: { ask: ask }
    transitions:
      approve: { target: done, description: Ship it. }
      reject:  failed
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: "\
machines/m.yml: again: a `check` state must handle `false`, itself or through an ancestor (V8)
machines/m.yml: approval: a `person` state needs a `question`: what is being decided (V8)
machines/m.yml: approval: option `reject` needs a `description`: write it as `reject: { target: failed, description: ... }` (V8)
",
    },
    // V9
    Case {
        rule: "V9",
        name: "output names a script state",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: A check on the output of a script state.
initial: work
states:
  work:
    invoke: work
    transitions: { done: ok }
  ok:
    invoke:
      check: { output: work, matches: \"^ok\" }
    transitions: { true: done, false: work }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V9",
        name: "output names a state that runs no script",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: A check on the output of a state that runs no script.
initial: work
states:
  work:
    invoke: work
    transitions: { done: ok }
  ok:
    invoke:
      check: { output: ok, matches: \"^ok\" }
    transitions: { true: done, false: work }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: "machines/m.yml: ok: output `ok` is not a state with a script invoke (V9)\n",
    },
    // V10
    Case {
        rule: "V10",
        name: "a condition on existing data",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: A check on visits against data.
data:
  max_rounds: { type: int, default: 2 }
initial: work
states:
  work:
    invoke: work
    transitions: { done: again }
  again:
    invoke:
      check: { visits: work, less_than: { data: max_rounds } }
    transitions: { true: work, false: done }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V10",
        name: "a condition on unknown data",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: A check on visits against data.
data:
  max_rounds: { type: int, default: 2 }
initial: work
states:
  work:
    invoke: work
    transitions: { done: again }
  again:
    invoke:
      check: { visits: work, less_than: { data: rounds } }
    transitions: { true: work, false: done }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: "machines/m.yml: again: check: unknown data `rounds` (V10)\n",
    },
    // V11
    Case {
        rule: "V11",
        name: "every state is reachable and can finish",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: Every state is reachable and can finish.
initial: work
states:
  work:
    invoke: work
    transitions: { done: again }
  again:
    invoke:
      check: { visits: work, less_than: 2 }
    transitions: { true: work, false: done }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V11",
        name: "an unreachable state and a stall",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: A machine with an unreachable state and a stall.
initial: work
states:
  work:
    invoke: work
    transitions: { done: done, hold: wait }
  wait:
    invoke:
      check: { visits: wait, less_than: 2 }
    transitions: { true: wait, false: wait }
  orphan:
    invoke: work
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: "\
machines/m.yml: orphan: state is unreachable from the root `initial` (V11)
machines/m.yml: wait: state cannot reach a root-level final state (V11)
",
    },
    // V12
    Case {
        rule: "V12",
        name: "every script resolves, one in the machine's folder",
        files: &[("machines/m.yml", V12_MACHINE)],
        scripts: &[Script::At("m/snapshot.sh")],
        expected: PASSES,
    },
    Case {
        rule: "V12",
        name: "a missing script and one that is not executable",
        files: &[("machines/m.yml", V12_MACHINE)],
        scripts: &[
            Script::Missing("setup"),
            Script::NotExecutable("snapshot.sh"),
        ],
        expected: "\
machines/m.yml: line 3: script `setup` not found; searched scripts/m, scripts (V12)
machines/m.yml: work: script `snapshot`: scripts/snapshot.sh is not executable (V12)
",
    },
    Case {
        rule: "V12",
        name: "a script with no extension named like its machine",
        files: &[(
            "machines/verify.yml",
            "\
name: verify
description: Its script is scripts/verify, a file where a per-machine directory would be.
initial: work
states:
  work:
    invoke: verify
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: PASSES,
    },
    // V13
    Case {
        rule: "V13",
        name: "emits names a machine",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: Base machine.
initial: work
states:
  work:
    invoke: work
    emits: [m]
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V13",
        name: "emits names an unknown machine",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: Base machine.
initial: work
states:
  work:
    invoke: work
    emits: [nope]
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: "machines/m.yml: work: emits unknown machine `nope` (V13)\n",
    },
    // V14
    Case {
        rule: "V14",
        name: "every default matches its type",
        files: &[("machines/m.yml", TYPED)],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V14",
        name: "a string default for a bool",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: Typed data.
data:
  max_rounds: { type: int, default: 2 }
  mode: { type: string, default: fast }
  strict: { type: bool, default: \"yes\" }
initial: work
states:
  work:
    invoke: work
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: "machines/m.yml: line 6: data `strict`: default `yes` is not of type `bool` (V14)\n",
    },
    // V15
    Case {
        rule: "V15",
        name: "done.state is handled",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: A compound state whose final state raises done.state.work.
initial: work
states:
  work:
    initial: step
    transitions: { done.state.work: done }
    states:
      step:
        invoke: work
        transitions: { done: fin }
      fin: { final: true }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V15",
        name: "nothing handles done.state",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: A compound state whose final state nothing handles.
initial: work
states:
  work:
    initial: step
    transitions: { stop: done }
    states:
      step:
        invoke: work
        transitions: { done: fin }
      fin: { final: true }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: "machines/m.yml: work: compound state has a final state, but nothing handles `done.state.work`, itself or through an ancestor (V15)\n",
    },
    // V16
    Case {
        rule: "V16",
        name: "a model with a machine named router",
        files: &[
            ("machines/m.yml", V16_MACHINE.0),
            (
                "machines/router.yml",
                "\
name: router
description: Answers a model request.
initial: ask
states:
  ask:
    invoke: work
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
",
            ),
        ],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V16",
        name: "no machine named router, min_confidence out of range",
        files: &[
            ("machines/m.yml", V16_MACHINE.1),
            (
                "machines/picker.yml",
                "\
name: picker
description: Answers a model request.
initial: ask
states:
  ask:
    invoke: work
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
",
            ),
        ],
        scripts: &[],
        expected: "\
machines/m.yml: triage: `model` names no `router`, and there is no machine named `router` (V16)
machines/m.yml: triage: min_confidence 80 is not between 0 and 1 (V16)
",
    },
    // V17
    Case {
        rule: "V17",
        name: "internal from a compound state to its child",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: An internal transition from a compound state to its child.
initial: work
states:
  work:
    initial: step
    transitions:
      again: { target: step, type: internal }
      done.state.work: done
    states:
      step:
        invoke: work
        transitions: { done: fin }
      fin: { final: true }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V17",
        name: "internal from an atomic state",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: An internal transition from an atomic state.
initial: work
states:
  work:
    invoke: work
    transitions: { done: { target: done, type: internal } }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: "machines/m.yml: work: transition `done`: `type: internal` is only allowed from a compound state to one of its descendants (V17)\n",
    },
    // V18
    Case {
        rule: "V18",
        name: "lowercase events",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: A script that prints pass or fail.
initial: work
states:
  work:
    invoke: work
    transitions: { pass: done, fail: failed }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V18",
        name: "an event with a capital",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: A script that prints pass or fail.
initial: work
states:
  work:
    invoke: work
    transitions: { Pass: done, fail: failed }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: "machines/m.yml: work: event `Pass` does not match ^[a-z][a-z0-9_]*(\\.[a-z0-9_]+)*$ (V18)\n",
    },
    // V19
    Case {
        rule: "V19",
        name: "only the SCXML subset",
        files: &[("machines/m.yml", BASE)],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V19",
        name: "cond on a transition, router on a state",
        files: &[
            (
                "machines/a.yml",
                "\
name: a
description: A transition with cond.
initial: work
states:
  work:
    invoke: work
    transitions:
      done: { target: done, cond: \"visits.work < 2\" }
  done: { final: true }
  failed: { final: true }
",
            ),
            ("machines/b.yml", ROUTER_ON_A_STATE),
        ],
        scripts: &[],
        expected: "\
machines/a.yml: work: transition `done`: cond on a transition is not supported: make the decision a state with invoke: { check: ... } (V19)
machines/b.yml: work.step: router on a state is not supported: make the decision a state with invoke: { model: { question: ... } } (V19)
",
    },
    Case {
        rule: "V19",
        name: "choose, the old decision invoke",
        files: &[("machines/m.yml", OLD_CHOOSE)],
        scripts: &[],
        expected: "machines/m.yml: approval: choose is not supported: write invoke: { model: { question: ... } } for a model, or invoke: { person: { question: ..., ask: <script> } } for a person (V19)\n",
    },
    Case {
        rule: "V19",
        name: "input beside a check",
        files: &[("machines/m.yml", OLD_INPUT)],
        scripts: &[],
        expected: "machines/m.yml: ok: input is not supported: name the state whose output is read with output, in the condition ({ output: <state>, matches: ... }) or in the model ({ model: { ..., output: <state> } }) (V19)\n",
    },
    Case {
        rule: "V19",
        name: "a bare matches",
        files: &[("machines/m.yml", OLD_BARE_MATCHES)],
        scripts: &[],
        expected: "machines/m.yml: ok: a bare matches is not supported: name the state it reads, { output: <state>, matches: ... } (V19)\n",
    },
    Case {
        rule: "V19",
        name: "max_attempts and timeout_s on a state",
        files: &[
            ("machines/a.yml", OLD_MAX_ATTEMPTS),
            ("machines/b.yml", OLD_TIMEOUT_S),
        ],
        scripts: &[],
        expected: "\
machines/a.yml: work: max_attempts on a state is not supported: write it inside the script invoke, invoke: { script: { name: <script>, max_attempts: <n> } } (V19)
machines/b.yml: work: timeout_s on a state is not supported: write it inside the invoke, invoke: { script: { name: <script>, timeout_s: <n> } } (or person: { ..., timeout_s: <n> }) (V19)
",
    },
    Case {
        rule: "V19",
        name: "params beside machine",
        files: &[
            ("machines/m.yml", OLD_MACHINE_PARAMS),
            ("machines/n.yml", V19_CHILD),
        ],
        scripts: &[],
        expected: "machines/m.yml: work: { machine: <name>, params: ... } is not supported: write invoke: { machine: { name: <name>, params: ... } } (V19)\n",
    },
    // V20
    Case {
        rule: "V20",
        name: "a child machine that invokes no machine",
        files: &[
            ("machines/m.yml", V20_M),
            (
                "machines/n.yml",
                "\
name: n
description: Runs a script.
initial: work
states:
  work:
    invoke: work
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
",
            ),
        ],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V20",
        name: "two machines that invoke each other",
        files: &[
            ("machines/m.yml", V20_M),
            (
                "machines/n.yml",
                "\
name: n
description: Runs m as a child.
initial: work
states:
  work:
    invoke: { machine: m }
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
",
            ),
        ],
        scripts: &[],
        expected: "\
machines/m.yml: work: machine `m` invokes itself: m -> n -> m; a machine never invokes itself, directly or through others (V20)
machines/n.yml: work: machine `n` invokes itself: n -> m -> n; a machine never invokes itself, directly or through others (V20)
",
    },
    // V21
    Case {
        rule: "V21",
        name: "done.state and error do not overlap",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: A compound state that handles its own completion by its full name.
initial: work
states:
  work:
    initial: step
    transitions: { done.state.work: done, error: failed }
    states:
      step:
        invoke: work
        transitions: { done: finished }
      finished: { final: true }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V21",
        name: "done and done.state overlap",
        files: &[(
            "machines/m.yml",
            "\
name: m
description: A compound state that handles its own completion by its full name.
initial: work
states:
  work:
    initial: step
    transitions: { done: done, done.state.work: failed }
    states:
      step:
        invoke: work
        transitions: { done: finished }
      finished: { final: true }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: "machines/m.yml: work: events `done` and `done.state.work` overlap: `done` also matches `done.state.work`, so at most one transition per state may match an event (V21)\n",
    },
    // M1
    Case {
        rule: "M1",
        name: "pending migrations name m with valid params; processed ones are not checked",
        files: &[
            ("machines/m.yml", TYPED),
            ("processed.md", "00-done.md\n"),
            (
                "migrations/00-done.md",
                "---\nmachine: gone\n---\n# Already processed, so not checked\n",
            ),
            (
                "migrations/01-first.md",
                "---\nmachine: m\nparams:\n  max_rounds: 3\n  strict: false\n---\n# First\n",
            ),
            (
                "migrations/02-second.md",
                "---\nroutine: m\n---\n# Uses the routine alias\n",
            ),
        ],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "M1",
        name: "an unknown param, an unknown machine, no machine key",
        files: &[
            ("machines/m.yml", TYPED),
            ("processed.md", ""),
            (
                "migrations/01-first.md",
                "---\nmachine: m\nparams:\n  rounds: 3\n---\n# First\n",
            ),
            (
                "migrations/02-second.md",
                "---\nroutine: nope\n---\n# Unknown machine\n",
            ),
            ("migrations/03-third.md", "# No machine key\n"),
        ],
        scripts: &[],
        expected: "\
migrations/01-first.md: line 3: unknown param `rounds`: machine `m` has no data `rounds` (M1)
migrations/02-second.md: line 2: unknown machine `nope` (M1)
migrations/03-third.md: line 1: no `machine` key (M1)
",
    },
    // M2
    Case {
        rule: "M2",
        name: "a message, a BOM and CRLF, a reply; hidden files are not checked",
        files: &[
            ("machines/m.yml", TYPED),
            ("inbox/a.md", "---\nmachine: m\n---\n# Plain message\n"),
            (
                "inbox/b.md",
                "\u{feff}---\r\nmachine: m\r\nparams: { mode: slow }\r\n---\r\nCRLF body\r\n",
            ),
            (
                "inbox/c.md",
                "---\nto: 20261001T120000Z-0b12aa.w3\nevent: approve\n---\nA reply, not a run.\n",
            ),
            ("inbox/.d.md.tmp", "---\nunclosed\n"),
            ("inbox/.e.md", "---\nunclosed\n"),
        ],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "M2",
        name: "no machine, a param of the wrong type, no closing fence, a duplicate key",
        files: &[
            ("machines/m.yml", TYPED),
            ("inbox/a.md", "# No frontmatter, so no machine\n"),
            (
                "inbox/b.md",
                "---\nmachine: m\nparams:\n  max_rounds: three\n---\nbody\n",
            ),
            (
                "inbox/c.md",
                "---\nmachine: m\nbody without a closing fence\n",
            ),
            ("inbox/d.md", "---\nmachine: m\nmachine: m\n---\n"),
        ],
        scripts: &[],
        expected: "\
inbox/a.md: line 1: no `machine` key (M2)
inbox/b.md: line 3: param `max_rounds` must be of type `int` (M2)
inbox/c.md: line 1: frontmatter has an opening `---` but no closing `---` (M2)
inbox/d.md: line 3: duplicate entry with key \"machine\" (M2)
",
    },
    // M3
    Case {
        rule: "M3",
        name: "a cron file with a machine and params",
        files: &[
            ("machines/m.yml", TYPED),
            (
                "cron/nightly.md",
                "---\ncron: \"0 2 * * *\"\nmachine: m\nparams:\n  mode: nightly\n---\n# Nightly\n",
            ),
        ],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "M3",
        name: "no cron key, an unknown machine, no machine key",
        files: &[
            ("machines/m.yml", TYPED),
            ("cron/a.md", "---\nmachine: m\n---\n# No cron key\n"),
            (
                "cron/b.md",
                "---\ncron: \"0 2 * * *\"\nmachine: nope\n---\n# Unknown machine\n",
            ),
            (
                "cron/c.md",
                "---\ncron: \"0 3 * * *\"\n---\n# No machine key\n",
            ),
        ],
        scripts: &[],
        expected: "\
cron/a.md: line 1: no `cron:` expression (M3)
cron/b.md: line 3: unknown machine `nope` (M3)
cron/c.md: line 1: no `machine` key (M3)
",
    },
];

/// Root and state `onentry`/`onexit` scripts (V12).
const V12_MACHINE: &str = "\
name: m
description: Root and state onentry/onexit scripts.
onentry: [setup]
onexit: [notify]
initial: work
states:
  work:
    invoke: work
    onentry: [snapshot]
    transitions: { done: done }
  done: { final: true, onentry: [commit] }
  failed: { final: true }
";

/// A `model` state with a valid and an invalid `min_confidence` (V16).
const V16_MACHINE: (&str, &str) = (
    "\
name: m
description: A model picks one option, through the machine named router.
initial: work
states:
  work:
    invoke: work
    transitions: { done: triage }
  triage:
    invoke:
      model:
        question: Is it done?
        min_confidence: 0.8
    transitions:
      finish: { target: done, description: It is done. }
      again:  { target: work, description: Work on it again. }
      unsure: failed
  done: { final: true }
  failed: { final: true }
",
    "\
name: m
description: A model picks one option, through the machine named router.
initial: work
states:
  work:
    invoke: work
    transitions: { done: triage }
  triage:
    invoke:
      model:
        question: Is it done?
        min_confidence: 80
    transitions:
      finish: { target: done, description: It is done. }
      again:  { target: work, description: Work on it again. }
      unsure: failed
  done: { final: true }
  failed: { final: true }
",
);

/// `choose: person`, which `person:` replaced (V19).
const OLD_CHOOSE: &str = "\
name: m
description: A person picks one option, written the old way.
initial: approval
states:
  approval:
    invoke: { choose: person, question: Ship it?, ask: ask }
    transitions:
      approve: { target: done, description: Ship it. }
      reject:  { target: failed, description: Do not ship. }
  done: { final: true }
  failed: { final: true }
";

/// `input:` beside `check:`, which `output:` in the condition replaced (V19).
const OLD_INPUT: &str = "\
name: m
description: A check on a script's output, written the old way.
initial: work
states:
  work:
    invoke: work
    transitions: { done: ok }
  ok:
    invoke: { check: { matches: \"^ok\" }, input: work }
    transitions: { true: done, false: work }
  done: { final: true }
  failed: { final: true }
";

/// A `matches` with no subject, which read the most recent script's output (V19).
const OLD_BARE_MATCHES: &str = "\
name: m
description: A check on the latest output, written the old way.
initial: work
states:
  work:
    invoke: work
    transitions: { done: ok }
  ok:
    invoke:
      check: { matches: \"^ok\" }
    transitions: { true: done, false: work }
  done: { final: true }
  failed: { final: true }
";

/// `max_attempts` on a state, which moved inside `invoke: { script: … }` (V19).
const OLD_MAX_ATTEMPTS: &str = "\
name: a
description: Retries written the old way.
initial: work
states:
  work:
    invoke: work
    max_attempts: 2
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

/// `timeout_s` on a state, which moved inside `invoke: { script: … }` (V19).
const OLD_TIMEOUT_S: &str = "\
name: b
description: A time limit written the old way.
initial: work
states:
  work:
    invoke: work
    timeout_s: 60
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

/// `{ machine: x, params }`, which `{ machine: { name: x, params } }` replaced (V19).
const OLD_MACHINE_PARAMS: &str = "\
name: m
description: A child machine with params, written the old way.
initial: work
states:
  work:
    invoke: { machine: n, params: { label: release } }
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

/// The child of `OLD_MACHINE_PARAMS`.
const V19_CHILD: &str = "\
name: n
description: A child machine with one datum.
data:
  label: { type: string, default: none }
initial: work
states:
  work:
    invoke: work
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

/// A 0.4 `router:` on a state inside a compound state (V19).
const ROUTER_ON_A_STATE: &str = "\
name: b
description: A router state inside a compound state.
initial: work
states:
  work:
    initial: step
    transitions: { done.state.work: done }
    states:
      step:
        invoke: work
        router: llm
        transitions:
          pass: { target: fin, description: The work is finished. }
          retry: { target: step, description: Run the work again. }
      fin: { final: true }
  done: { final: true }
  failed: { final: true }
";

/// Runs `n` as a child (V20).
const V20_M: &str = "\
name: m
description: Runs n as a child.
initial: work
states:
  work:
    invoke: { machine: n }
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

/// The script names in a machine (V12): a string `invoke`, `ask`, and `onentry`/`onexit`.
fn script_names(value: &Value, names: &mut Vec<String>) {
    match value {
        Value::Mapping(map) => {
            for (key, value) in map {
                match (key.as_str(), value) {
                    (Some("invoke" | "script" | "ask"), Value::String(name)) => {
                        names.push(name.clone())
                    }
                    (Some("script"), Value::Mapping(script)) => {
                        names.extend(script.get("name").and_then(Value::as_str).map(String::from))
                    }
                    (Some("onentry" | "onexit"), Value::Sequence(list)) => {
                        names.extend(list.iter().filter_map(|v| v.as_str()).map(String::from))
                    }
                    _ => script_names(value, names),
                }
            }
        }
        Value::Sequence(list) => list.iter().for_each(|v| script_names(v, names)),
        _ => {}
    }
}

fn write(path: &Path, text: &str, mode: u32) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// Write `files` and the scripts the machines need into `decree`.
fn write_project(decree: &Path, files: &[(&str, &str)], scripts: &[Script]) {
    let mut names = Vec::new();
    for (path, text) in files {
        write(&decree.join(path), text, 0o644);
        if path.starts_with("machines/") {
            if let Ok(value) = serde_norway::from_str::<Value>(text) {
                script_names(&value, &mut names);
            }
        }
    }
    for script in scripts {
        match script {
            Script::Missing(_) => {}
            Script::At(path) => write(&decree.join("scripts").join(path), STUB, 0o755),
            Script::NotExecutable(path) => write(&decree.join("scripts").join(path), STUB, 0o644),
        }
    }
    for name in names {
        if scripts.iter().all(|s| s.name() != name) {
            write(&decree.join("scripts").join(&name), STUB, 0o755);
        }
    }
}

/// `decree check` in a temp project holding `files`: (exit code, stdout).
fn check(files: &[(&str, &str)], scripts: &[Script]) -> (i32, String) {
    let tmp = TempDir::new().unwrap();
    write_project(&tmp.path().join(".decree"), files, scripts);
    let out = cargo_bin_cmd!("decree")
        .current_dir(tmp.path())
        .env("NO_COLOR", "1")
        .arg("check")
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
    )
}

/// Every case of `rule`: a passing one exits 0 and prints nothing; a failing one exits 1 and
/// prints exactly `expected`, each line naming the rule.
fn run_rule(rule: &str) {
    let cases: Vec<&Case> = CASES.iter().filter(|c| c.rule == rule).collect();
    assert!(!cases.is_empty(), "no case for {rule}");
    for case in cases {
        let (code, stdout) = check(case.files, case.scripts);
        let label = format!("{rule}: {}", case.name);
        if case.expected == PASSES {
            assert_eq!((code, stdout.as_str()), (0, ""), "{label}");
            continue;
        }
        assert_eq!(code, 1, "{label}: {stdout}");
        assert_eq!(stdout, case.expected, "{label}");
        for line in stdout.lines() {
            assert!(line.ends_with(&format!("({rule})")), "{label}: {line}");
        }
    }
}

const RULES: [&str; 24] = [
    "V1", "V2", "V3", "V4", "V5", "V6", "V7", "V8", "V9", "V10", "V11", "V12", "V13", "V14", "V15",
    "V16", "V17", "V18", "V19", "V20", "V21", "M1", "M2", "M3",
];

#[test]
fn every_rule_has_a_passing_and_a_failing_case() {
    for rule in RULES {
        let cases = CASES.iter().filter(|c| c.rule == rule);
        let (pass, fail): (Vec<&Case>, Vec<&Case>) = cases.partition(|c| c.expected == PASSES);
        assert!(!pass.is_empty() && !fail.is_empty(), "{rule}");
    }
    assert!(CASES.iter().all(|c| RULES.contains(&c.rule)));
}

macro_rules! rule_tests {
    ($($test:ident => $rule:literal,)*) => {
        $(
            #[test]
            fn $test() {
                run_rule($rule);
            }
        )*
    };
}

rule_tests! {
    v1_name => "V1",
    v2_state_ids => "V2",
    v3_initial => "V3",
    v4_targets => "V4",
    v5_failed => "V5",
    v6_compound => "V6",
    v7_final => "V7",
    v8_decisions => "V8",
    v9_output => "V9",
    v10_condition => "V10",
    v11_reachable => "V11",
    v12_scripts => "V12",
    v13_emits => "V13",
    v14_data => "V14",
    v15_done_state => "V15",
    v16_invokes => "V16",
    v17_internal => "V17",
    v18_events => "V18",
    v19_scxml => "V19",
    v20_cycles => "V20",
    v21_overlapping_events => "V21",
    m1_migrations => "M1",
    m2_inbox => "M2",
    m3_cron => "M3",
}

/// Four machines, each wrong in its own way: every error is its own line.
#[test]
fn several_invalid_machines_print_one_line_per_error() {
    let files = [
        (
            "machines/a.yml",
            "\
name: a
description: Base machine.
initial: work
states:
  work:
    invoke: work
    transitions: { done: nowhere }
  done: { final: true }
  failed: { final: true }
",
        ),
        (
            "machines/b.yml",
            "\
name: b
description: Base machine.
initial: work
states:
  work:
    invoke: work
    transitions: { done: done }
  done: { final: true }
",
        ),
        (
            "machines/c.yml",
            "name: c\ndescription: d\ninitial: x\nstates:\n  x: { invok: work }\n",
        ),
        (
            "machines/d.yml",
            "\
name: d
description: A check that handles neither of its events.
initial: work
states:
  work:
    invoke: work
    transitions: { done: again, skip: done }
  again:
    invoke:
      check: { visits: work, less_than: 2 }
    transitions: { pass: done }
  done: { final: true }
  failed: { final: true }
",
        ),
    ];
    let (code, stdout) = check(&files, &[]);
    assert_eq!(code, 1);
    assert_eq!(
        stdout,
        "\
machines/a.yml: work: transition `done` targets unknown state `nowhere` (V4)
machines/a.yml: done: state is unreachable from the root `initial` (V11)
machines/b.yml: line 4: no root-level final state `failed`: every machine needs one for unhandled errors (V5)
machines/c.yml: x: unknown field `invok`, expected one of `final`, `description`, `invoke`, `onentry`, `onexit`, `initial`, `states`, `transitions`, `emits` (V19)
machines/d.yml: again: a `check` state must handle `true`, itself or through an ancestor (V8)
machines/d.yml: again: a `check` state must handle `false`, itself or through an ancestor (V8)
"
    );
}
