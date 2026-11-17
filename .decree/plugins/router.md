---
name: router
machine:
  description: Ask an AI CLI (claude, opencode or copilot) to pick one of the options in the request.
  initial: ask
  states:
    ask:
      invoke:
        script:
          name: ask
          attempts: 2
      transitions:
        done: done
    done:
      final: true
    failed:
      final: true
scripts:
  ask.sh: |
    #!/usr/bin/env bash
    # router's only script. Reads the request decree wrote, asks an AI CLI, and
    # writes the reply. The CLI is, most specific first: the run's data entry `ai_cli`
    # (DECREE_DATA_AI_CLI), else AI_CLI (in .decree/.env, or the shell decree runs in),
    # else the first of claude, opencode and copilot found on PATH, as ai.sh picks it.
    # Exits non-zero (and so runs once more, attempts: 2) unless the reply names one of
    # the options. Replace this machine to use another model; decree validates the
    # reply again either way. Keep an edit with `decree plugin export router`.
    set -euo pipefail
    req="$DECREE_REQUEST"
    prompt=$(jq -r '
      "You choose the next step of a workflow. Reply with only a JSON object:\n" +
      "{\"event\": \"<one option name>\", \"reason\": \"<one sentence>\", \"confidence\": <0 to 1>}\n\n" +
      "Workflow: \(.machine): \(.machine_description)\n" +
      "Current step: \(.state): \(.state_description)\n" +
      "Question: \(.question)\n\n" +
      "Output this decision reads (empty if none):\n\(.input)\n\n" +
      "Task message:\n\(.message_body)\n\n" +
      "Run so far:\n\(.history | map("- " + .) | join("\n"))\n\n" +
      "Options:\n\(.options | map("- \(.event): \(.description)") | join("\n"))"' "$req")
    cli=${DECREE_DATA_AI_CLI:-${AI_CLI:-}}
    if [ -z "$cli" ]; then
      for cli in claude opencode copilot; do
        command -v "$cli" >/dev/null && break
        cli=
      done
    fi
    echo "$prompt"
    echo "--- reply ($cli)"
    case $cli in
      claude) reply=$(printf '%s' "$prompt" | claude -p) ;;
      opencode) reply=$(opencode run "$prompt") ;;
      copilot) reply=$(copilot -p "$prompt") ;;
      "") echo "no AI CLI: set AI_CLI (in .decree/.env) or put claude, opencode or copilot on PATH" >&2; exit 2 ;;
      *) echo "router knows no AI CLI \`$cli\` (AI_CLI): use claude, opencode or copilot, or edit this script (and keep it with \`decree plugin export router\`)" >&2; exit 2 ;;
    esac
    echo "$reply"
    # the last JSON object in the reply, fences and prose around it allowed: the last
    # fenced block (which may span lines) if there is one, else the last line holding one
    fenced=$(printf '%s\n' "$reply" | awk '
      /^[[:space:]]*```/ { if (inside) { last = block; inside = 0 } else { inside = 1; block = "" }; next }
      inside { block = block $0 " " }
      END { printf "%s", last }')
    json=$(printf '%s\n' "${fenced:-$reply}" | grep -o '{.*}' | tail -n 1 || true)
    json=$(jq -c . <<<"$json" 2>/dev/null) || { echo "no JSON object in the reply" >&2; exit 1; }
    event=$(jq -r '.event | strings' <<<"$json")
    jq -e --arg e "$event" '.options | map(.event) | index($e)' "$req" > /dev/null \
      || { echo "not an option: $event" >&2; exit 1; }
    printf '%s\n' "$json" > "$DECREE_REPLY"
    # for the log only: decree takes the event from the reply
    echo "picked $event"
---
# router

The default router: a `model` state with no `router:` asks this machine to pick one of its options. It renders the request decree wrote into a prompt, asks an AI CLI, and writes the reply, which decree validates.

## What it needs

- `jq`, and an AI CLI on `PATH`: the run's `ai_cli` data, else `AI_CLI` (in `.decree/.env`), else the first of `claude`, `opencode` and `copilot` found.

## How to use it

Add it, then write a state that asks a question:

```yaml
  review:
    invoke:
      model: { question: Is this change ready to merge? }
    transitions:
      ready: { target: merge, description: The change is complete and tested. }
      not_ready: { target: fix, description: Something is missing or broken. }
```

A reply that is not one of the options fails the script, which runs once more (`attempts: 2`). To use another model, edit `.decree/scripts/router/ask.sh`, then keep the edit with `decree plugin export router`.
