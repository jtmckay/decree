# Runs

One message moving through one machine is a run, stored in `.decree/runs/<id>/`:

```text
runs/<id>/
  message.md                 the claimed message; decree mirrors `state:` into it
  events.jsonl               one JSON line per event: the record and the telemetry
  0001-<state>-<script>.log  output of each script execution, numbered in run order
  request.json, reply.json   in a router's child run
  received/                  replies delivered to this run
  .lock, .running            the process stepping the run, and the script running now
```

## events.jsonl

The source of truth: the run's state is the `to` of its last `transition` event. Every line
carries `v`, `seq`, `ts`, `type`, `run_id`, `machine` and `trigger`. Types:

| `type` | Meaning |
| --- | --- |
| `transition` | `from`, `event`, `to`, `source` (`claim`, `exit_code`, `stdout`, `attempt`, `check`, `model`, `person`, `machine`, `timeout`, `internal`, `invalid_message`, `retry`). |
| `script` | One script finished: `state`, `phase`, `script`, `path`, `attempt`, `duration_ms`, `exit_code`, `log`. |
| `decision` | A `check` or `choose` produced its event: `kind`, `event`, and for models `pick`, `reason`, `confidence`, `router`, `child_run`. |
| `waiting` | Paused for a person (`wait_id`, `options`, `timeout_at`) or a child run (`child`). |
| `received` | A reply, a timeout or a child's result arrived. |
| `interrupted` | Stopped before a final state: `cause` is `signal` or `crash`. |
| `run_finished` | Reached a root final state. |

To find out why a run failed, read its `events.jsonl` from the end, then the log the last
`script` event names. `decree status <id>` shows the same as a table.

## Run status

| Status | Meaning | What to do |
| --- | --- | --- |
| `finished` | Reached a final state (`done`, `failed`, ...). | Nothing; or `decree retry` to run it again from a state. |
| `active` | A live process holds its lock. | `decree tail` to watch it. |
| `waiting` | Paused for a person, or for a child run. | `decree event <wait id> <event>`; `decree process` prints the commands. |
| `pending` | A reply arrived, or `decree retry` was run. | `decree process` continues it. |
| `interrupted` | Stopped by a signal or crash. | Fix the cause, then `decree retry <id>`. decree never continues it on its own. |

A failed, interrupted or waiting migration blocks every later migration; `decree process` exits 1
naming the `decree retry` command (or 0 while only waiting).

## Graphs

`decree graph` writes `.decree/graph/<machine>.md` (a Mermaid `stateDiagram-v2`) for each
machine and `.decree/graph/system.md` (machines, `emits` and `invokes` edges, cron entry points).
Edge labels say who decided: `(check)`, `(model)`, `(person)`, `(machine: <name>)`; `(implicit)`
marks an unhandled `error` going to `failed`. Commit the files; they render on GitHub, GitLab,
Obsidian and in VS Code's Markdown preview (`Ctrl+Shift+V`), or paste the diagram into
https://mermaid.live.

Read a machine's graph before changing it, and run `decree graph` and `decree check` after.
