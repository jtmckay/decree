//! `decree check` for each validation rule of docs/reference/machines.md (Validation), V1–V21
//! and M1–M3, as a table. Each case is a rule, a name, the files of `.decree/` inline, and the
//! exact stdout of `decree check` (or `PASSES`: exit 0 and no output). The helper writes the
//! case into a temp project and adds a stub executable for every script the machines
//! reference, except where the case places a script itself or says it is missing.
//!
//! The same cases hold the JSON Schemas to `decree check` (docs/reference/machines.md, Schema):
//! every file of a passing case validates, and every file a failing case reports is rejected
//! by the schema, unless `CHECK_ONLY` lists it with the reason only `decree check` can tell.

mod common;

use serde_norway::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[path = "common/schema.rs"]
mod schema;

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

/// `BASE` as a plugin, `plugins/m.md`, with its script, the `STUB` every case gets.
const PLUGIN_BASE: &str = "\
---
name: m
machine:
  description: Base machine.
  initial: work
  states:
    work:
      invoke: work
      transitions: { done: done }
    done: { final: true }
    failed: { final: true }
scripts:
  work: |
    #!/usr/bin/env bash
    exit 0
---
# m
";

/// `PLUGIN_BASE` with another description.
const PLUGIN_CHANGED: &str = "\
---
name: m
machine:
  description: Changed machine.
  initial: work
  states:
    work:
      invoke: work
      transitions: { done: done }
    done: { final: true }
    failed: { final: true }
scripts:
  work: |
    #!/usr/bin/env bash
    exit 0
---
# m
";

/// `BASE` edited in place.
const BASE_EDITED: &str = "\
name: m
description: Edited in place.
initial: work
states:
  work:
    invoke: work
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

/// A plugin `r`, with its script.
const PLUGIN_R: &str = "\
---
name: r
machine:
  description: Another machine.
  initial: work
  states:
    work:
      invoke: work
      transitions: { done: done }
    done: { final: true }
    failed: { final: true }
scripts:
  work: |
    #!/usr/bin/env bash
    exit 0
---
";

/// A machine with one datum of each type.
/// An `enum` and a `number`, compared with a float, an int and an int `data` value.
const CHOICES: &str = "\
name: m
description: An enum and a number.
data:
  need: { type: string, enum: [plan, fix], default: fix }
  megapixels: { type: number, default: 1.0 }
  max: { type: int, default: 2 }
