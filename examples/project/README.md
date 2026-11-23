# Project: what a decree project's files look like

This example is a small decree project frozen partway through its life. It has three machines, each the simplest that does its job, their scripts, one finished order file, one run waiting for a person, one interrupted run, and a person's reply waiting in the inbox.

Nothing here runs on its own: it is a snapshot, and the [reference](../../docs/reference/README.md) quotes it. Tests hold it to the reference: `decree check` passes here (installing its plugins first, as in a fresh clone), `decree graph` reproduces each `graph.md` and `system.md` byte for byte, and each recorded run replays through decree to the same events. [`observability`](../observability/README.md) ships these runs to Grafana.

## Running it

Read it rather than run it: its scripts stand in for an AI agent and a deploy, and `decree daemon --once` would continue the recorded runs. These commands only read the project:

```bash
cd examples/project
decree install                        # install the plugins, as after any clone (git ignores what is installed)
decree check                          # every machine and message is valid
decree graph                          # rewrites each graph.md and system.md with no change
decree status                         # the waiting run, the interrupted run and the queue
decree status 20261001T143005.000000Z-189ed2  # the finished order file's run: its states, scripts and logs
```

## The files

```text
examples/project/.decree/
  plugins/                  MACHINES and SCRIPTS, one file each: the committed source
    hello.md                  one script
    develop.md                implement (a local model, then Claude), then test; and a nightly cron file
    deploy.md                 build, a person approves, ship
    decree_order.md           the official plugin that runs each machine's order/ at start (decree's copy)
    decree_ai.md              the official library develop's implement.sh sources (decree's copy)
  machines/<name>/          one folder per machine, installed from plugins/ by `decree install`
    machine.yml               control flow, no code; git ignores it
    scripts/                  the work: develop's implement.sh picks the model from $DECREE_ATTEMPT_VALUE and
                              asks it through ai.sh (`ai`; AI_CLI picks the backend); deploy's build.sh,
                              ship.sh, and notify_person.sh, which tells a person the options of a reply state
    cron/nightly-audit.md     (develop) a message template that drops into inbox/ every night
    order/01-rate-limit-upload.md
                              (develop) MESSAGES: run in order at each start; committed
    state/order.git.txt       (develop) the order files that ran, with their hashes; committed
    graph.md                  written by `decree graph`; committed
  system.md                 written by `decree graph`: how the machines connect
  inbox/                    MESSAGES: FIFO queue; here, a person's reply
  runs/<id>/                one folder per run: message copy, events.jsonl, traces.jsonl, numbered logs
  schema/                   written by `decree install`: the JSON Schemas of every file
```

[`develop`](.decree/plugins/develop.md) is the end of the decree skill's growth path: a straight line, with an attempt list where runs showed the need. Installed, its machine is:

```yaml
name: develop
description: Implement a message, then test it.
initial: implement
states:
  implement:
    invoke:                        # the local model fails often: try Claude second
      script: { name: implement, attempts: [local, claude] }
    transitions: { done: test }
  test:
    invoke: test
    transitions: { done: done }
  done:   { final: true }
  failed: { final: true }
```

## The runs

- [`runs/20261001T143005.000000Z-189ed2/`](.decree/runs/20261001T143005.000000Z-189ed2/events.jsonl), the run of the order file `01-rate-limit-upload.md` on `develop`, finished `done`, and recorded in `state/order.git.txt`. `implement`'s `local` attempt failed (event 2, exit 1); decree ran the `claude` attempt in place (event 3, a `transition` with `source: "attempt"` and `attempt_value: "claude"`), which succeeded, and `test` passed.
- [`runs/20261001T153012Z-7b4e2a/`](.decree/runs/20261001T153012Z-7b4e2a/events.jsonl), a `deploy` run from the inbox, is waiting in `approval` for a person. `ask_person.sh` ([`deploy`](.decree/plugins/deploy.md)'s) printed the commands that answer it ([its log](.decree/runs/20261001T153012Z-7b4e2a/0002-approval-notify_person.log)). The person ran `decree reply 20261001T153012Z-7b4e2a.w3 approve -m "..."`, which wrote [`inbox/20261001T160301Z-9be210.md`](.decree/inbox/20261001T160301Z-9be210.md); the next `decree daemon --once` delivers it and the run goes on to `ship`.
- [`runs/20261001T030000Z-c4e81b/`](.decree/runs/20261001T030000Z-c4e81b/events.jsonl), the nightly cron run on `develop`, was interrupted by Ctrl-C during `implement`. decree does not continue it on its own: `decree retry 20261001T030000Z-c4e81b` does.

[Runs](../../docs/reference/runs.md) describes every event in these logs.

In a real project `.decree/.gitignore` contains `inbox/` and `runs/`. This example commits them so you can read them.
