# Project: what a decree project's files look like

This example is a small decree project frozen partway through its life. It has three machines, each the simplest that does its job, their scripts, one finished migration, one run waiting for a person, one interrupted run, and a person's reply waiting in the inbox.

Nothing here runs on its own: it is a snapshot, and the [reference](../../docs/reference/README.md) quotes it. Tests hold it to the reference: `decree check` passes here (installing its plugins first, as in a fresh clone), `decree graph` reproduces `.decree/graph/*.md` byte for byte, and each recorded run replays through decree to the same events. [`observability`](../observability/README.md) ships these runs to Grafana.

## Running it

Read it rather than run it: its scripts stand in for an AI agent and a deploy, and `decree process` would continue the recorded runs. These commands only read the project:

```bash
cd examples/project
decree check                          # every machine and message is valid
decree graph                          # rewrites .decree/graph/ with no change
decree status                         # the waiting run, the interrupted run and the queue
decree status 01-rate-limit-upload    # the finished migration: its states, scripts and logs
```

## The files

```text
examples/project/.decree/
  plugins/                  MACHINES and SCRIPTS, one file each: the committed source
    hello.md                  one script
    develop.md                implement (a local model, then Claude), then test
    deploy.md                 build, a person approves, ship
  machines/  scripts/       installed from plugins/ by `decree check`, `graph` or `process`; git ignores them
    machines/develop.yml      control flow, no code
    scripts/develop/          the work: implement.sh picks the model from $DECREE_ATTEMPT_VALUE and asks it
                              through ai.sh (`ai`; AI_CLI picks the backend); test.sh
    scripts/deploy/           build.sh, ship.sh, and ask_person.sh, which tells a person the options of a person state
  migrations/  processed.md MESSAGES: ordered, run once; processed.md lists 01 as run
  inbox/                    MESSAGES: FIFO queue; here, a person's reply
  cron/nightly-audit.md     a message template that drops into inbox/ every night
  runs/<id>/                one folder per run: message copy, events.jsonl, traces.jsonl, numbered logs
  graph/                    written by `decree graph`: one Markdown file per machine, plus system.md
  schema/                   written by `decree schema`: the JSON Schemas of every file
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

- [`runs/20261001T143005.000000Z-189ed2/`](.decree/runs/20261001T143005.000000Z-189ed2/events.jsonl), the run of migration `01-rate-limit-upload.md` on `develop`, finished `done`. `implement`'s `local` attempt failed (event 2, exit 1); decree ran the `claude` attempt in place (event 3, a `transition` with `source: "attempt"` and `attempt_value: "claude"`), which succeeded, and `test` passed.
- [`runs/20261001T153012Z-7b4e2a/`](.decree/runs/20261001T153012Z-7b4e2a/events.jsonl), a `deploy` run from the inbox, is waiting in `approval` for a person. `ask_person.sh` ([`deploy`](.decree/plugins/deploy.md)'s) printed the commands that answer it ([its log](.decree/runs/20261001T153012Z-7b4e2a/0002-approval-ask_person.log)). The person ran `decree event 20261001T153012Z-7b4e2a.w3 approve -m "..."`, which wrote [`inbox/20261001T160301Z-9be210.md`](.decree/inbox/20261001T160301Z-9be210.md); the next `decree process` delivers it and the run goes on to `ship`.
- [`runs/20261001T030000Z-c4e81b/`](.decree/runs/20261001T030000Z-c4e81b/events.jsonl), the nightly cron run on `develop`, was interrupted by Ctrl-C during `implement`. decree does not continue it on its own: `decree process --retry 20261001T030000Z-c4e81b` does.

[Runs](../../docs/reference/runs.md) describes every event in these logs.

In a real project `.decree/.gitignore` contains `inbox/` and `runs/`. This example commits them so you can read them.
