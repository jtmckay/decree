# Decision log

Why decree 0.5 works the way it does. Each entry is an Architecture Decision Record in Michael Nygard's form: the **context** (the forces at play), the **decision**, and its **consequences**. Entries are not edited when a later one replaces them; the later entry says what it supersedes.

What decree does is in the [reference](reference/README.md). The construction plan for the 0.5 rewrite (the implementation spec, the 0.4.2 inventory and two spikes) has done its job and was removed; its last version is in `docs/` at commit `32223bb`. Evidence below is linked by commit hash.

| # | Decision | Source |
| --- | --- | --- |
| [D1](#d1-machines-follow-scxml) | Machines follow SCXML | 0.4 to 0.5 design |
| [D2](#d2-yaml-is-the-only-syntax) | YAML is the only syntax | 0.4 to 0.5 design |
| [D3](#d3-no-scxml-library) | No SCXML library | 0.4 to 0.5 design |
| [D4](#d4-a-router-is-a-machine) | A router is a machine | Router spike R1 |
| [D5](#d5-each-choose-model-names-its-router-or-uses-the-machine-named-router) | Each `choose: model` names its router, or uses the machine named `router` | Router spike R2 |
| [D6](#d6-the-prompt-lives-in-the-routers-script) | The prompt lives in the router's script | Router spike R3 |
| [D7](#d7-min_confidence-turns-a-weak-pick-into-unsure) | `min_confidence` turns a weak pick into `unsure` | Router spike R4 |
| [D8](#d8-asking-a-person-is-choose-person-plus-a-reply-message) | Asking a person is `choose: person` plus a reply message | Router spike R5 |
| [D9](#d9-what-the-router-request-holds) | What the router request holds | Router spike R6 |
| [D10](#d10-self-hosting-is-a-router-machine) | Self-hosting is a router machine | Router spike R7 |
| [D11](#d11-every-decision-is-auditable) | Every decision is auditable | Router spike R8 |
| [D12](#d12-ai-and-people-decide-only-where-a-machine-says-choose) | AI and people decide only where a machine says `choose` | Router spike R9 |
| [D13](#d13-no-detection-of-ai-inside-scripts) | No detection of AI inside scripts | Router spike R10 |
| [D14](#d14-the-graph-is-mermaid-text-in-markdown) | The graph is Mermaid text in Markdown | Graph spike |
| [D15](#d15-onentry-and-onexit-run-once-per-visit) | `onentry` and `onexit` run once per visit | 0.4 to 0.5 design |
| [D16](#d16-delete-with-the-last-caller) | Delete with the last caller | 0.4 to 0.5 design |
| [D17](#d17-no-configuration-file) | No configuration file | 0.4 to 0.5 design |
| [D18](#d18-c1-clippy-fixes-in-test-files) | C1: clippy fixes in test files | Inventory |
| [D19](#d19-c2-sigterm-stops-decree-and-its-script) | C2: SIGTERM stops decree and its script | Inventory |
| [D20](#d20-c3-sigkill-after-a-10-s-grace) | C3: SIGKILL after a 10 s grace | Inventory |
| [D21](#d21-c4-one-executor-for-every-script) | C4: one executor for every script | Inventory |
| [D22](#d22-c5-the-inbox-is-first-in-first-out) | C5: the inbox is first-in, first-out | Inventory |
| [D23](#d23-c6-the-daemon-runs-the-same-pipeline-as-process) | C6: the daemon runs the same pipeline as `process` | Inventory |
| [D24](#d24-c7-deletions-are-checked-with-qualified-names) | C7: deletions are checked with qualified names | Inventory |
| [D25](#d25-c8-no-max_retries-alias) | C8: no `max_retries` alias | Inventory |
| [D26](#d26-c9-init-never-touches-an-existing-decree) | C9: `init` never touches an existing `.decree/` | Inventory |
| [D27](#d27-q1-the-processedmd-ledger) | Q1: the `processed.md` ledger | Open questions |
| [D28](#d28-q2-no-commands-configuration) | Q2: no commands configuration | Open questions |
| [D29](#d29-q3-every-04-hook-maps-to-onentry-or-onexit) | Q3: every 0.4 hook maps to `onentry` or `onexit` | Open questions |
| [D30](#d30-q4-init-writes-the-decree-skill) | Q4: `init` writes the decree skill | Open questions |
| [D31](#d31-q5-one-run-at-a-time-per-process) | Q5: one run at a time per process | Open questions |
| [D32](#d32-q6-routers-are-commands-not-http-clients) | Q6: routers are commands, not HTTP clients | Open questions |
| [D33](#d33-q7-no-decree-replay-yet) | Q7: no `decree replay` yet | Open questions |
| [D34](#d34-q8-emitted-messages-run-first-in-first-out) | Q8: emitted messages run first-in, first-out | Open questions |
| [D35](#d35-q9-free-form-messages-are-routed-by-a-machine) | Q9: free-form messages are routed by a machine | Open questions |
| [D36](#d36-q10-a-killed-run-waits-for-decree-retry) | Q10: a killed run waits for `decree retry` | Open questions |
| [D37](#d37-q11-one-flat-scripts-directory-with-per-machine-overrides) | Q11: one flat `scripts/` directory with per-machine overrides | Open questions |
| [D38](#d38-q12-the-router-contract-is-requestjson-and-replyjson) | Q12: the router contract is `request.json` and `reply.json` | Open questions |
| [D39](#d39-q13-graphs-are-viewed-in-existing-tools) | Q13: graphs are viewed in existing tools | Open questions |
| [D40](#d40-q14-a-person-answers-with-a-reply-message) | Q14: a person answers with a reply message | Open questions |
| [D41](#d41-q15-a-machine-can-invoke-another-machine) | Q15: a machine can invoke another machine | Open questions |
| [D42](#d42-q16-scripts-do-not-run-inside-tmux) | Q16: scripts do not run inside tmux | Open questions |
| [D43](#d43-delete-finished-runs-on-request-never-automatically) | Delete finished runs on request, never automatically | Migration 70 |
| [D44](#d44-one-shape-per-machine-key) | One shape per machine key | Migration 71 |

## D1: Machines follow SCXML

**Context.** decree 0.4 routed each message with a global LLM prompt, and a routine was a bash script with hooks around it. Control flow was hidden in prompts and scripts, could not be validated, and could not be drawn. A workflow engine needs states, nesting, entry and exit actions and well-defined transition semantics; inventing them invites subtle bugs and forces every user, human or AI, to learn a new model.

**Decision.** Machines follow W3C SCXML 1.0 (Recommendation, 2015): its terms, its key names, its transition semantics (Appendix D), for a strict subset of its features. Every difference is listed in [SCXML subset](reference/machines.md#scxml-subset); extensions are marked as such in [Keys](reference/machines.md#keys).

**Consequences.** Anyone who knows SCXML or Harel statecharts knows how a machine behaves. Features outside the subset (`<parallel>`, `<history>`, `cond`, executable content) fail validation with the SCXML name and the decree alternative. Where SCXML leaves a choice to the platform, the reference records decree's. Evidence: `a0079ea` (first plan), `691ae4c` (SCXML conformance fixes queued).

## D2: YAML is the only syntax

**Context.** SCXML is defined in XML. decree's users write machines by hand and with AI agents next to markdown messages and bash scripts; XML is verbose there, and two syntaxes would double the parser, the tests and the docs.

**Decision.** Machines are YAML 1.2 documents, parsed with `serde_norway`. There is no XML input or output.

**Consequences.** Machines are short and readable. YAML 1.1 tools read bare `yes`, `no` and `on` as booleans; decree reads them as strings, which matters because `yes` and `no` are `check` events. Inline maps need quoted questions and descriptions when they contain `, ` or `?`. Evidence: `a0079ea`; the `serde_yaml` to `serde_norway` switch in `759fa59`.

## D3: No SCXML library

**Context.** The Rust SCXML crates either only parse XML (`harel`) or simulate a chart without exposing the exit and entry sets (`scxml`). decree needs exactly those sets, to run `onentry` and `onexit` scripts in order and record each step, and its subset of the Appendix D algorithm is small.

**Decision.** decree implements its subset of the algorithm itself, on a flattened arena of the machine that the interpreter, the validator and the graph exporter share.

**Consequences.** No dependency to track, and the picture (`decree graph`) can never drift from the behaviour. The conformance burden is decree's own; the SCXML fixtures in `tests/fixtures/scxml/` carry it. Evidence: `a0079ea`, `759fa59`.

## D4: A router is a machine

**Context.** A `choose: model` decision has to ask some model. The spike first decided that a router is a command named in `config.yml`, which reads a rendered prompt or a JSON request on stdin and prints a reply. That kept decree free of HTTP clients, but put prompts, retries and budgets in configuration that could not be seen, validated or drawn like the rest of the workflow.

**Decision.** A router is an ordinary machine. decree writes `request.json` in a child run's folder, runs the router machine as that child run, and reads and validates `reply.json` ([Model](reference/runs.md#model)). This supersedes the spike's command design (its R1, R2, R3, R6 and R7 as first recorded) and the `routers:` and `default_router` keys of `config.yml`.

**Consequences.** Routers are replaceable and visible like any machine: their own runs, logs, events and graphs. Any backend and any language fit. decree contains no model code. A router can itself escalate between models. Evidence: `759fa59` (questions table changed to "a router is a machine"), `8074a50` and `aa4139b` (implemented), `docs/spikes/router.md` at `32223bb` (both versions of the record).

## D5: Each `choose: model` names its router, or uses the machine named `router`

**Context.** Different decisions suit different backends: a local classifier for document types, a large model for code review. The spike first chose named `routers:` plus `default_router` in `config.yml`; [D17](#d17-no-configuration-file) later removed the configuration file.

**Decision.** `choose: model, router: <machine>` picks a router. Without `router:`, the machine named `router` is used, and `decree init` writes it. `decree check` fails V16 if a `choose: model` names no router and there is no `machines/router.yml`.

**Consequences.** The default is a naming convention, not a setting, so it is visible in `machines/`. Evidence: `759fa59` (router per state), `235d7e5` and `0652778` (`default_router` replaced by the machine named `router`).

## D6: The prompt lives in the router's script

**Context.** The spike's first record had decree render one chat prompt for every chat backend. With routers as machines ([D4](#d4-a-router-is-a-machine)), a prompt inside decree would bind every router to one shape, and typed-choice models take no prompt at all.

**Decision.** decree writes a structured request. The default router's script renders the prompt from it ([The default router](reference/runs.md#the-default-router)), and the reference documents that prompt.

**Consequences.** Changing a prompt is a script edit, never a decree release. Typed-choice routers map the request onto their own API. Evidence: `759fa59`, `aa4139b` (the 0.4.2 router call moved into `ask_claude`).

## D7: `min_confidence` turns a weak pick into `unsure`

**Context.** Typed-choice models such as TypeSafe Jev report a confidence, and acting on a guess is worse than asking. The options were to record confidence only, to fall back to a default event, or to produce a declared event.

**Decision.** `min_confidence` on `choose: model`: below it, or with no confidence reported, the state's event is `unsure`, an ordinary event the machine must handle (V8). Escalating to a person is a transition from `unsure` to a `choose: person` state. There are no bands in the invoke; a `confidence` check after `unsure` splits further when wanted.

**Consequences.** The threshold sits where the decision is made, and escalation shows in the machine and its graph. Confidence is the router's own number, so a threshold is calibrated per router ([Model](reference/runs.md#model)). Evidence: `759fa59`, `aa4139b`.

## D8: Asking a person is `choose: person` plus a reply message

**Context.** The spike first leaned to a final state such as `needs_review` that a human continues with `decree retry --state`. That needs someone to know decree's commands, and leaves no record of the answer.

**Decision.** `choose: person`: the state's `ask` script tells someone, the run pauses with a wait id, and a reply message (`to:` and `event:`) delivers one option ([Replies](reference/messages.md#replies)). This is SCXML's external event, and the AWS Step Functions callback pattern with the wait id as the task token.

**Consequences.** decree never knows there is a question or who answers; any tool that can write a file can reply. Wrong replies become failed `invalid_message` runs and leave the waiting run unchanged. Evidence: `759fa59` (design), `8074a50` (the invoke), `8f9634b` (replies).

## D9: What the router request holds

**Context.** A router needs enough to decide, and no more: secrets must stay out of prompts, and the input must be explainable. The first record had decree cut the input and body to a router's `max_input_bytes`.

**Decision.** The request holds the machine and state with their descriptions, the question, the options with descriptions, `min_confidence`, the input state's output, the message body and the run's history ([Model](reference/runs.md#model)). A script chooses what the model sees by what it prints. Trimming to a model's budget is the router's job.

**Consequences.** Secrets stay out by design, not by redaction. `input` and `message_body` stay separate, so a router can pass structured context (Jev's `state` takes any JSON). The request is versioned (`v: 1`), so fields such as attachments can be added without breaking routers. Evidence: `759fa59`, `8074a50`.

## D10: Self-hosting is a router machine

**Context.** Users want local models: SGLang, vLLM, Ollama, or a small classifier such as GLiNER2.5-Decide that runs on CPU. Running model servers is a large problem of its own.

**Decision.** A self-hosted backend is a router machine whose script talks to a server the user runs. [Router machines](routers.md) shows TypeSafe Jev, GLiNER2.5-Decide behind a small long-running server, a self-hosted LLM and a cheap-model-first router. Running a model server is outside decree ([Long-running services](services.md)).

**Consequences.** decree stays small. A model loads once in its own service, not per decision. Evidence: `759fa59`, `aa4139b`.

## D11: Every decision is auditable

**Context.** Model decisions must be checkable after the fact: for audit, for calibrating thresholds, and for a future replay.

**Decision.** Each router call is its own child run with its logs, `request.json` and `reply.json`. Every `check` and `choose` produces a `decision` event; for a model it records the router, the child run, the pick (even when the event is `unsure`), the reason, the confidence and the probabilities ([events.jsonl](reference/runs.md#eventsjsonl)).

**Consequences.** Thresholds can be checked against outcomes in Grafana. A replay can feed recorded picks back ([D33](#d33-q7-no-decree-replay-yet)). Evidence: `759fa59`, `aa4139b`.

## D12: AI and people decide only where a machine says `choose`

**Context.** In 0.4 it was hard to tell where an AI made a decision. The spike considered expression strings and extra model primitives (Jev's Score and yes/no) for conditions.

**Decision.** AI and people appear only in `choose: model` and `choose: person` states; deterministic decisions are `check` states with typed conditions, not expression strings. Graph edges say who decided: `(check)`, `(model)`, `(person)`.

**Consequences.** It is always clear when an AI is involved. Typed conditions keep YAML readable and let `decree check` catch mistakes (V10). Model-backed conditions are not part of decree. Evidence: `759fa59`, `8074a50`.

## D13: No detection of AI inside scripts

**Context.** A script may call a model itself, as the `develop` scripts do. decree could try to detect that.

**Decision.** Not pursued.

**Consequences.** What a script does internally is the script's business. Only decisions that pick an edge are decree's, and those are `choose` states ([D12](#d12-ai-and-people-decide-only-where-a-machine-says-choose)). Evidence: `759fa59`.

## D14: The graph is Mermaid text in Markdown

**Context.** `decree graph` prints Mermaid. That renders on GitHub, GitLab, VS Code and Obsidian, but a user in a terminal sees text. The spike weighed shelling out to a renderer (mermaid-cli, merman-cli), building merman into decree, a self-contained HTML page, a mermaid.live link, DOT, an XState export and terminal rendering, against four criteria: show nesting, notes and who decides each edge; work offline; add little to install; open in one command. Renders of the mock with mermaid-cli and merman-cli matched; mmdr did not.

**Decision.** decree writes Mermaid `stateDiagram-v2` (and a `flowchart LR` system graph) inside Markdown files in `.decree/graph/`, committed with the project. Users view them in VS Code 1.121+, GitHub, GitLab or Obsidian, or paste a diagram into mermaid.live ([Viewing](reference/graph.md#viewing)). No image rendering, no DOT, no terminal rendering, no XState export (Stately Studio needs an account to view). A clickable graph that links to logs is a separate product built on decree.

**Consequences.** decree stays a small single binary, and the graph is byte-for-byte deterministic and testable. decree's contract for tools built on it: stable state ids in the diagrams, and `machine`, `state` and `run_id` on every event. Evidence: `a0079ea`, `docs/spikes/graph.md` and its renders at `32223bb`.

## D15: `onentry` and `onexit` run once per visit

**Context.** 0.4.2 had five hooks (`beforeAll`, `afterAll`, `beforeEach`, `afterEach`, `onDeadLetter`), and its git-stash hooks restored a baseline before the final retry. SCXML runs entry and exit content on each entry and exit of a state, not per retry.

**Decision.** `beforeAll` is root `onentry`, `afterAll` root `onexit`, `beforeEach` and `afterEach` a state's `onentry` and `onexit`, and `onDeadLetter` the `onentry` of `failed`. They run once per visit; `max_attempts` re-runs only the invoke. The git-stash hooks became `git_baseline` (root `onentry`, records `HEAD` once) and `snapshot` (a working state's `onentry`, a checkpoint each visit). Restoring the baseline before the final attempt is dropped: a machine that wants a clean retry loops back through a state.

**Consequences.** Hooks are visible in the machine and its graph. A run that ends `failed`, including after a failing `onentry`, runs `failed`'s `onentry` exactly once; 0.4.2 skipped `onDeadLetter` after a `beforeEach` failure, and 0.5 reverses that on purpose. Evidence: `b1d5613`, `446dce7`.

## D16: Delete with the last caller

**Context.** The rewrite removed 22 pieces of 0.4.2 (the global router prompt, fuzzy routine matching, the routine registry, the outbox, the dead-letter directory, hooks, three script resolvers and more) while the new code was built beside them. Deleting a symbol whose callers are still alive breaks the build; keeping dead code around leaves shims.

**Decision.** Every deletion starts with a grep of its symbol. A symbol is deleted by the change that removes its last caller, and that change also deletes whatever it leaves without a caller. No compatibility shims; the one exception is `routine:` as an alias of `machine:`, because migrations are immutable and existing projects have pending migrations that carry it.

**Consequences.** The tree builds at every step and ends with no dead code. A 0.4 project is upgraded once by `scripts/migrate-0.4-to-0.5.sh`, not by the binary. Evidence: `aa4139b` (the rule), `8ab1a72` (the bulk deletion).

## D17: No configuration file

**Context.** 0.4's `config.yml` held the router command, the default routine, `max_retries`, the emit depth, the log size and shared routines. After [D4](#d4-a-router-is-a-machine) and [D5](#d5-each-choose-model-names-its-router-or-uses-the-machine-named-router), what was left were defaults that hid behaviour outside the machines, and a strict-keys version still had to be kept in step with the migration script.

**Decision.** There is no configuration file ([No configuration file](reference/README.md#no-configuration-file)). The router is the machine named `router`; every message names its machine; `max_attempts` defaults to 1 per state; the emit depth limit (10) and the log cap (2 MiB) are fixed; shared machines and scripts are symlinks. The daemon interval is a flag. A leftover `.decree/config.yml` is an error naming `scripts/migrate-0.4-to-0.5.sh`.

**Consequences.** A project is machines, scripts and messages, and everything that affects a run is in them. Evidence: `dc2184a` (strict `config.yml`, superseded), `235d7e5` and `0652778` (removed).

## D18: C1: clippy fixes in test files

**Context.** The 0.4.2 baseline failed `cargo fmt --check` and had 21 clippy errors in `src/`. Once `src/` was clean, clippy found 3 `useless_conversion` errors in `tests/integration_test.rs`, but the clean-up was allowed to touch only formatting in tests.

**Decision.** Clippy fixes in `tests/` are allowed; the three were rewritten to `cargo_bin_cmd!("decree")`.

**Consequences.** `cargo clippy --all-targets -- -D warnings` passes and is a gate for every change. Evidence: `759fa59`.

## D19: C2: SIGTERM stops decree and its script

**Context.** 0.4.2 registered SIGINT twice and SIGTERM never, though a comment said it forwarded SIGTERM. SIGTERM killed decree by default and left the routine running in its own process group.

**Decision.** decree handles SIGINT and SIGTERM alike: it stops the running script's process group, appends an `interrupted` event with `cause: "signal"`, and exits (130 for `process`).

**Consequences.** `systemctl stop` and `docker stop` stop a run cleanly, and the stop is in the record. Evidence: `759fa59`.

## D20: C3: SIGKILL after a 10 s grace

**Context.** Both 0.4.2 executors sent SIGTERM to the group and then waited for the child with no deadline.

**Decision.** decree sends SIGTERM to the script's process group, waits up to 10 s for every process in it to exit, then sends SIGKILL, as `docker stop` does ([Execution](reference/scripts.md#execution)).

**Consequences.** A script that ignores SIGTERM cannot hang decree. Evidence: `759fa59`.

## D21: C4: one executor for every script

**Context.** Hooks, the precheck and the AI router command ran through `bash` with `.output()`: no process group, no timeout, not stopped on a signal.

**Decision.** One executor runs every script (invokes, `onentry`, `onexit`, `ask`), directly with no `bash` wrapper. Hooks became `onentry` and `onexit` ([D15](#d15-onentry-and-onexit-run-once-per-visit)), the precheck an ordinary first state, and the router command a router machine ([D4](#d4-a-router-is-a-machine)).

**Consequences.** Every script gets the same environment, log, timeout, signal handling and `script` event. Evidence: `759fa59`, `aa4139b`, `8ab1a72`.

## D22: C5: the inbox is first-in, first-out

**Context.** 0.4.2's `process` took the alphabetically last inbox file (or the deepest in the current chain) and the daemon took the last: last-in, first-out.

**Decision.** decree claims the inbox file with the lowest byte-order filename; emitted and cron messages are named by their time-ordered id, so they run first-in, first-out ([Lifecycle](reference/messages.md#lifecycle)).

**Consequences.** Order is predictable. A step that must finish before its parent continues belongs in the same machine ([D34](#d34-q8-emitted-messages-run-first-in-first-out)). Evidence: `759fa59` (claim), `cb22c9d` (0.4.2 selection deleted).

## D23: C6: the daemon runs the same pipeline as `process`

**Context.** 0.4.2's daemon had its own pipeline: it never ran migrations and handled messages differently from `process`.

**Decision.** `daemon` calls the same functions as `process`, adding a cron tick and a sleep ([cli.md](reference/cli.md)). A failed or interrupted inbox run does not stop it; a failed, interrupted or waiting migration blocks later migrations only.

**Consequences.** One pipeline to test. Evidence: `8f9634b`.

## D24: C7: deletions are checked with qualified names

**Context.** "Zero hits" for each deleted symbol cannot hold for leaf names such as `run`, `error`, `init` or `log`, which match code 0.5 keeps; and `routine` must keep matching as the `machine:` alias.

**Decision.** Deletions are verified with qualified patterns (`routine::levenshtein`, `fn run_precheck`, `HookType`, …).

**Consequences.** The checks are meaningful and pass. Evidence: `759fa59`, `8ab1a72`.

## D25: C8: no `max_retries` alias

**Context.** 0.4.2's routine configuration accepted `max_retries` as an alias of `max_attempts`, which no design document mentioned.

**Decision.** No alias: it went with the routine configuration. Retries are the state's `max_attempts`.

**Consequences.** One name. A 0.4 `config.yml` is not read at all ([D17](#d17-no-configuration-file)). Evidence: `8ab1a72`.

## D26: C9: `init` never touches an existing `.decree/`

**Context.** Without a terminal, 0.4.2's `init` proceeded over an existing `.decree/`; with one, it asked.

**Decision.** `init` asks nothing. It refuses an existing `.decree/` and exits 2; flags replace the prompts.

**Consequences.** `init` is safe in scripts and CI. Evidence: `759fa59`.

## D27: Q1: the `processed.md` ledger

**Context.** Migrations must run once, in order, with their history in git, but migration files are immutable.

**Decision.** The committed `processed.md` lists the migrations that ran. It is written before the final state's `onentry` scripts, so a `commit` there includes it ([Migrations](reference/messages.md#migrations-ordered-run-once-stop-on-error)).

**Consequences.** A fresh clone knows which migrations ran; `runs/` is never committed. Evidence: `759fa59`.

## D28: Q2: no commands configuration

**Context.** 0.4.2's `config::CommandsConfig` named the AI commands for routing and interactive use.

**Decision.** Replaced by the router machine ([D4](#d4-a-router-is-a-machine), [D5](#d5-each-choose-model-names-its-router-or-uses-the-machine-named-router)) and by the AI calls in the scripts `init` writes; `ai_interactive` is gone. The first answer, a `default_router` key, was superseded by [D17](#d17-no-configuration-file).

**Consequences.** Which AI is called is visible in scripts. Evidence: `aa4139b`, `0652778`.

## D29: Q3: every 0.4 hook maps to `onentry` or `onexit`

**Context.** 0.4.2 had five hook types.

**Decision.** All five map, as [D15](#d15-onentry-and-onexit-run-once-per-visit) lists.

**Consequences.** No hook concept remains. Evidence: `b1d5613`.

## D30: Q4: `init` writes the decree skill

**Context.** 0.4 had a `skill` command and a `sow` skill.

**Decision.** `decree init` writes a rewritten decree skill to `.claude/skills/decree/` or `.github/skills/decree/`, never overwriting an existing file. The `skill` command and the `sow` skill are gone.

**Consequences.** AI agents learn messages, machines and scripts from the project. Evidence: `43c9158`.

## D31: Q5: one run at a time per process

**Context.** Parallel runs would complicate locking, logs and GPU use.

**Decision.** One run at a time per process. The run lock only guards against two processes.

**Consequences.** A daemon never asks for two GPU services at once ([Long-running services](services.md)). Evidence: `a0079ea`.

## D32: Q6: routers are commands, not HTTP clients

**Context.** A router could call an HTTP API with schema-constrained output.

**Decision.** No HTTP in decree. decree validates replies itself; a router's script calls whatever it likes.

**Consequences.** No HTTP dependency; any backend fits through a script ([D4](#d4-a-router-is-a-machine)). Evidence: `a0079ea`.

## D33: Q7: no `decree replay` yet

**Context.** Re-running a run with its recorded router choices would make model decisions reproducible.

**Decision.** Deferred. `events.jsonl` and the router runs store what it needs ([D11](#d11-every-decision-is-auditable)).

**Consequences.** It can be added without changing the record. Evidence: `a0079ea`.

## D34: Q8: emitted messages run first-in, first-out

**Context.** A run can emit several follow-up messages.

**Decision.** They run first-in, first-out by filename ([D22](#d22-c5-the-inbox-is-first-in-first-out)). A step that must finish before the parent continues belongs in the same machine, as a state or a sub-machine.

**Consequences.** Emits are fire-and-forget. Evidence: `759fa59`.

## D35: Q9: free-form messages are routed by a machine

**Context.** 0.4.2's global router sent free-form messages to a routine.

**Decision.** An ordinary machine does it: a `choose: model` state picks an option, and the target state's script runs `decree emit` for the chosen machine (`mock/.decree/machines/triage.yml`).

**Consequences.** Routing is a visible, replaceable machine, not a global prompt. Evidence: `759fa59`.

## D36: Q10: a killed run waits for `decree retry`

**Context.** A run stopped by a signal or a crash could be continued automatically, as durable-execution engines do.

**Decision.** No. Signals and crashes both leave the run `interrupted`; only `decree retry` continues it. A kill may be deliberate, and decree cannot tell.

**Consequences.** Scripts must be safe to re-run; continuing re-runs root and ancestor `onentry` scripts. Evidence: `759fa59` (interrupts), `b1d5613` (`decree retry`).

## D37: Q11: one flat `scripts/` directory with per-machine overrides

**Context.** 0.4.2 had three script resolvers and nested routine directories.

**Decision.** One flat `scripts/` directory shared by all machines, with optional `scripts/<machine>/` overrides ([Resolution](reference/scripts.md#resolution)).

**Consequences.** Generic scripts are written once; an override is visible in the `script` event's `path`. Evidence: `759fa59`.

## D38: Q12: the router contract is `request.json` and `reply.json`

**Context.** A router could take a chat prompt, a typed choice (Jev), or both.

**Decision.** A router is a machine that reads `request.json` and writes `reply.json` ([D4](#d4-a-router-is-a-machine)). Prompts, models and retries live in it.

**Consequences.** Chat models and typed-choice models both fit. Evidence: `759fa59`, `aa4139b`.

## D39: Q13: graphs are viewed in existing tools

**Context.** A user outside an IDE or a website still needs to see a graph.

**Decision.** As [D14](#d14-the-graph-is-mermaid-text-in-markdown): Markdown files with Mermaid, opened in VS Code, GitHub or Obsidian, or pasted into mermaid.live. No renderer in decree.

**Consequences.** See D14. Evidence: `a0079ea`.

## D40: Q14: a person answers with a reply message

**Context.** A run waiting for a person should not need someone to run `decree retry`.

**Decision.** As [D8](#d8-asking-a-person-is-choose-person-plus-a-reply-message): a `choose: person` invoke plus reply messages. decree never knows it is a question.

**Consequences.** See D8. Evidence: `759fa59`.

## D41: Q15: a machine can invoke another machine

**Context.** Workflows compose: a release runs a feature, then a deploy.

**Decision.** `invoke: { machine: <name> }`, SCXML's child state machine. The child is a separate run with a `parent` reference, and the root final state it reaches is the parent state's event ([Sub-machines](reference/runs.md#sub-machines)). Routers are invoked the same way.

**Consequences.** Each machine stays small and has its own runs and graph. V20 forbids cycles. Evidence: `759fa59`, `6a35eaf`, `aa4139b`.

## D42: Q16: scripts do not run inside tmux

**Context.** Running scripts in tmux would make them easy to watch.

**Decision.** No: decree needs exact exit codes, separate stdout and stderr, and no terminal. Live visibility comes from `decree status` and `decree tail`; tmux stays a personal dashboard ([Long-running services](services.md)).

**Consequences.** Scripts are plain processes. Evidence: `b1d5613` (`decree tail`).

## D43: Delete finished runs on request, never automatically

**Context.** Run folders were never removed, so `runs/` grows by every run's events, logs and replies (about 64 KB for a mock migration run). Deleting a folder is not free of meaning: a failed migration's folder is what keeps `process` from starting it again, and a parent run may still read its child's results. A project that ships `events.jsonl` and the script logs to Loki ([observability.md](reference/observability.md)) already keeps its history there. Prior art: `docker system prune --filter until=<duration>` and `git gc --prune=<date>` delete on request with an age cut-off; AWS Step Functions keeps execution history for a fixed period and then deletes it.

**Decision.** `decree prune --older-than <age> [--dry-run]` deletes the folders of finished runs whose `run_finished` event is older than `<age>`, and nothing else ever deletes a run ([cli.md](reference/cli.md)). The age is required, so a bare `decree prune` deletes nothing. It keeps runs that are not finished, migrations that ended in `failed`, and children of unfinished parents, and it takes the run lock before deleting. No archive: it would only move the growth.

**Consequences.** Retention is the log store's job, and the local `runs/` is a working copy; a project that ships nothing keeps every run until someone prunes. `decree status <id>` and `decree retry` cannot reach a pruned run. Evidence: migration 70.

## D44: One shape per machine key

**Context.** The machine format had places where one idea took several shapes. `invoke` named its kind three ways: a bare string was a script, `check:` and `machine:` were keys, and `choose: model` and `choose: person` were values, each with its own sibling fields. Where a check's text came from was implicit: `{ matches: re }` read the `input:` state named beside `check:`, or, without one, whichever script ran last, and `matches` was both a subject and an operator. A check produced `yes` or `no` where a reader expects true and false. `max_attempts` and a script's `timeout_s` sat on the state while a person's `timeout_s` sat inside `invoke`. And the examples wrote prose inside flow maps, where an unquoted `?` or `, ` breaks YAML. People and models had to guess which shape a key took. Prior art for naming a kind by its key: serde's externally tagged enums (`{ variant: value }`, a map with one key), and GitHub Actions steps, which are a `uses:` step or a `run:` step by the key that is present.

**Decision.** One shape per key, explicit over implicit ([Invoke](reference/machines.md#invoke-the-states-function)):

- `invoke` is a map with exactly one key, which names the kind: `script`, `check`, `model`, `person` or `machine`. `invoke: <name>` is short for `invoke: { script: <name> }`, and a bare name under `script` or `machine` is short for `{ name: <name> }`.
- A check's events are `true` and `false`. YAML 1.2 reads `true:` and `false:` as booleans; decree reads a boolean key in `transitions` as that event name, so no quotes are needed.
- A condition has exactly one subject and one operator. `output: <state>` is the subject that reads a state's script output, and `matches` is only an operator. A `model` reads the state named by its `output`, or nothing: there is no fallback to the most recent script.
- Script settings (`max_attempts`, `timeout_s`) sit inside the script invoke, as a person's `timeout_s` sits inside `person`.
- Machines write decision invokes and prose in block style.

No old shape is accepted: `choose`, `input`, a bare `matches`, a state-level `max_attempts` or `timeout_s`, and `{ machine: x, params }` each fail V19 with a message that names the new shape. Nothing old is read or translated. This supersedes the syntax in [D5](#d5-each-choose-model-names-its-router-or-uses-the-machine-named-router), [D8](#d8-asking-a-person-is-choose-person-plus-a-reply-message) and [D12](#d12-ai-and-people-decide-only-where-a-machine-says-choose) (the decisions stand; `choose: model` is now `model:` and `choose: person` is now `person:`), and the input fallback in [D9](#d9-what-the-router-request-holds).

**Consequences.** A reader knows an invoke's kind from its one key and a condition's text from its named `output`, so a machine reads the same to a person and a model, and a JSON Schema can describe each kind on its own (migration 72). Every machine, recorded check event and doc example changed once, before 0.5 is released. A model state that names no `output` gets an empty `input`, so its prompt holds only what the machine names. Evidence: migration 71.

## D45: JSON Schema for shape, `decree check` for meaning

**Context.** The machine format was described only in prose, so a person learned a misspelled key from `decree check` after saving, and a model writing a machine had to infer the contract from examples. Prior art: JSON Schema (draft 2020-12) is the standard way to describe a JSON or YAML document; the YAML language server (VS Code's YAML extension by Red Hat, and others) applies a schema named in a `# yaml-language-server: $schema=<path>` comment, and SchemaStore publishes schemas in that form for GitHub Actions workflows, Docker Compose files and many other YAML formats. Models are held to JSON Schemas for structured output.

**Decision.** decree ships two schemas, `machine.schema.json` and `message.schema.json`, compiled in from `src/templates/schema/` as their single source. `decree schema` writes them to `.decree/schema/`, `decree init` writes them, and `decree check` warns when they are missing or stale, as it does for `.decree/graph/`. Every machine starts with `# yaml-language-server: $schema=../schema/machine.schema.json`. The schema describes everything about one file's shape: keys, types, required keys, each `invoke` kind and condition, name patterns and ranges, with `additionalProperties: false` everywhere as V19, and a description and examples on every key. What needs other files or the whole graph stays with `decree check`, which remains the authority ([Schema](reference/machines.md#schema)). The schema never accepts a machine that `decree check` rejects for its shape; `validation_test.rs` holds it to every case. It uses only keywords that draft-07 tools also understand, plus `$defs`, so older editors read it too.

**Consequences.** Editors complete keys and underline mistakes as you type, a model reads one precise contract, and any JSON Schema validator checks a machine without decree. There are two descriptions of the format to keep in step, the parser and the schema; the test table catches drift. The `jsonschema` crate is a dev-dependency only, so decree itself does not validate against the schema. Evidence: migration 72.
