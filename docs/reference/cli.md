# CLI

decree has 13 commands, plus `--version` (`-v`). `decree` with no command runs `decree process`. Every command takes `--no-color`; without it, colour follows `NO_COLOR` and whether the output is a terminal. Every command that reports something takes `--format <text|json>`, and `decree check` also `--format sarif` ([Machine-readable output](#machine-readable-output)).

Every command except `init` and `help` works on the project whose `.decree/` is in the current directory or the nearest ancestor. Without one it fails with `not inside a decree project (run \`decree init\` first)`. Errors are printed to stderr as `error: <message>`.

| Command | Behaviour | Exit code |
| --- | --- | --- |
| `decree init [--ai <claude\|copilot\|opencode>] [--permissions]` | Creates the [`.decree/` layout](README.md#file-layout); the `router` machine with its script ([The default router](runs.md#the-default-router)); the `develop` and `rust_develop` machines with their scripts in `scripts/develop/` and `scripts/rust_develop/`; the shared scripts `git_baseline` (a root `onentry` that records `HEAD` once, safe to repeat) and `snapshot` (a working state's `onentry` that stashes a checkpoint on each visit), which no built-in machine uses; `.decree/graph/`; `.decree/schema/` ([Schema](machines.md#schema)); and the decree skill, in `.claude/skills/decree/` (`claude`) or `.github/skills/decree/` (`copilot`), never overwriting an existing file. `--ai` picks which AI the `router` machine asks and the `develop` machines' scripts call: the router's script is `scripts/router/ask_<ai>.sh`, differing only in the CLI call (`claude -p`, `copilot -p`, `opencode run`); without `--ai`, the first of `opencode`, `claude`, `copilot` found on `PATH`, else `opencode`. `--permissions` writes the backend's default permissions file: `.claude/settings.json` allowing `Write` and `Edit` (`claude`), or `opencode.json` (`opencode`), unless the file exists; for `copilot` it prints how to set them. Everything it writes passes `decree check`. No interactive prompts. Refuses to touch an existing `.decree/`. | 0, or 2 if `.decree/` exists |
| `decree process [--dry-run [--format <text\|json>]]` | `--dry-run` lists what would run and runs nothing. Otherwise: validates all machines and pending migrations. Then marks crashed runs `interrupted` and repeats until nothing is left: deliver replies and timeouts, continue `pending` runs, drain `inbox/` in filename order, start the next migration ([Migrations](messages.md#migrations-ordered-run-once-stop-on-error)). Never continues an `interrupted` run. Stops at the first run that ends in `failed`, and before a `failed`, `interrupted` or `waiting` migration. Ends by printing every waiting run with its question, the accepted events and a `decree event` command for each. | 0 if everything finished or is waiting, 1 if it stopped on a `failed` or `interrupted` run or invalid input, 130 on SIGINT or SIGTERM (the current run is `interrupted`) |
| `decree daemon [--interval <duration>]` | Validates all machines, marks crashed runs `interrupted`, then loops: deliver replies and timeouts, continue `pending` runs, cron tick, drain inbox, next migration, sleep `--interval`, a [duration](machines.md#durations) (default `2s`). It calls the same functions as `process`; there is no second pipeline. A failed or interrupted inbox run does not stop it; a failed, interrupted or waiting migration blocks later migrations only. On SIGINT or SIGTERM it interrupts the current run ([Replies](messages.md#replies), Stopping) and exits. | 0 on signal stop, 2 for a bad `--interval` |
| `decree check [--format <text\|json\|sarif>]` | Runs V1–V21 and M1–M3. Prints one line per error. Warns on stderr, without failing, when `.decree/graph/` or `.decree/schema/` is missing or differs from what `decree graph` or `decree schema` would write, including a file there that they do not write. | 0 if valid, else 1 |
| `decree graph [--format <text\|json>]` | Writes `.decree/graph/<machine>.md` for every machine and `.decree/graph/system.md` ([graph.md](graph.md)), removes stale `.md` files there, and prints the paths. decree renders no images; `--help` says how to view the files. | 0 |
| `decree schema [--format <text\|json>]` | Writes the JSON Schemas (draft 2020-12) of every file decree reads or writes into `.decree/schema/v1/`: `events`, `machine`, `message`, `reply` and `request`, each `<name>.schema.json` ([Schemas](README.md#schemas)), and of each `--format json` document into `.decree/schema/v1/cli/<command>.schema.json` ([Machine-readable output](#machine-readable-output)), each through a temp file and a rename, removes every other file in `.decree/schema/`, and prints the paths. Machines point at the first with `# yaml-language-server: $schema=../schema/v1/machine.schema.json`, so editors complete keys and underline mistakes. | 0 |
| `decree emit --machine <id> [--param k=v]... [--format <text\|json>]` | Reads the body from stdin and writes `inbox/<id>.md` via temp file and rename. Sets `parent` from `DECREE_MESSAGE_ID` and `depth` to the parent's `depth` + 1; refuses if that exceeds `max_depth`. Sets `trigger: emit`. If `DECREE_MACHINE` and `DECREE_STATE` are set, `<id>` must be in that state's `emits`. Validates `--param` against the target machine. Prints the new `id`. | 0, or 1 on any check failure |
| `decree status [<id>] [--cron] [--format <text\|json>]` | No id: counts and lists of runs by status (`active`, `waiting`, `pending`, `interrupted`, finished per final state) and of queued messages; for each `active` run, the script running now with its pid, how long it has run and its log path (from `.running`, [Execution](scripts.md#execution)); for each `waiting` run, its wait id and options (or the child it waits for). With id: frontmatter, status, and the events as a table (transitions, scripts with durations, decisions, waits and replies). `--cron`: cron files and next fire time, as text only. | 0, or 2 for `--cron --format json` |
| `decree tail [<id>]` | Follows the live output of a run, by default the `active` one: prints the log of the script running now (named in `.running`) as it is written, with a header line per script (`== 0004 implement/implement ==`), and moves on to the next script's log as the run proceeds, including into child runs. Stops when the run finishes, waits or is interrupted. Reads the log files only; it never touches the run. | 0, or 1 if there is no such run |
| `decree retry <id> [--state <s>] [--format <text\|json>]` | For `interrupted` and finished runs; not for `waiting` runs, which take a reply instead. Appends a `transition` event with `source: "retry"` and `to: <s>`, and mirrors `state`; the run is then `pending` and the next `process` or `daemon` continues it. Default `<s>`: for an interrupted run, the state it was in; for a finished run, the `from` of the last `transition` that has one. | 0, or 1 if the run is `active` or `pending`, or `<s>` is not an atomic state |
| `decree prune --older-than <age> [--dry-run] [--format <text\|json>]` | Deletes `runs/<id>/` for every finished run (its last event is `run_finished`) whose `run_finished` `ts` is older than `<age>`, a [duration](machines.md#durations) (`30d`, `12h`, `90m`); `0s` means every finished run. `--older-than` is required, so a bare `decree prune` deletes nothing; nothing else ever deletes a run. Never deleted: a run that is not finished; a migration run that ended in `failed` (its file is not in `processed.md`, and its folder is what keeps `process` from starting it again, [Migrations](messages.md#migrations-ordered-run-once-stop-on-error)); a child run (its `message.md` has `parent`) whose parent run still exists and is not finished. For each run it takes the [run lock](messages.md#run-lock), skips the run if the lock is held, checks again under the lock, then deletes the folder. Prints, in `id` order, `pruned <id>  <machine>  <final state>  finished <ts>` per run, then `pruned N run(s), <size> freed` with the size of the files deleted in KB, MB or GB. `--dry-run` takes no lock, deletes nothing and prints `would prune …` and `would prune N run(s), <size>`. Ship runs to a log store before pruning them ([observability.md](observability.md)). | 0, 1 on an I/O error (after pruning what it could), 2 for a missing or bad `<age>` |
| `decree event <wait id \| run id> <event> [-m <note>] [--format <text\|json>]` | Writes a reply message ([Replies](messages.md#replies)) to `inbox/` via the `emit` writer, with the note as its body. Checks first that the run is waiting and accepts the event, so mistakes fail at once. The next `process` or `daemon` pass delivers it. | 0, or 1 if the run is not waiting or does not accept the event |
| `decree help` | Prints the full help: commands, the `.decree/` layout, a message and a machine example. `--help` (on `decree` or any command) prints the short help. | 0 |

Usage errors (an unknown command or flag, a missing argument, `--format` where it does not apply) exit 2.

Error formats:

- `process` and `daemon` validate every machine (V1–V21) before anything runs. If any fails, they print each error as `decree check` does, then `N machine error(s); nothing was processed. Run \`decree check\`.`, and exit 1.
- An invalid pending migration stops them the same way: `N migration(s) are invalid; nothing was processed.` ([Migrations](messages.md#migrations-ordered-run-once-stop-on-error), rule 6).
- A waiting run is printed as:

  ```text
  Waiting: run <run id> in `<state>`: <the state's description, else its id>
    wait id <wait id>, options: <option>, <option>
    decree event <wait id> <option>
  ```

  with one `decree event` line per option.

## Machine-readable output

CI systems and AI agents read `--format json` instead of parsing the text. `--format <text|json>` is on `check`, `status` (with and without an id), `emit`, `event`, `retry`, `prune`, `graph`, `schema` and `process --dry-run`. `text` is the default and unchanged. With `json`, the command prints exactly one JSON document on stdout (pretty-printed; keys in any order), errors still go to stderr, and the exit code is the text mode's. A command that fails before it has anything to report (`emit`, `event` or `retry` refusing, `graph` with a broken cron file) prints nothing on stdout. `process` (without `--dry-run`), `daemon` and `tail` produce streams, and `init` and `help` are for people, so they take no `--format`; nor does `status --cron`.

Each document has a JSON Schema in `.decree/schema/v1/cli/<command>.schema.json`, written by `decree schema` with the others ([Schemas](README.md#schemas)) and versioned the same way ([Versioning](README.md#versioning)). Paths are relative to the project root (`.decree/inbox/<id>.md`), except the files `check` names, which are relative to `.decree/`, as in its text.

| Command | Document |
| --- | --- |
| `check` | `{ "valid", "errors": [{ "rule", "file", "line"?, "state"?, "message" }], "warnings": [{ "file", "message" }] }`. One error per text line, in the same order: `rule` is `V1`…`V21` or `M1`…`M3` ([Validation](machines.md#validation)), or `null` for an error no rule names, such as YAML that does not parse; `line` when the error names a line, `state` (a dotted path) when it is inside a state; `message` without the rule. `warnings` are what the text prints on stderr. Exit 1 when `errors` is not empty. |
| `status` | `{ "counts": { "total", "active", "waiting", "pending", "interrupted", "finished" }, "runs": { "active": [run], "waiting": [run], "pending": [run], "interrupted": [run], "finished": { "<final state>": [run] } }, "queued": { "inbox": [file], "migrations": [file] } }`. A run is `{ "id", "machine", "state" }`, plus `running` (`script`, `phase`, `state`, `pid`, `started_at`, `log`) for an `active` run, `wait_id` and `options` for a run waiting for a person, or `child` for a run waiting for a child run. |
| `status <id>` | `{ "id", "machine", "status", "state", "events" }`: the derived status, the current state (`null` before the first transition), and every line of `events.jsonl` parsed, each valid against `events.schema.json`. An unknown id is reported on stderr, with nothing on stdout and exit 0, as in text. |
| `emit`, `event` | `{ "id", "path" }`: the queued message and its file, `.decree/inbox/<id>.md`. |
| `retry` | `{ "id", "state" }`: the run, pending in that state. |
| `prune` | `{ "runs": [{ "id", "machine", "state", "finished" }], "bytes", "dry_run" }`: the runs deleted (or, with `--dry-run`, that would be), their final state and `run_finished` time, and their size in bytes. A run that could not be pruned is reported on stderr after the document, with exit 1. |
| `graph`, `schema` | `{ "written": [path], "removed": [path] }`: the files written, and the stale ones removed. |
| `process --dry-run` | `{ "migrations": [item], "inbox": [item] }`, in the order `process` would take them. An item is `{ "file", "valid", "machine"? , "to"? }`: `machine` for a message that starts a run, `to` for a reply; an invalid one has neither, and its errors go to stderr with exit 1. |

### SARIF

`decree check --format sarif` prints a [SARIF 2.1.0](https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/sarif-v2.1.0-errata01-os-complete.html) log, the OASIS standard that GitHub code scanning, GitLab and Azure DevOps read, so machine errors show up inline in pull requests. It has one run, whose `tool.driver` is `decree` with its version, an `informationUri` (these reference docs) and one `rules` entry per V1–V21 and M1–M3, each with its `id`, a `shortDescription` (the check in the [Validation](machines.md#validation) table) and a `helpUri` to that table. Each error is a `result` with `ruleId` (absent for an error no rule names), `level: "error"`, `message.text` (prefixed with the state path when the error is inside a state) and a `physicalLocation` whose `artifactLocation.uri` is the file relative to the project root (`.decree/machines/develop.yml`), with `region.startLine` when the error names a line. Each warning is a result with `level: "warning"` and no `ruleId`. Exit 1 if there are errors, as in text and JSON.

A GitHub Actions job that uploads it to code scanning with [`github/codeql-action/upload-sarif`](https://github.com/github/codeql-action/tree/main/upload-sarif) ([Uploading a SARIF file to GitHub](https://docs.github.com/en/code-security/code-scanning/integrating-with-code-scanning/uploading-a-sarif-file-to-github)); `|| true` keeps the job going to the upload when there are errors, and the last step fails the job on them:

```yaml
name: decree check
on: [push, pull_request]
jobs:
  check:
    runs-on: ubuntu-latest
    permissions:
      security-events: write # upload results to code scanning
      contents: read
    steps:
      - uses: actions/checkout@v4
      - run: cargo install decree
      - run: decree check --format sarif > decree.sarif || true
      - uses: github/codeql-action/upload-sarif@v4
        with:
          sarif_file: decree.sarif
          category: decree
      - run: decree check
```

Code scanning on a private repository needs GitHub Code Security (or Advanced Security).
