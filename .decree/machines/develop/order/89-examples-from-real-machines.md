---
machine: rust_develop
---
# 89: Examples learn from real machines: attempt lists, one ComfyUI machine, commissions

## Overview

The user runs decree 0.5 at work with a `develop` machine (a local model, escalating to a stronger one) and a `comfy` machine that renders commissions on ComfyUI. Running them taught three things the examples do not show yet:

1. **Retrying with a stronger model is one state.** `route-by-complexity` needs three states for it today (`implement_local`, `implement_claude`, `tried_claude`). With `attempts` (migration 88) it is one state, whose script picks the model from `DECREE_ATTEMPT_VALUE`.
2. **ComfyUI's API is fire and forget, so a render is build, submit, await, fetch.** `text-to-media` posts a prompt and stops: it never knows whether the render worked, and never gets the file. It also has three nearly identical machines, one per workflow, where one machine with a `method` param does.
3. **A machine commissions work from another with `emits`:** the user's `develop` machine ends with a `commission` state that emits one `comfy` message per piece of art the change needs.

Each example must stay the **simplest machine that does its job**. Add no state, check, model decision or param that this migration does not name.

## Requirements

Read migration 88 (now done), `docs/reference/machines.md` and both examples first.

### 1. `examples/route-by-complexity/`

- `develop_by_size` becomes:

  ```yaml
  initial: describe
  states:
    describe:
      invoke: describe
      transitions: { done: size_up }
    size_up:                       # the classifier picks where to start; unsure or down goes to Claude
      invoke:
        model:
          question: How much reasoning does this change need?
          router: gliner_router
          min_confidence: 0.7
      transitions:
        small: { target: local_first, description: … }   # keep today's descriptions
        large: { target: claude_only, description: … }
        unsure: claude_only
        error: claude_only
    local_first:                   # each attempt implements, then runs the tests; a failure moves to the next
      invoke:
        script: { name: implement, attempts: [local, local, claude] }
      transitions: { done: done }
    claude_only:
      invoke:
        script: { name: implement, attempts: [claude, claude] }
      transitions: { done: done }
    done:   { final: true }
    failed: { final: true }
  ```

  Keep `describe` and the `stop` handling as they are today, adapted: a `stop` from any attempt ends the run in `failed`, as now.
- One script, `develop_by_size/implement.sh`, replaces `implement_local.sh`, `implement_claude.sh` and `verify.sh`. Its value names the tool: `local` runs `opencode run --model ollama/$OLLAMA_MODEL`, and `claude` runs `claude -p` with `progress.md` and `STOP` as `implement_claude.sh` does today. Then it runs `$TEST_CMD` (`cargo test` by default) and exits non-zero if the tests fail, so a failing attempt moves to the next value. Any other value is an error naming it.
- README: the diagram (regenerate with `decree graph` and paste it), the steps, the LogQL queries (local attempts that failed now show as `source: "attempt"` transitions with `attempt_value`), and the file list. Explain in two sentences why the tests run inside the attempt: an attempt succeeds when the change passes the tests.

### 2. `examples/text-to-media/`

- **One machine, `comfy`,** replaces `comfy_image_text`, `comfy_image_text_image` and `comfy_video_i2v`:

  ```yaml
  data:
    method: { type: string, default: "" }        # a workflow in workflows/, by file stem; required
    output: { type: string, default: "" }        # repo path without extension; required
    input_image: { type: string, default: "" }   # repo path, uploaded to ComfyUI; for image-to-* methods
    width:  { type: int, default: 0 }            # 0 keeps the workflow's own value
    height: { type: int, default: 0 }
    seed:   { type: int, default: -1 }           # -1 keeps the workflow's own value
  initial: build
  states:
    build:                         # patches workflows/<method>.json with the params; fails on an unknown method or a missing required param
      invoke: build
      transitions: { done: submit }
    submit:                        # uploads input_image, POST /prompt, keeps the prompt id
      invoke:
        script: { name: submit, attempts: 3, timeout: 2m }
      transitions: { done: await }
    await:                         # polls /history/<prompt id> until the prompt finishes
      invoke:
        script: { name: await, timeout: 3h }
      transitions: { done: fetch }
    fetch:                         # downloads each output to `output` (+ its extension)
      invoke:
        script: { name: fetch, attempts: 3, timeout: 10m }
      transitions: { done: done }
    done:   { final: true }
    failed: { final: true }
  ```

  - `build` reads the method from the param; no state per method. A wrong method fails in `build` with the list of workflows.
  - `submit` and `fetch` are safe to repeat, so they have attempts. `await` has none: if a render never finishes, that is something to look at, not to retry.
  - `COMFY_URL` (default `http://127.0.0.1:8188`, as in `tmux-services`) replaces the `api_url` param.
  - Keep `precheck` as the root `onentry` (curl and jq).
  - Each workflow file keeps its name, so `tmux-services`' `COMFY_WORKFLOW` default still resolves. The methods are those file stems.
- **README:**
  - The machine, its graph and the message format: one example message per method.
  - **"Commissioning from another machine":** a fragment of a state with `emits: [comfy]` whose script runs `decree emit --machine comfy --param method=… --param output=…` once per piece of art, as the user's `develop` machine does after a change is verified. Explain that each piece becomes its own run, so one failed render does not fail the change.
  - **"When messages stop naming a method":** the next step to take only once senders cannot name the method. Show a YAML fragment: `check` states on which images were given narrow the choice, then a typed `model` decision (`gliner_router`, as in `route-by-complexity`) picks among only the methods that fit, with `unsure: failed`. Say plainly that the example leaves this out on purpose, because it is not needed until then.
- `examples/tmux-services/` uses `text-to-media`'s workflow; check that its README and scripts still match, and change only what migration 89 breaks.

### 3. Everything else

- `docs/` or other examples that name the removed machines or scripts are updated to match.
- `CHANGELOG.md`, under Changed: `route-by-complexity` retries with Claude through an attempt list, and `text-to-media` is one `comfy` machine that waits for the render and saves it.

- Only this migration's scope. No change to decree's behaviour.
- Do not edit `.decree/migrations/` or `.decree/runs/`.
- If anything here contradicts the reference docs in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model, ComfyUI or the network. No new dependencies.
- Print the evidence for each acceptance criterion at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.

## Acceptance Criteria

- **Given** `examples/route-by-complexity` and `examples/text-to-media`
  **When** `decree check` and `decree graph` run in each
  **Then** both exit 0, and `decree graph` changes nothing that is committed

- **Given** `develop_by_size`
  **When** it is read
  **Then** it has the states `describe`, `size_up`, `local_first`, `claude_only`, `done` and `failed`, and no others

- **Given** `implement.sh` with a stubbed `opencode`, `claude` and `TEST_CMD=false` on `PATH` (a test in a temp copy, or a shell check printed as evidence)
  **When** `local_first` runs
  **Then** the script runs three times, with `local`, `local` and `claude`, and the run ends in `failed`

- **Given** the `comfy` machine
  **When** it is read
  **Then** it has the states `build`, `submit`, `await`, `fetch`, `done` and `failed`, and no other `.decree/machines/` file exists in `text-to-media`

- **Given** a stub ComfyUI (a small local HTTP server in a test, or `curl` replaced on `PATH`) that accepts a prompt, reports it finished and serves one image
  **When** a `comfy` message with `method` and `output` is processed
  **Then** the image is saved at `output` with its extension, and the run ends in `done`

- **Given** a `comfy` message with an unknown `method`
  **When** it is processed
  **Then** the run fails in `build`, and the log lists the available methods