initial: big
states:
  big:
    invoke: { check: { data: megapixels, more_than: 0.75 } }
    transitions: { true: capped, false: one }
  one:
    invoke: { check: { data: megapixels, equals: 1 } }
    transitions: { true: capped, false: capped }
  capped:
    invoke: { check: { data: megapixels, at_most: { data: max } } }
    transitions: { true: work, false: work }
  work:
    invoke: work
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

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
        files: &[("machines/m/machine.yml", BASE)],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V1",
        name: "name differs from the file stem",
        files: &[(
            "machines/m/machine.yml",
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
        expected: "machines/m/machine.yml: line 1: name `other` does not equal the file stem `m` (V1)\n",
    },
    // V2
    Case {
        rule: "V2",
        name: "lowercase state ids",
        files: &[("machines/m/machine.yml", BASE)],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V2",
        name: "a state id with a capital",
        files: &[(
            "machines/m/machine.yml",
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
        expected: "machines/m/machine.yml: Work: state id `Work` does not match ^[a-z][a-z0-9_]*$ (V2)\n",
    },
    // V3
    Case {
        rule: "V3",
        name: "initial names a child",
        files: &[("machines/m/machine.yml", BASE)],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V3",
        name: "initial names no child",
        files: &[(
            "machines/m/machine.yml",
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
        expected: "machines/m/machine.yml: line 3: initial `nowhere` is not a direct child state (V3)\n",
    },
    // V4
    Case {
        rule: "V4",
        name: "every target exists",
        files: &[("machines/m/machine.yml", BASE)],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V4",
        name: "a target that does not exist",
        files: &[(
            "machines/m/machine.yml",
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
        expected: "machines/m/machine.yml: work: transition `skip` targets unknown state `nowhere` (V4)\n",
    },
    // V5
    Case {
        rule: "V5",
        name: "a root final state named failed",
        files: &[("machines/m/machine.yml", BASE)],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V5",
        name: "no state named failed",
        files: &[(
            "machines/m/machine.yml",
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
        expected: "machines/m/machine.yml: line 4: no root-level final state `failed`: every machine needs one for unhandled errors (V5)\n",
    },
    // V6
    Case {
        rule: "V6",
        name: "a compound state without invoke",
        files: &[(
            "machines/m/machine.yml",
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
            "machines/m/machine.yml",
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
        expected: "machines/m/machine.yml: work: compound state may not have `invoke` (V6)\n",
    },
    // V7
    Case {
        rule: "V7",
        name: "final states with only final",
        files: &[("machines/m/machine.yml", BASE)],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V7",
        name: "a final state with transitions",
        files: &[(
            "machines/m/machine.yml",
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
        expected: "machines/m/machine.yml: done: final state may only have `final`, `description`, `onentry` and `emits`, not `transitions` (V7)\n",
    },
    // V8
    Case {
        rule: "V8",
        name: "a check handles true and false, a person is asked a question",
        files: &[(
            "machines/m/machine.yml",
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
      reply:
        question: Ship it?
        notify: ask
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
            "machines/m/machine.yml",
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
      reply: { notify: ask }
    transitions:
      approve: { target: done, description: Ship it. }
      reject:  failed
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: "\
machines/m/machine.yml: again: a `check` state must handle `false`, itself or through an ancestor (V8)
machines/m/machine.yml: approval: a `reply` state needs a `question`: what is being decided (V8)
machines/m/machine.yml: approval: option `reject` needs a `description`: write it as `reject: { target: failed, description: ... }` (V8)
",
    },
    Case {
        rule: "V8",
        name: "a check still written with yes and no",
        files: &[(
            "machines/m/machine.yml",
            "\
name: m
description: A check whose transitions use the old yes and no.
initial: work
states:
  work:
    invoke: work
    transitions: { done: again, skip: done }
  again:
    invoke:
      check: { visits: work, less_than: 2 }
    transitions: { yes: work, no: done }
  done: { final: true }
  failed: { final: true }
",
        )],
        scripts: &[],
        expected: "\
machines/m/machine.yml: again: a `check` produces `true` and `false`: rename its `yes` and `no` transitions to `true` and `false` (V8)
",
    },
    // V9
    Case {
        rule: "V9",
        name: "output names a script state",
        files: &[(
            "machines/m/machine.yml",
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
            "machines/m/machine.yml",
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
        expected: "machines/m/machine.yml: ok: output `ok` is not a state with a script invoke (V9)\n",
    },
    // V10
    Case {
        rule: "V10",
        name: "a condition on existing data",
        files: &[(
            "machines/m/machine.yml",
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
            "machines/m/machine.yml",
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
        expected: "machines/m/machine.yml: again: check: unknown data `rounds` (V10)\n",
    },
    // V11
    Case {
        rule: "V11",
        name: "every state is reachable and can finish",
        files: &[(
            "machines/m/machine.yml",
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
            "machines/m/machine.yml",
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
machines/m/machine.yml: orphan: state is unreachable from the root `initial` (V11)
machines/m/machine.yml: wait: state cannot reach a root-level final state (V11)
",
    },
    // V12
    Case {
        rule: "V12",
        name: "every script resolves, one in the machine's folder",
        files: &[("machines/m/machine.yml", V12_MACHINE)],
        scripts: &[Script::At("m/snapshot.sh")],
        expected: PASSES,
    },
    Case {
        rule: "V12",
        name: "a missing script and one that is not executable",
        files: &[("machines/m/machine.yml", V12_MACHINE)],
        scripts: &[
            Script::Missing("setup"),
            Script::NotExecutable("snapshot.sh"),
        ],
        expected: "\
machines/m/machine.yml: line 3: script `setup` not found; searched machines/m/scripts (V12)
machines/m/machine.yml: work: script `snapshot`: machines/m/scripts/snapshot.sh is not executable (V12)
",
    },
    Case {
        rule: "V12",
        name: "a script with no extension named like its machine",
        files: &[(
            "machines/verify/machine.yml",
            "\
name: verify
description: Its script is machines/verify/scripts/verify, with no extension.
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
            "machines/m/machine.yml",
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
            "machines/m/machine.yml",
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
        expected: "machines/m/machine.yml: work: emits unknown machine `nope` (V13)\n",
    },
    // V14
    Case {
        rule: "V14",
        name: "every default matches its type",
        files: &[("machines/m/machine.yml", TYPED)],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V14",
        name: "a string default for a bool",
        files: &[(
            "machines/m/machine.yml",
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
        expected: "machines/m/machine.yml: line 6: data `strict`: default `yes` is not of type `bool` (V14)\n",
    },
    Case {
        rule: "V14",
        name: "an enum and a number, compared numerically",
        files: &[("machines/m/machine.yml", CHOICES)],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V14",
        name: "an enum default not in the list, an enum on an int, an empty or repeating enum, a non-number",
        files: &[(
            "machines/m/machine.yml",
            "\
name: m
description: Typed data.
data:
  need: { type: string, enum: [plan, fix], default: review }
  rounds: { type: int, enum: [one], default: 1 }
  empty: { type: string, enum: [], default: x }
  twice: { type: string, enum: [a, a], default: a }
  megapixels: { type: number, default: x }
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
        expected: "\
machines/m/machine.yml: line 6: data `empty`: `enum` is empty (V14)
machines/m/machine.yml: line 8: data `megapixels`: default `x` is not of type `number` (V14)
machines/m/machine.yml: line 4: data `need`: default `review` is not one of the `enum` values: plan, fix (V14)
machines/m/machine.yml: line 5: data `rounds`: `enum` is for type `string`, not `int` (V14)
machines/m/machine.yml: line 7: data `twice`: `enum` lists `a` twice (V14)
",
    },
    Case {
        rule: "V14",
        name: "state_files names are files or folders in the state folder, with a description",
        files: &[(
            "machines/m/machine.yml",
            "\
name: m
description: Stored.
state_files:
  seen.tsv: Links already sent. work reads and appends.
  \"a/b\": x
  cache: \"  \"
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
        expected: "\
machines/m/machine.yml: line 5: state_files `a/b`: the name does not match ^[A-Za-z0-9][A-Za-z0-9._-]*$: a file or folder directly in the state folder (V14)
machines/m/machine.yml: line 6: state_files `cache`: the description is empty: say what it is and which states read or write it (V14)
",
    },
    // V15
    Case {
        rule: "V15",
        name: "done.state is handled",
        files: &[(
            "machines/m/machine.yml",
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
            "machines/m/machine.yml",
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
        expected: "machines/m/machine.yml: work: compound state has a final state, but nothing handles `done.state.work`, itself or through an ancestor (V15)\n",
    },
    // V16
    Case {
        rule: "V16",
        name: "a model with a machine named router",
        files: &[
            ("machines/m/machine.yml", V16_MACHINE.0),
            (
                "machines/router/machine.yml",
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
            ("machines/m/machine.yml", V16_MACHINE.1),
            (
                "machines/picker/machine.yml",
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
machines/m/machine.yml: triage: `model` names no `router`, and there is no machine named `router` (V16)
machines/m/machine.yml: triage: min_confidence 80 is not between 0 and 1 (V16)
",
    },
    Case {
        rule: "V14",
        name: "an env_file named .env.<name>",
        files: &[(
            "machines/m/machine.yml",
            "\
name: m
description: One script with its own secrets.
env_file: .env.comfy
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
        expected: PASSES,
    },
    Case {
        rule: "V14",
        name: "an env_file that is the committed template",
        files: &[(
            "machines/m/machine.yml",
            "\
name: m
description: One script with its own secrets.
env_file: .env.example
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
        expected: "\
machines/m/machine.yml: line 3: env_file `.env.example` is not `.env.<name>` with <name> matching ^[A-Za-z0-9][A-Za-z0-9._-]*$ (and not `.env.example`): a file directly in .decree/ that .gitignore's `.env*` keeps out of git (V14)
",
    },
    Case {
        rule: "V16",
        name: "a timeout in each unit",
        files: &[
            ("machines/a/machine.yml", "\
name: a
description: A script with a time limit.
initial: work
states:
  work:
    invoke: { script: { name: work, timeout: 90s } }
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
"),
            ("machines/b/machine.yml", "\
name: b
description: A script with a time limit.
initial: work
states:
  work:
    invoke: { script: { name: work, timeout: 10m } }
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
"),
            ("machines/c/machine.yml", "\
name: c
description: A script with a time limit.
initial: work
states:
  work:
    invoke: { script: { name: work, timeout: 12h } }
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
"),
            ("machines/d/machine.yml", TIMEOUT_REPLY),
        ],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V16",
        name: "attempts as a count and as a list of model ids",
        files: &[
            ("machines/a/machine.yml", "\
name: a
description: Retries.
initial: work
states:
  work:
    invoke: { script: { name: work, attempts: 3 } }
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
"),
            ("machines/b/machine.yml", "\
name: b
description: A local model twice, then a hosted one.
initial: work
states:
  work:
    invoke: { script: { name: work, attempts: [qwen3:8b, qwen3:8b, claude-opus-5-5, local] } }
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
"),
        ],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V16",
        name: "attempts empty, zero, or with an entry that has a space",
        files: &[
            ("machines/a/machine.yml", "\
name: a
description: No attempts.
initial: work
states:
  work:
    invoke: { script: { name: work, attempts: [] } }
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
"),
            ("machines/b/machine.yml", "\
name: b
description: Zero attempts.
initial: work
states:
  work:
    invoke: { script: { name: work, attempts: 0 } }
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
"),
            ("machines/c/machine.yml", "\
name: c
description: A value with a space.
initial: work
states:
  work:
    invoke: { script: { name: work, attempts: [local, \"claude opus\"] } }
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
"),
        ],
        scripts: &[],
        expected: "\
machines/a/machine.yml: work: attempts: [] lists no attempts (V16)
machines/b/machine.yml: work: attempts: 0 is not a positive integer (V16)
machines/c/machine.yml: work: attempts entry `claude opus` is not a valid value: it starts with a letter or digit, then up to 127 of letters, digits and `._:/@-` (V16)
",
    },
    Case {
        rule: "V16",
        name: "a timeout that is not a duration",
        files: &[
            ("machines/a/machine.yml", "\
name: a
description: A script with a time limit.
initial: work
states:
  work:
    invoke: { script: { name: work, timeout: 1.5h } }
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
"),
            ("machines/b/machine.yml", "\
name: b
description: A script with a time limit.
initial: work
states:
  work:
    invoke: { script: { name: work, timeout: 1h30m } }
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
"),
            ("machines/c/machine.yml", "\
name: c
description: A script with a time limit.
initial: work
states:
  work:
    invoke: { script: { name: work, timeout: 10 } }
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
"),
            ("machines/d/machine.yml", "\
name: d
description: A script with a time limit.
initial: work
states:
  work:
    invoke: { script: { name: work, timeout: -1m } }
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
"),
            ("machines/e/machine.yml", "\
name: e
description: A script with a time limit.
initial: work
states:
  work:
    invoke: { script: { name: work, timeout: 1w } }
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
"),
            ("machines/f/machine.yml", "\
name: f
description: A script with a time limit.
initial: work
states:
  work:
    invoke: { script: { name: work, timeout: '' } }
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
"),
        ],
        scripts: &[],
        expected: "\
machines/a/machine.yml: work: timeout: `1.5h` is not a duration: a whole number of at most 9 digits followed by s, m, h or d, such as 90s, 10m, 12h or 7d (V16)
machines/b/machine.yml: work: timeout: `1h30m` is not a duration: a whole number of at most 9 digits followed by s, m, h or d, such as 90s, 10m, 12h or 7d (V16)
machines/c/machine.yml: work: timeout: `10` is not a duration: a whole number of at most 9 digits followed by s, m, h or d, such as 90s, 10m, 12h or 7d (V16)
machines/d/machine.yml: work: timeout: `-1m` is not a duration: a whole number of at most 9 digits followed by s, m, h or d, such as 90s, 10m, 12h or 7d (V16)
machines/e/machine.yml: work: timeout: `1w` is not a duration: a whole number of at most 9 digits followed by s, m, h or d, such as 90s, 10m, 12h or 7d (V16)
machines/f/machine.yml: work: timeout: `` is not a duration: a whole number of at most 9 digits followed by s, m, h or d, such as 90s, 10m, 12h or 7d (V16)
",
    },
    // V17
    Case {
        rule: "V17",
        name: "internal from a compound state to its child",
        files: &[(
            "machines/m/machine.yml",
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
            "machines/m/machine.yml",
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
        expected: "machines/m/machine.yml: work: transition `done`: `type: internal` is only allowed from a compound state to one of its descendants (V17)\n",
    },
    // V18
    Case {
        rule: "V18",
        name: "lowercase events",
        files: &[(
            "machines/m/machine.yml",
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
            "machines/m/machine.yml",
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
        expected: "machines/m/machine.yml: work: event `Pass` does not match ^[a-z][a-z0-9_]*(\\.[a-z0-9_]+)*$ (V18)\n",
    },
    // V19
    Case {
        rule: "V19",
        name: "only the SCXML subset",
        files: &[("machines/m/machine.yml", BASE)],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "V19",
        name: "cond on a transition, router on a state",
        files: &[
            (
                "machines/a/machine.yml",
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
            ("machines/b/machine.yml", ROUTER_ON_A_STATE),
        ],
        scripts: &[],
        expected: "\
machines/a/machine.yml: work: transition `done`: cond on a transition is not supported: make the decision a state with invoke: { check: ... } (V19)
machines/b/machine.yml: work.step: router on a state is not supported: make the decision a state with invoke: { model: { question: ... } } (V19)
",
    },
    Case {
        rule: "V19",
        name: "choose, the old decision invoke",
        files: &[("machines/m/machine.yml", OLD_CHOOSE)],
        scripts: &[],
        expected: "machines/m/machine.yml: approval: choose is not supported: write invoke: { model: { question: ... } } for a model, or invoke: { reply: { question: ..., notify: <script> } } to wait for a reply (V19)\n",
    },
    Case {
        rule: "V19",
        name: "person with ask, the old name of reply with notify",
        files: &[("machines/m/machine.yml", OLD_PERSON)],
        scripts: &[],
        expected: "machines/m/machine.yml: approval: person: is now reply: (and its ask: is notify:) (V19)\n",
    },
    Case {
        rule: "V19",
        name: "input beside a check",
        files: &[("machines/m/machine.yml", OLD_INPUT)],
        scripts: &[],
        expected: "machines/m/machine.yml: ok: input is not supported: name the state whose output is read with output, in the condition ({ output: <state>, matches: ... }) or in the model ({ model: { ..., output: <state> } }) (V19)\n",
    },
    Case {
        rule: "V19",
        name: "a bare matches",
        files: &[("machines/m/machine.yml", OLD_BARE_MATCHES)],
        scripts: &[],
        expected: "machines/m/machine.yml: ok: a bare matches is not supported: name the state it reads, { output: <state>, matches: ... } (V19)\n",
    },
    Case {
        rule: "V19",
        name: "max_attempts and timeout_s on a state",
        files: &[
            ("machines/a/machine.yml", OLD_MAX_ATTEMPTS),
            ("machines/b/machine.yml", OLD_TIMEOUT_S),
        ],
        scripts: &[],
        expected: "\
machines/a/machine.yml: work: max_attempts is not supported: write `attempts: 2` or `attempts: [<value>, …]` inside the script invoke, invoke: { script: { name: <script>, attempts: … } } (V19)
machines/b/machine.yml: work: timeout_s on a state is not supported: write timeout: <n>s|m|h|d inside the invoke, invoke: { script: { name: <script>, timeout: <n>s|m|h|d } } (or reply: { ..., timeout: <n>s|m|h|d }) (V19)
",
    },
    Case {
        rule: "V19",
        name: "max_attempts in a script invoke, attempts in a model invoke",
        files: &[
            ("machines/a/machine.yml", "\
name: a
description: Retries written the old way.
initial: work
states:
  work:
    invoke: { script: { name: work, max_attempts: 2 } }
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
"),
            ("machines/b/machine.yml", "\
name: b
description: Attempts on a model.
initial: pick
states:
  pick:
    invoke:
      model: { question: Which?, attempts: 2 }
    transitions:
      a: { target: done, description: A. }
      b: { target: done, description: B. }
  done: { final: true }
  failed: { final: true }
"),
        ],
        scripts: &[],
        expected: "\
machines/a/machine.yml: work: max_attempts is not supported: write `attempts: 2` or `attempts: [<value>, …]` (V19)
machines/b/machine.yml: pick: unknown field `attempts`, expected one of `question`, `router`, `min_confidence`, `output` (V19)
",
    },
    Case {
        rule: "V19",
        name: "timeout_s in a script and a reply invoke",
        files: &[
            ("machines/a/machine.yml", OLD_TIMEOUT_S_SCRIPT),
            ("machines/d/machine.yml", OLD_TIMEOUT_S_REPLY),
        ],
        scripts: &[],
        expected: "\
machines/a/machine.yml: work: timeout_s is not supported: write timeout: <n>s|m|h|d (V19)
machines/d/machine.yml: approval: timeout_s is not supported: write timeout: <n>s|m|h|d (V19)
",
    },
    Case {
        rule: "V19",
        name: "params beside machine",
        files: &[
            ("machines/m/machine.yml", OLD_MACHINE_PARAMS),
            ("machines/n/machine.yml", V19_CHILD),
        ],
        scripts: &[],
        expected: "machines/m/machine.yml: work: { machine: <name>, params: ... } is not supported: write invoke: { machine: { name: <name>, params: ... } } (V19)\n",
    },
    // V20
    Case {
        rule: "V20",
        name: "a child machine that invokes no machine",
        files: &[
            ("machines/m/machine.yml", V20_M),
            (
                "machines/n/machine.yml",
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
            ("machines/m/machine.yml", V20_M),
            (
                "machines/n/machine.yml",
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
machines/m/machine.yml: work: machine `m` invokes itself: m -> n -> m; a machine never invokes itself, directly or through others (V20)
machines/n/machine.yml: work: machine `n` invokes itself: n -> m -> n; a machine never invokes itself, directly or through others (V20)
",
    },
    // V21
    Case {
        rule: "V21",
        name: "done.state and error do not overlap",
        files: &[(
            "machines/m/machine.yml",
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
            "machines/m/machine.yml",
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
        expected: "machines/m/machine.yml: work: events `done` and `done.state.work` overlap: `done` also matches `done.state.work`, so at most one transition per state may match an event (V21)\n",
    },
    // M1
    Case {
        rule: "M1",
        name: "pending migrations name m with valid params; processed ones are not checked",
        files: &[
            ("machines/m/machine.yml", TYPED),
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
                "---\nmachine: m\n---\n# Second\n",
            ),
        ],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "M1",
        name: "an unknown param, an unknown machine, no machine key",
        files: &[
            ("machines/m/machine.yml", TYPED),
            ("processed.md", ""),
            (
                "migrations/01-first.md",
                "---\nmachine: m\nparams:\n  rounds: 3\n---\n# First\n",
            ),
            (
                "migrations/02-second.md",
                "---\nmachine: nope\n---\n# Unknown machine\n",
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
    Case {
        rule: "M1",
        name: "a param outside the enum, a number param that is not a number",
        files: &[
            ("machines/m/machine.yml", CHOICES),
            (
                "processed.md",
                "",
            ),
            (
                "migrations/01-first.md",
                "---\nmachine: m\nparams:\n  need: review\n---\n# First\n",
            ),
            (
                "migrations/02-second.md",
                "---\nmachine: m\nparams:\n  megapixels: x\n---\n# Second\n",
            ),
        ],
        scripts: &[],
        expected: "\
migrations/01-first.md: line 3: param `need` is `review`, not one of the allowed values: plan, fix (M1)
migrations/02-second.md: line 3: param `megapixels` must be of type `number` (M1)
",
    },
    // M2
    Case {
        rule: "M2",
        name: "a message, a BOM and CRLF, a reply; hidden files are not checked",
        files: &[
            ("machines/m/machine.yml", TYPED),
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
            ("machines/m/machine.yml", TYPED),
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
    Case {
        rule: "M2",
        name: "an enum value, a float and an int for a number",
        files: &[
            ("machines/m/machine.yml", CHOICES),
            (
                "inbox/a.md",
                "---\nmachine: m\nparams: { need: plan, megapixels: 1.5 }\n---\nbody\n",
            ),
            (
                "inbox/b.md",
                "---\nmachine: m\nparams: { megapixels: 2 }\n---\nbody\n",
            ),
        ],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "M2",
        name: "a param outside the enum",
        files: &[
            ("machines/m/machine.yml", CHOICES),
            (
                "inbox/a.md",
                "---\nmachine: m\nparams: { need: review }\n---\nbody\n",
            ),
        ],
        scripts: &[],
        expected: "inbox/a.md: line 3: param `need` is `review`, not one of the allowed values: plan, fix (M2)\n",
    },
    // M3
    Case {
        rule: "M3",
        name: "a cron file with a machine and params",
        files: &[
            ("machines/m/machine.yml", TYPED),
            (
                "machines/m/cron/nightly.md",
                "---\ncron: \"0 2 * * *\"\nmachine: m\nparams:\n  mode: nightly\n---\n# Nightly\n",
            ),
        ],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "M3",
        name: "no cron key, an unknown machine",
        files: &[
            ("machines/m/machine.yml", TYPED),
            ("machines/m/cron/a.md", "---\nmachine: m\n---\n# No cron key\n"),
            (
                "machines/m/cron/b.md",
                "---\ncron: \"0 2 * * *\"\nmachine: nope\n---\n# Unknown machine\n",
            ),
        ],
        scripts: &[],
        expected: "\
machines/m/cron/a.md: line 1: no `cron:` expression (M3)
machines/m/cron/b.md: line 3: unknown machine `nope` (M3)
",
    },
    Case {
        rule: "M3",
        name: "no machine key runs the folder's machine",
        files: &[
            ("machines/m/machine.yml", BASE),
            (
                "machines/m/cron/c.md",
                "---\ncron: \"0 3 * * *\"\n---\n# No machine key\n",
            ),
        ],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "M3",
        name: "a param outside the enum",
        files: &[
            ("machines/m/machine.yml", CHOICES),
            (
                "machines/m/cron/a.md",
                "---\ncron: '0 2 * * *'\nmachine: m\nparams: { need: review }\n---\n# Nightly\n",
            ),
        ],
        scripts: &[],
        expected: "machines/m/cron/a.md: line 4: param `need` is `review`, not one of the allowed values: plan, fix (M3)\n",
    },
    // E1
    Case {
        rule: "E1",
        name: "a dotenv file with comments, quotes and export",
        files: &[
            ("machines/m/machine.yml", BASE),
            (
                ".env",
                "# GPU box\n\nexport COMFY_URL=\"http://box:8188\"\nMODEL='sdxl'\nSTEPS=30\n",
            ),
        ],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "E1",
        name: "a reserved key, a line that is not a pair, a bad key, an unclosed quote",
        files: &[
            ("machines/m/machine.yml", BASE),
            (
                ".env",
                "DECREE_X=1\nnot a pair\nTRACEPARENT=x\n1A=x\nB=\"open\n",
            ),
        ],
        scripts: &[],
        expected: "\
.env: line 1: key `DECREE_X` is reserved: `DECREE_*`, `TRACEPARENT` and `TRACESTATE` belong to decree (E1)
.env: line 2: `not a pair` is not `KEY=value` (E1)
.env: line 3: key `TRACEPARENT` is reserved: `DECREE_*`, `TRACEPARENT` and `TRACESTATE` belong to decree (E1)
.env: line 4: key `1A` does not match `^[A-Za-z_][A-Za-z0-9_]*$` (E1)
.env: line 5: the value of `B` opens a \" quote it does not close (E1)
",
    },
    // P1
    Case {
        rule: "P1",
        name: "a plugin with its script",
        files: &[("plugins/r.md", PLUGIN_R)],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "P1",
        name: "a script path that leaves the folder",
        files: &[(
            "plugins/m.md",
            "---\nname: m\nmachine: { description: d }\nscripts:\n  ../x.sh: hi\n---\n",
        )],
        scripts: &[],
        expected: "plugins/m.md: line 4: script path `../x.sh` is not a plain relative path: no `..`, `.`, empty parts or leading `/` (P1)\n",
    },
    Case {
        rule: "P1",
        name: "name differs from the file stem",
        files: &[("plugins/n.md", PLUGIN_BASE)],
        scripts: &[],
        expected: "plugins/n.md: line 2: `name` is `m`, but the file is `n.md`: they must be equal (P1)\n",
    },
    // P2
    Case {
        rule: "P2",
        name: "needs a plugin that is there",
        files: &[
            ("plugins/r.md", PLUGIN_R),
            (
                "plugins/m.md",
                "---\nname: m\nneeds: [r]\nmachine:\n  description: Base machine.\n  initial: work\n  states:\n    work:\n      invoke: { machine: { name: r } }\n      transitions: { done: done }\n    done: { final: true }\n    failed: { final: true }\n---\n",
            ),
        ],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "P2",
        name: "needs a plugin that is not there",
        files: &[(
            "plugins/m.md",
            "---\nname: m\nneeds: [r]\nmachine:\n  description: Base machine.\n  initial: work\n  states:\n    work:\n      invoke: work\n      transitions: { done: done }\n    done: { final: true }\n    failed: { final: true }\nscripts:\n  work: \"#!/bin/sh\\n\"\n---\n",
        )],
        scripts: &[],
        expected: "plugins/m.md: `needs` names `r`, not in `plugins/`: import it with `decree plugin import` (P2)\n",
    },
    // P3
    Case {
        rule: "P3",
        name: "a changed plugin over an unedited copy is no conflict",
        files: &[
            ("cache/plugins/m.md", PLUGIN_BASE),
            ("machines/m/machine.yml", BASE),
            ("plugins/m.md", PLUGIN_CHANGED),
        ],
        scripts: &[],
        expected: PASSES,
    },
    Case {
        rule: "P3",
        name: "a changed plugin over an edited copy",
        files: &[
            ("cache/plugins/m.md", PLUGIN_BASE),
            ("machines/m/machine.yml", BASE_EDITED),
            ("plugins/m.md", PLUGIN_CHANGED),
        ],
        scripts: &[],
        expected: "plugins/m.md: plugin `m` changed, but its installed copy (machines/m/machine.yml, or a file in machines/m/scripts/ or machines/m/cron/) was edited in place, so it was not installed and the edited copy keeps running. To keep the edit, run `decree plugin export m` and merge with git; to take the plugin's version, run `decree plugin install m` (P3)\n",
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

/// `choose: person`, which `reply:` replaced (V19).
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

/// `person:` with `ask:`, which `reply:` with `notify:` replaced (V19).
const OLD_PERSON: &str = "\
name: m
description: Wait for a reply, written the old way.
initial: approval
states:
  approval:
    invoke:
      person:
        question: Ship it?
        ask: ask
        timeout: 1h
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

/// `timeout_s` inside a script invoke, which `timeout: <duration>` replaced (V19).
const OLD_TIMEOUT_S_SCRIPT: &str = "\
name: a
description: A time limit in seconds, written the old way.
initial: work
states:
  work:
    invoke: { script: { name: work, timeout_s: 60 } }
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
";

/// `timeout_s` inside a reply invoke, which `timeout: <duration>` replaced (V19).
const OLD_TIMEOUT_S_REPLY: &str = "\
name: d
description: Ask a person, for a week at most.
initial: approval
states:
  approval:
    invoke:
      reply:
        question: Ship this build?
        notify: ask_person
        timeout_s: 604800
    transitions:
      approve: { target: done, description: Ship this build. }
      reject: { target: done, description: Do not ship. }
  done: { final: true }
  failed: { final: true }
";

/// A reply invoke with a `timeout` in days (V16).
const TIMEOUT_REPLY: &str = "\
name: d
description: Ask a person, for a week at most.
initial: approval
states:
  approval:
    invoke:
      reply:
        question: Ship this build?
        notify: ask_person
        timeout: 7d
    transitions:
      approve: { target: done, description: Ship this build. }
      reject: { target: done, description: Do not ship. }
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

/// A `router:` on a state inside a compound state (V19): only a `model` invoke takes one.
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

/// The script names in a machine (V12): a string `invoke`, `notify`, and `onentry`/`onexit`.
fn script_names(value: &Value, names: &mut Vec<String>) {
    match value {
        Value::Mapping(map) => {
            for (key, value) in map {
                match (key.as_str(), value) {
                    (Some("invoke" | "script" | "notify"), Value::String(name)) => {
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

/// Where script `path` of a case goes: `<machine>/<file>` in that machine's
/// `machines/<machine>/scripts/`; a bare `<file>` in every machine's.
fn script_paths(decree: &Path, machines: &[String], path: &str) -> Vec<PathBuf> {
    match path.split_once('/') {
        Some((m, file)) => vec![decree.join("machines").join(m).join("scripts").join(file)],
        None => machines
            .iter()
            .map(|m| decree.join("machines").join(m).join("scripts").join(path))
            .collect(),
    }
}

/// Write `files` and the scripts the machines need into `decree`.
fn write_project(decree: &Path, files: &[(&str, &str)], scripts: &[Script]) {
    let mut machines = Vec::new();
    let mut needed = Vec::new();
    for (path, text) in files {
        write(&decree.join(path), text, 0o644);
        let machine = path
            .strip_prefix("machines/")
            .and_then(|p| p.strip_suffix("/machine.yml"));
        if let Some(machine) = machine {
            machines.push(machine.to_string());
            let mut names = Vec::new();
            if let Ok(value) = serde_norway::from_str::<Value>(text) {
                script_names(&value, &mut names);
            }
            needed.extend(names.into_iter().map(|n| (machine.to_string(), n)));
        }
    }
    for script in scripts {
        let (path, mode) = match script {
            Script::Missing(_) => continue,
            Script::At(path) => (path, 0o755),
            Script::NotExecutable(path) => (path, 0o644),
        };
        for at in script_paths(decree, &machines, path) {
            write(&at, STUB, mode);
        }
    }
    for (machine, name) in needed {
        if scripts.iter().all(|s| s.name() != name) {
            write(
                &decree
                    .join("machines")
                    .join(&machine)
                    .join("scripts")
                    .join(&name),
                STUB,
                0o755,
            );
        }
    }
}

/// `decree check` in a temp project holding `files`: (exit code, stdout).
fn check(files: &[(&str, &str)], scripts: &[Script]) -> (i32, String) {
    let tmp = TempDir::new().unwrap();
    write_project(&tmp.path().join(".decree"), files, scripts);
    let out = common::decree_at(tmp.path())
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

const RULES: [&str; 28] = [
    "V1", "V2", "V3", "V4", "V5", "V6", "V7", "V8", "V9", "V10", "V11", "V12", "V13", "V14", "V15",
    "V16", "V17", "V18", "V19", "V20", "V21", "M1", "M2", "M3", "E1", "P1", "P2", "P3",
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
    p1_plugin_schema => "P1",
    p2_plugin_needs => "P2",
    p3_plugin_edit_conflict => "P3",
}

/// Files of failing cases that the schemas accept: what is wrong is beyond what a JSON Schema
/// of one file can say, so only `decree check` catches it. (rule, case name, file, reason).
const CHECK_ONLY: &[(&str, &str, &str, &str)] = &[
    (
        "P1",
        "name differs from the file stem",
        "plugins/n.md",
        "a schema sees the document, not its file name",
    ),
    (
        "P2",
        "needs a plugin that is not there",
        "plugins/m.md",
        "which plugins exist depends on other files",
    ),
    (
        "P3",
        "a changed plugin over an edited copy",
        "plugins/m.md",
        "an edit conflict depends on the installed copy and the record of its install",
    ),
    (
        "V1",
        "name differs from the file stem",
        "machines/m/machine.yml",
        "a schema sees the document, not its file name",
    ),
    (
        "V3",
        "initial names no child",
        "machines/m/machine.yml",
        "a schema cannot require a value to be one of the document's own keys",
    ),
    (
        "V4",
        "a target that does not exist",
        "machines/m/machine.yml",
        "a target may be any state in the machine, at any depth: a cross-reference",
    ),
    (
        "V9",
        "output names a state that runs no script",
        "machines/m/machine.yml",
        "whether the named state has a script invoke is a cross-reference",
    ),
    (
        "V10",
        "a condition on unknown data",
        "machines/m/machine.yml",
        "whether `data` names an entry of the machine's `data`, and its type, is a cross-reference",
    ),
    (
        "V11",
        "an unreachable state and a stall",
        "machines/m/machine.yml",
        "reachability is a property of the whole graph",
    ),
    (
        "V12",
        "a missing script and one that is not executable",
        "machines/m/machine.yml",
        "scripts are files on disk, outside the document",
    ),
    (
        "V13",
        "emits names an unknown machine",
        "machines/m/machine.yml",
        "other machines are other files",
    ),
    (
        "V15",
        "nothing handles done.state",
        "machines/m/machine.yml",
        "the event is named after the state's own key, done.state.<id>, and may be handled by an ancestor",
    ),
    (
        "V20",
        "two machines that invoke each other",
        "machines/m/machine.yml",
        "a cycle runs through other machines, which are other files",
    ),
    (
        "V20",
        "two machines that invoke each other",
        "machines/n/machine.yml",
        "a cycle runs through other machines, which are other files",
    ),
    (
        "V21",
        "done and done.state overlap",
        "machines/m/machine.yml",
        "a schema cannot compare one key of a map with the others",
    ),
    (
        "M1",
        "an unknown param, an unknown machine, no machine key",
        "migrations/01-first.md",
        "valid params depend on the machine's `data`, in another file",
    ),
    (
        "M1",
        "an unknown param, an unknown machine, no machine key",
        "migrations/02-second.md",
        "which machines exist depends on other files",
    ),
    (
        "M2",
        "no machine, a param of the wrong type, no closing fence, a duplicate key",
        "inbox/b.md",
        "a param's type depends on the machine's `data`, in another file",
    ),
    (
        "M2",
        "no machine, a param of the wrong type, no closing fence, a duplicate key",
        "inbox/c.md",
        "with no closing `---` there is no frontmatter to validate",
    ),
    (
        "M2",
        "no machine, a param of the wrong type, no closing fence, a duplicate key",
        "inbox/d.md",
        "a duplicate key is a YAML error, before any schema; YAML parsers and editors report it",
    ),
    (
        "M3",
        "no cron key, an unknown machine",
        "machines/m/cron/a.md",
        "one message schema covers every message; only the directory makes a file a cron file",
    ),
    (
        "M3",
        "no cron key, an unknown machine",
        "machines/m/cron/b.md",
        "which machines exist depends on other files",
    ),
    (
        "E1",
        "a reserved key, a line that is not a pair, a bad key, an unclosed quote",
        ".env",
        "`.decree/.env` is a dotenv file, not YAML or JSON; no schema covers it",
    ),
    (
        "M1",
        "a param outside the enum, a number param that is not a number",
        "migrations/01-first.md",
        "valid params depend on the machine's `data`, in another file",
    ),
    (
        "M1",
        "a param outside the enum, a number param that is not a number",
        "migrations/02-second.md",
        "valid params depend on the machine's `data`, in another file",
    ),
    (
        "M2",
        "a param outside the enum",
        "inbox/a.md",
        "valid params depend on the machine's `data`, in another file",
    ),
    (
        "M3",
        "a param outside the enum",
        "machines/m/cron/a.md",
        "valid params depend on the machine's `data`, in another file",
    ),
];

/// The schema errors of one case file: `Some(errors)` for a machine or a message (empty when
/// it validates), `None` when the file is neither or does not parse.
fn schema_errors(
    path: &str,
    text: &str,
    machine: &jsonschema::Validator,
    message: &jsonschema::Validator,
) -> Option<Vec<String>> {
    let file = path.rsplit('/').next().unwrap();
    if file.starts_with('.') || !file.ends_with(".yml") && !file.ends_with(".md") {
        return None;
    }
    // A cron file, `machines/<m>/cron/<file>.md`, is a message whose `machine` defaults to
    // the folder's, as decree reads it.
    let parts: Vec<&str> = path.split('/').collect();
    if let ["machines", owner, "cron", _] = parts.as_slice() {
        let text = match text.strip_prefix("---\n") {
            Some(rest)
                if !rest
                    .split("\n---")
                    .next()
                    .unwrap_or("")
                    .contains("machine:") =>
            {
                format!("---\nmachine: {owner}\n{rest}")
            }
            _ => text.to_string(),
        };
        return schema::message_errors(message, &text);
    }
    match path.split('/').next() {
        Some("machines") => schema::machine_errors(machine, text),
        Some("migrations" | "inbox") => schema::message_errors(message, text),
        Some("plugins") => schema::frontmatter_to_json(text)
            .map(|json| schema::errors(&schema::plugin_validator(), &json)),
        _ => None,
    }
}

/// The schemas never reject what `decree check` accepts, and reject every file a failing case
/// reports, except the ones in `CHECK_ONLY`, which they accept.
#[test]
fn schemas_agree_with_decree_check_on_every_case() {
    let (machine, message) = (schema::machine_validator(), schema::message_validator());
    let mut wrong = Vec::new();
    let mut listed = 0;
    for case in CASES {
        let label = format!("{}: {}", case.rule, case.name);
        for (path, text) in case.files {
            let errors = schema_errors(path, text, &machine, &message);
            let reported = case
                .expected
                .lines()
                .any(|l| l.starts_with(&format!("{path}: ")));
            let check_only = CHECK_ONLY
                .iter()
                .any(|(r, n, f, _)| (*r, *n, *f) == (case.rule, case.name, *path));
            listed += usize::from(check_only);
            match (reported, check_only, errors) {
                // Not reported (or a passing case): the schema accepts it too.
                (false, false, Some(errors)) if !errors.is_empty() => {
                    wrong.push(format!("{label}: {path}: valid, but the schema rejects it: {errors:?}"))
                }
                (false, true, _) => wrong.push(format!("{label}: {path}: in CHECK_ONLY, but decree check does not report it")),
                // Reported, and only decree check can tell: the schema accepts it, or cannot read it.
                (true, true, Some(errors)) if !errors.is_empty() => wrong.push(format!(
                    "{label}: {path}: the schema now rejects it; remove it from CHECK_ONLY: {errors:?}"
                )),
                // Reported: the schema rejects it.
                (true, false, None) => {
                    wrong.push(format!("{label}: {path}: not YAML; list it in CHECK_ONLY"))
                }
                (true, false, Some(errors)) if errors.is_empty() => wrong.push(format!(
                    "{label}: {path}: the schema accepts it; reject it, or list it in CHECK_ONLY"
                )),
                _ => {}
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert_eq!(
        listed,
        CHECK_ONLY.len(),
        "a CHECK_ONLY entry names no case file"
    );
}

/// Four machines, each wrong in its own way: every error is its own line.
#[test]
fn several_invalid_machines_print_one_line_per_error() {
    let files = [
        (
            "machines/a/machine.yml",
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
            "machines/b/machine.yml",
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
            "machines/c/machine.yml",
            "name: c\ndescription: d\ninitial: x\nstates:\n  x: { invok: work }\n",
        ),
        (
            "machines/d/machine.yml",
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
machines/a/machine.yml: work: transition `done` targets unknown state `nowhere` (V4)
machines/a/machine.yml: done: state is unreachable from the root `initial` (V11)
machines/b/machine.yml: line 4: no root-level final state `failed`: every machine needs one for unhandled errors (V5)
machines/c/machine.yml: x: unknown field `invok`, expected one of `final`, `description`, `invoke`, `onentry`, `onexit`, `initial`, `states`, `transitions`, `emits` (V19)
machines/d/machine.yml: again: a `check` state must handle `true`, itself or through an ancestor (V8)
machines/d/machine.yml: again: a `check` state must handle `false`, itself or through an ancestor (V8)
"
    );
}

/// `decree check --format <format>` in a temp project holding `files`: (exit code, stdout).
fn check_as(files: &[(&str, &str)], scripts: &[Script], format: &str) -> (i32, String) {
    let tmp = TempDir::new().unwrap();
    write_project(&tmp.path().join(".decree"), files, scripts);
    let out = common::decree_at(tmp.path())
        .env("NO_COLOR", "1")
        .args(["check", "--format", format])
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
    )
}

/// A text error line in parts: (file, line, rule).
fn text_parts(line: &str) -> (&str, Option<u64>, Option<&str>) {
    let (file, rest) = line.split_once(": ").unwrap();
    let number = rest
        .strip_prefix("line ")
        .and_then(|r| r.split_once(": "))
        .and_then(|(n, _)| n.parse().ok());
    let rule = line
        .strip_suffix(')')
        .and_then(|l| l.rsplit_once(" ("))
        .map(|(_, rule)| rule);
    (file, number, rule)
}

/// `decree check --format json` on every case: the document validates against
/// `cli/check.schema.json`, lists the same errors as the text, line for line, and the exit
/// code is the text's.
#[test]
fn check_json_lists_the_same_errors_as_text_for_every_case() {
    let validator = schema::validator(schema::CLI_CHECK_SCHEMA);
    for case in CASES {
        let label = format!("{}: {}", case.rule, case.name);
        let (code, stdout) = check_as(case.files, case.scripts, "json");
        let (text_code, text) = check(case.files, case.scripts);
        assert_eq!(code, text_code, "{label}");
        let doc: serde_json::Value = serde_json::from_str(&stdout).expect(&label);
        let errors = schema::errors(&validator, &doc);
        assert!(errors.is_empty(), "{label}: {errors:?}");
        assert_eq!(doc["valid"], case.expected == PASSES, "{label}");
        let lines: String = doc["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                let mut line = format!("{}: ", e["file"].as_str().unwrap());
                if let Some(n) = e["line"].as_u64() {
                    line.push_str(&format!("line {n}: "));
                }
                if let Some(state) = e["state"].as_str() {
                    line.push_str(&format!("{state}: "));
                }
                line.push_str(e["message"].as_str().unwrap());
                if let Some(rule) = e["rule"].as_str() {
                    line.push_str(&format!(" ({rule})"));
                }
                line + "\n"
            })
            .collect();
        assert_eq!(lines, text, "{label}");
    }
}

/// `decree check --format sarif` on every failing case: a SARIF 2.1.0 log whose driver is
/// decree with every rule of the Validation table, and one `error` result per text error
/// with its rule as `ruleId`, its file as `.decree/<file>` and its line as `startLine`.
/// Warnings (here: no `.decree/graph/` or `.decree/schema/`) are results with no rule.
#[test]
fn check_sarif_has_every_rule_and_a_result_per_text_error() {
    for case in CASES.iter().filter(|c| c.expected != PASSES) {
        let label = format!("{}: {}", case.rule, case.name);
        let (code, stdout) = check_as(case.files, case.scripts, "sarif");
        assert_eq!(code, 1, "{label}");
        let log: serde_json::Value = serde_json::from_str(&stdout).expect(&label);
        assert_eq!(log["version"], "2.1.0");
        assert_eq!(
            log["$schema"],
            "https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json"
        );
        let runs = log["runs"].as_array().unwrap();
        assert_eq!(runs.len(), 1);
        let driver = &runs[0]["tool"]["driver"];
        assert_eq!(driver["name"], "decree");
        assert_eq!(driver["version"], env!("CARGO_PKG_VERSION"));
        assert!(driver["informationUri"]
            .as_str()
            .unwrap()
            .ends_with("docs/reference/README.md"));
        let rules = driver["rules"].as_array().unwrap();
        let ids: Vec<&str> = rules.iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert_eq!(ids, RULES);
        for rule in rules {
            assert!(!rule["shortDescription"]["text"]
                .as_str()
                .unwrap()
                .is_empty());
            assert!(rule["helpUri"]
                .as_str()
                .unwrap()
                .ends_with("docs/reference/machines.md#validation"));
        }

        let results = runs[0]["results"].as_array().unwrap();
        let (errors, warnings): (Vec<_>, Vec<_>) =
            results.iter().partition(|r| r["level"] == "error");
        for w in &warnings {
            assert_eq!(w["level"], "warning", "{label}");
            assert!(w.get("ruleId").is_none(), "{label}");
        }
        let (_, text) = check(case.files, case.scripts);
        assert_eq!(errors.len(), text.lines().count(), "{label}");
        for (result, line) in errors.iter().zip(text.lines()) {
            let (file, number, rule) = text_parts(line);
            assert_eq!(result["ruleId"].as_str(), rule, "{label}: {line}");
            assert!(!result["message"]["text"].as_str().unwrap().is_empty());
            let location = &result["locations"][0]["physicalLocation"];
            assert_eq!(
                location["artifactLocation"]["uri"],
                format!(".decree/{file}"),
                "{label}: {line}"
            );
            assert_eq!(
                location["region"]["startLine"].as_u64(),
                number,
                "{label}: {line}"
            );
        }
    }
}

/// An error no rule names, a machine that is not YAML: `rule` is null in JSON, and the
/// SARIF result has no `ruleId`.
#[test]
fn an_error_without_a_rule_has_a_null_rule_and_no_rule_id() {
    let files = [("machines/m/machine.yml", "name: m\nstates: [\n")];
    let (code, stdout) = check_as(&files, &[], "json");
    assert_eq!(code, 1);
    let doc: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let error = &doc["errors"][0];
    assert_eq!(error["rule"], serde_json::Value::Null);
    assert_eq!(error["file"], "machines/m/machine.yml");
    assert!(error["line"].is_u64(), "{error}");
    let errors = schema::errors(&schema::validator(schema::CLI_CHECK_SCHEMA), &doc);
    assert!(errors.is_empty(), "{errors:?}");

    let (code, stdout) = check_as(&files, &[], "sarif");
    assert_eq!(code, 1);
    let log: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let result = &log["runs"][0]["results"][0];
    assert_eq!(result["level"], "error");
    assert!(result.get("ruleId").is_none(), "{result}");
}
