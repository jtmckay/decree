# An escalation ladder for develop

The built-in `develop` machine gives a message one pass: implement, gate, fix, verify. When `verify` fails, or the gate still fails after `fix`, the run ends in `failed`. This recipe adds a ladder: rounds that close what is still missing, first with a cheap agent (a local model through OpenCode), then one round with Claude, then a person. It comes from a beta project where it worked well.

It is more machinery than the simplest machine, which is where a project should start ([Start with the simplest machine](../src/templates/skills/decree/SKILL.md)). Add it once plain `develop` runs show that failed verifies are worth more rounds.

## How it works

```text
… final_gate ─fail→ gate_gaps ─→ rounds_left ─true→ fill_gaps (gaps, ai_cli: default) ─unfilled→ rounds_left
     verify ─fail────────────────↗           └false→ claude_round (gaps, ai_cli: claude) ─unfilled→ ask_human
                                                     filled → done                         accept → done · retry → claude_round · fail → failed
```

- `verify` writes `gaps.md` when it fails ([Run-directory files](reference/scripts.md#run-directory-files)), and `gate_gaps` writes it from `gate.log` when the gate still fails after `fix`. `gaps.md` is what each round works on.
- `rounds_left` is a `check` on `visits`: `fill_gaps` runs `max_gap_rounds` times. A `decree process --retry` counts visits again from the retry, so a retried run gets every round again ([Visits](reference/runs.md#step-loop)).
- Each round is a child run of the `gaps` machine: fill, gate, fix, final gate, verify. Its steps are `develop`'s own scripts, through symlinks. They keep their files (`progress.md`, `gaps.md`, `gate.log`, `plan.md`, `STOP`) in the root run's folder, `$DECREE_ROOT_RUN_DIR`, so every round reads and adds to the same notes as `develop`.
- `gaps`' data entry `ai_cli` picks the backend: `lib/ai.sh`'s `ai` calls the one it names ([The AI helper](reference/scripts.md#the-ai-helper)). `fill_gaps` uses the default; `claude_round` passes `params: { ai_cli: claude }`. Every call is a line in `.decree/ai-use.md`, so you can see what each round cost.
- `ask_human` is a `person` state: the run waits for `decree event <wait id> accept|retry|fail`. After 30 days with no answer its event is `error`, which, like a round that ends in `failed`, goes to `failed`.

## The files

Start from a project `decree init` wrote. Replace `.decree/machines/develop.yml` with this one; it is the built-in machine with `data`, and the states from `final_gate` on changed or added.

`.decree/machines/develop.yml`:

```yaml
# Graph: ../graph/develop.md
name: develop
description: Implement a message with an AI in small steps, run the gate, fix and verify; then spend rounds on what is missing, first with the default agent, then with Claude, then ask a person.
data:
  max_gap_rounds: { type: int, default: 2 }
onentry: [clean_tree]              # the run starts from a clean git work tree
initial: precheck
states:
  precheck:                        # fails fast if lib/infra/requirements is not all there
    invoke: precheck
    transitions: { done: implement }
  implement:                       # logs each step in progress.md; writes STOP instead of guessing
    invoke:
      script: { name: implement, attempts: 3 }
    transitions: { done: gate, stop: failed }
  gate:                            # the project's checks: pass or fail
    invoke: gate
    transitions: { pass: verify, fail: fix }
  fix:                             # the AI fixes what gate.log reports
    invoke:
      script: { name: fix, attempts: 3 }
    transitions: { done: final_gate, stop: failed }
  final_gate:                      # the gate again; its failures become gaps
    invoke: gate
    transitions: { pass: verify, fail: gate_gaps }
  gate_gaps:                       # gate.log's failures into gaps.md
    invoke: gate_gaps
    transitions: { done: rounds_left }
  verify:                          # the AI checks the acceptance criteria; on fail it writes gaps.md
    invoke:
      script: { name: verify, attempts: 3 }
    transitions: { pass: done, fail: rounds_left, stop: failed }
  rounds_left:                     # a retry gets every round again
    invoke:
      check: { visits: fill_gaps, less_than: { data: max_gap_rounds } }
    transitions: { true: fill_gaps, false: claude_round }
  fill_gaps:                       # one round with gaps' default agent
    invoke:
      machine: { name: gaps }
    transitions: { filled: done, unfilled: rounds_left }
  claude_round:                    # one round with Claude
    invoke:
      machine: { name: gaps, params: { ai_cli: claude } }
    transitions: { filled: done, unfilled: ask_human }
  ask_human:                       # every round is spent: a person decides
    invoke:
      person:
        question: Every round is spent. Read gaps.md and progress.md in the run folder; accept the work, give Claude another round, or fail the run.
        ask: ask_human
        timeout: 30d
    transitions:
      accept: { target: done, description: Commit the work as it is. }
      retry: { target: claude_round, description: Give Claude another round; put any guidance in plan.md first. }
      fail: { target: failed, description: Stop here and keep the changes for a person. }
  done:   { final: true, onentry: [commit] }   # one commit per run
  failed: { final: true, onentry: [keep] }     # the run's changes stay in the tree; its attention file holds the queue
```

`.decree/machines/gaps.yml`, one round. Set `ai_cli`'s default to the backend your cheap rounds use:

```yaml
# Graph: ../graph/gaps.md
name: gaps
description: One round on gaps.md, by the CLI `ai_cli` names; ends filled if verify passes.
data:
  ai_cli: { type: string, default: opencode, enum: [opencode, claude, copilot] }
initial: fill
states:
  fill:                            # closes each gap in gaps.md, test first
    invoke:
      script: { name: fill, attempts: 3 }
    transitions: { done: gate, stop: failed }
  gate:
    invoke: gate
    transitions: { pass: verify, fail: fix }
  fix:
    invoke:
      script: { name: fix, attempts: 3 }
    transitions: { done: final_gate, stop: failed }
  final_gate:
    invoke: gate
    transitions: { pass: verify, fail: gate_gaps }
  gate_gaps:
    invoke: gate_gaps
    transitions: { done: unfilled }
  verify:
    invoke:
      script: { name: verify, attempts: 3 }
    transitions: { pass: filled, fail: unfilled, stop: failed }
  filled: { final: true }
  unfilled: { final: true }
  failed: { final: true }
```

`gaps` reuses `develop`'s `gate`, `fix` and `verify`, and the `gate_gaps` below, through symlinks in its own script folder ([Resolution](reference/scripts.md)):

```bash
mkdir -p .decree/scripts/gaps
for s in gate fix verify gate_gaps; do ln -s "../develop/$s.sh" ".decree/scripts/gaps/$s.sh"; done
```

`.decree/scripts/gaps/fill.sh`, the only script of its own. It notes the round in `progress.md` and asks the round's agent to close each gap:

```bash
#!/usr/bin/env bash
# gaps' fill: close each gap in gaps.md, test first, with the agent the round names.
set -euo pipefail
. "${DECREE_LIB}/ai.sh"

dir=${DECREE_ROOT_RUN_DIR}
gaps="${dir}/gaps.md"
progress="${dir}/progress.md"
plan="${dir}/plan.md"
stop="${dir}/STOP"
stopped() {
  [ -f "${stop}" ] || return 1
  cat "${stop}" >&2
  echo stop > "${DECREE_EVENT_FILE}"
}
stopped && exit 0
if [ "${DECREE_ATTEMPT}" = 1 ]; then
  round=$(($(grep -c '^## Gaps, round' "${progress}" 2>/dev/null || true) + 1))
  printf '\n## Gaps, round %s (ai_cli: %s)\n' "${round}" "${DECREE_DATA_AI_CLI}" >> "${progress}"
fi

prompt="You are a senior engineer. ${gaps} lists what is still missing in the work on
${DECREE_MESSAGE}; ${progress} notes what was done so far. Close each gap, test first:
write or fix the test that shows it, then the code. Append a line to ${progress} for each
gap you close. If ${plan} exists, follow it. If a gap needs a decision the message does
not make, write the question to ${stop} and stop.
Write scratch files only under ${DECREE_RUN_DIR}, never outside the project (not /tmp)."
echo "=== AI prompt (gaps, ai_cli: ${DECREE_DATA_AI_CLI}) ==="
echo "${prompt}"
ai "${prompt}"
stopped || true
```

`.decree/scripts/develop/gate_gaps.sh`, for a gate that still fails after `fix`. It keeps the end of `gate.log`; a project can pick its test tool's failed tests and first errors out of the log instead:

```bash
#!/usr/bin/env bash
# The gate still fails after fix: its failures are the gaps for the next round.
set -euo pipefail
dir=${DECREE_ROOT_RUN_DIR}
{
  echo "The gate still fails after fix. The end of gate.log:"
  echo
  echo '```'
  tail -n 60 "${dir}/gate.log"
  echo '```'
} > "${dir}/gaps.md.new"
mv "${dir}/gaps.md.new" "${dir}/gaps.md"
cat "${dir}/gaps.md"
```

`.decree/scripts/develop/ask_human.sh`, `ask_human`'s `ask` script. It prints what a person needs; a real one could post it to chat or an issue:

```bash
#!/usr/bin/env bash
# Every round is spent: show a person what is left, and how to answer.
set -euo pipefail
dir=${DECREE_ROOT_RUN_DIR}
echo "Every round is spent on: $(sed -n 's/^# //p' "${DECREE_MESSAGE}" | head -n 1)"
echo "Run folder: ${dir}"
echo "Still missing (gaps.md):"
cat "${dir}/gaps.md" 2>/dev/null || echo "(none written)"
echo "Answer with one of:"
for event in ${DECREE_EVENTS}; do
  [ "${event}" = error ] && continue
  echo "  decree event ${DECREE_WAIT_ID} ${event}"
done
```

Then `chmod +x .decree/scripts/gaps/fill.sh .decree/scripts/develop/gate_gaps.sh .decree/scripts/develop/ask_human.sh`, `decree check` and `decree graph`.

## Tuning it

- **More or fewer cheap rounds**: `max_gap_rounds`, per message with `params: { max_gap_rounds: 3 }`.
- **A different cheap agent**: `gaps`' `ai_cli` default. With OpenCode, set `OPENCODE_MODEL` in `.decree/.env` (for example `llama.cpp/qwen3`), and let `decree init --permissions` write `"external_directory": "deny"` into `opencode.json`.
- **Claude's model for its round**: `CLAUDE_MODEL`. A round that meets Claude's usage limit waits for the reset, up to `AI_LIMIT_WAIT`, without using its attempts.
- **No person**: point `claude_round`'s `unfilled` at `failed`. The run keeps its changes and holds the queue, as any failed run does.
