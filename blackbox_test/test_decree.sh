#!/usr/bin/env bash
# test_decree.sh — Black-box walkthrough of the decree 0.5 CLI binary
#
# Usage:
#   ./test_decree.sh           # run all tests
#   ./test_decree.sh -v        # verbose (show pass details)
#   ./test_decree.sh -f PAT    # filter tests by pattern
#
# Runs the built release binary (../target/release/decree, or $DECREE_BIN) as a
# user would: each test gets a fresh temp directory and a project from
# `decree init`, with no AI tool needed (a stub `claude` is first on PATH).
# It drives every command once: init, emit, process, status, tail, retry,
# event, check, graph, plus daemon.
#
# Dropped because tests/ covers them end to end with `assert_cmd`:
#   - each validation rule V1–V20 and M1–M3, one passing and one failing
#     fixture per rule (tests/check_test.rs)
#   - graph output byte for byte, including the mock (tests/graph_test.rs)
#   - migration rules 1–6 and invalid messages (tests/process_test.rs)
#   - reply delivery, rejection and timeouts (tests/reply_test.rs)
#   - onentry/onexit order, attempts and failed's onentry (tests/hooks_test.rs)
#   - SIGTERM, crashes, locks and retry after a kill (tests/interrupt_test.rs)
#   - emit parent, depth, max_depth and emits (tests/emit_test.rs)
#   - status and tail of a running script, tail into child runs, choose: model
#     through a router (tests/cli_test.rs)
#   - init's machines, routers, skill and permissions files, and the built-in
#     develop and rust_develop machines against 0.4.2's outcomes
#     (tests/integration_test.rs, tests/develop_test.rs)
set -uo pipefail

DECREE_BIN="${DECREE_BIN:-$(cd "$(dirname "$0")" && pwd)/../target/release/decree}"
VERBOSE=false
FILTER=""
PASSED=0
FAILED=0
SKIPPED=0
FAILURES=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    -v|--verbose) VERBOSE=true; shift ;;
    -f|--filter)  FILTER="$2"; shift 2 ;;
    *) echo "Unknown option: $1"; exit 2 ;;
  esac
done

if [[ ! -x "$DECREE_BIN" ]]; then
  echo "decree binary not found at $DECREE_BIN (run: cargo build --release)" >&2
  exit 2
fi

# ---------- helpers ----------

setup_tmpdir() {
  TEST_DIR="$(mktemp -d)"
  mkdir "$TEST_DIR/bin"
  cp "$DECREE_BIN" "$TEST_DIR/bin/decree"
  # Stub AI tool: decree init finds it, and nothing here calls a real model.
  printf '#!/usr/bin/env bash\necho "stub claude"\n' > "$TEST_DIR/bin/claude"
  chmod +x "$TEST_DIR/bin/decree" "$TEST_DIR/bin/claude"
  OLD_PATH="$PATH"
  export PATH="$TEST_DIR/bin:$PATH"
  cd "$TEST_DIR"
}

teardown_tmpdir() {
  cd /
  export PATH="$OLD_PATH"
  rm -rf "$TEST_DIR"
}

# A fresh project from `decree init`, plus a `hello` machine whose `greet`
# script prints the environment decree gives it.
init_project() {
  decree init --ai claude </dev/null >/dev/null 2>&1 || return 1
  machine hello <<'YAML'
# Graph: ../graph/hello.md
name: hello
description: Run one script.
data:
  who: { type: string, default: world }
initial: greet
states:
  greet:
    invoke: greet
    max_attempts: 3
    transitions: { done: done }
  done:   { final: true }
  failed: { final: true }
YAML
  script hello/greet <<'SH'
#!/usr/bin/env bash
echo "hello ${DECREE_DATA_WHO} from ${DECREE_MACHINE}/${DECREE_STATE} trigger=${DECREE_TRIGGER} attempt=${DECREE_ATTEMPT}"
SH
}

# machine NAME < yaml
machine() { cat > ".decree/machines/$1.yml"; }

# script [MACHINE/]NAME < body
script() {
  mkdir -p "$(dirname ".decree/scripts/$1")"
  cat > ".decree/scripts/$1.sh"
  chmod +x ".decree/scripts/$1.sh"
}

# The state mirrored in a run's message.md.
run_state() { sed -n 's/^state: //p' ".decree/runs/$1/message.md"; }

run_test() {
  local name="$1"
  shift

  if [[ -n "$FILTER" ]] && [[ "$name" != *"$FILTER"* ]]; then
    ((SKIPPED++))
    return 0
  fi

  setup_tmpdir
  local output
  local rc=0
  output=$("$@" 2>&1) || rc=$?

  if [[ $rc -eq 0 ]]; then
    ((PASSED++))
    if $VERBOSE; then
      echo "  PASS  $name"
    fi
  else
    ((FAILED++))
    FAILURES+=("$name")
    echo "  FAIL  $name"
    echo "$output" | head -10 | sed 's/^/        /'
  fi
  teardown_tmpdir
}

# Assertion helpers — each prints a message and returns 0/1
assert_eq() {
  local expected="$1" actual="$2" msg="${3:-}"
  if [[ "$expected" != "$actual" ]]; then
    echo "expected: '$expected', got: '$actual' ${msg:+($msg)}"
    return 1
  fi
}

assert_contains() {
  local haystack="$1" needle="$2" msg="${3:-}"
  if [[ "$haystack" != *"$needle"* ]]; then
    echo "expected to contain: '$needle' ${msg:+($msg)}"
    echo "actual: ${haystack:0:400}"
    return 1
  fi
}

assert_not_contains() {
  local haystack="$1" needle="$2" msg="${3:-}"
  if [[ "$haystack" == *"$needle"* ]]; then
    echo "expected NOT to contain: '$needle' ${msg:+($msg)}"
    return 1
  fi
}

assert_file_exists() {
  local path="$1" msg="${2:-}"
  if [[ ! -e "$path" ]]; then
    echo "file not found: '$path' ${msg:+($msg)}"
    return 1
  fi
}

assert_file_contains() {
  local path="$1" needle="$2" msg="${3:-}"
  if [[ ! -f "$path" ]]; then
    echo "file not found: '$path' ${msg:+($msg)}"
    return 1
  fi
  if ! grep -qF -- "$needle" "$path"; then
    echo "file '$path' does not contain: '$needle' ${msg:+($msg)}"
    return 1
  fi
}

assert_exit_code() {
  local expected="$1" actual="$2" msg="${3:-}"
  if [[ "$expected" != "$actual" ]]; then
    echo "expected exit code $expected, got $actual ${msg:+($msg)}"
    return 1
  fi
}

# ================================================================
# CLI basics
# ================================================================

test_version_flag() {
  local out
  out=$(decree --version) || return 1
  assert_contains "$out" "0.5."
}

test_help_lists_every_command() {
  local out cmd
  out=$(decree --help) || return 1
  for cmd in init process check graph emit event daemon status tail retry; do
    assert_contains "$out" "  $cmd " || return 1
  done
}

test_unknown_subcommand_exit_2() {
  local rc=0
  decree routine >/dev/null 2>&1 || rc=$?
  assert_exit_code 2 "$rc"
}

test_commands_without_project_fail() {
  local cmd rc
  for cmd in process check graph status; do
    rc=0
    decree "$cmd" >/dev/null 2>&1 || rc=$?
    [[ $rc -ne 0 ]] || { echo "decree $cmd succeeded without .decree/"; return 1; }
  done
}

# ================================================================
# init and check
# ================================================================

test_init_writes_the_layout() {
  decree init --ai claude </dev/null >/dev/null || return 1
  local p
  for p in .gitignore processed.md migrations inbox runs cron machines scripts graph \
    machines/develop.yml machines/rust_develop.yml machines/router.yml \
    scripts/router/ask_claude.sh graph/system.md; do
    assert_file_exists ".decree/$p" || return 1
  done
  assert_eq $'inbox/\nruns/' "$(cat .decree/.gitignore)" || return 1
  # 0.5 has no configuration file (docs/reference/README.md).
  [ "$(ls -A .decree | grep -c '\.yml$')" -eq 0 ] || return 1
}

test_init_refuses_an_existing_project() {
  decree init --ai claude </dev/null >/dev/null || return 1
  local rc=0
  decree init --ai claude </dev/null >/dev/null 2>&1 || rc=$?
  assert_exit_code 2 "$rc"
}

test_check_passes_on_init_output_without_warning() {
  decree init --ai claude </dev/null >/dev/null || return 1
  local out
  out=$(decree check 2>&1) || { echo "$out"; return 1; }
  assert_eq "" "$out"
}

test_check_reports_a_bad_machine() {
  init_project || return 1
  machine broken <<'YAML'
name: broken
description: Missing its failed state and pointing nowhere.
initial: start
states:
  start:
    transitions: { done: nowhere }
  done: { final: true }
YAML
  local out rc=0
  out=$(decree check 2>&1) || rc=$?
  assert_exit_code 1 "$rc" || return 1
  assert_contains "$out" "machines/broken.yml" || return 1
  assert_contains "$out" "nowhere"
}

# ================================================================
# graph
# ================================================================

test_graph_writes_a_file_per_machine() {
  init_project || return 1
  local out
  out=$(decree graph) || return 1
  assert_contains "$out" ".decree/graph/hello.md" || return 1
  assert_file_contains .decree/graph/hello.md "stateDiagram-v2" || return 1
  assert_file_contains .decree/graph/hello.md "greet --> done: done" || return 1
  assert_file_contains .decree/graph/system.md 'hello["hello"]'
}

test_check_warns_until_graph_is_rewritten() {
  init_project || return 1
  local out
  out=$(decree check 2>&1) || return 1
  assert_contains "$out" "graph" "a new machine makes the graph stale" || return 1
  decree graph >/dev/null || return 1
  out=$(decree check 2>&1) || return 1
  assert_eq "" "$out"
}

# ================================================================
# emit, process, status
# ================================================================

test_emit_process_status() {
  init_project || return 1
  local id out
  id=$(echo "# Say hello" | decree emit --machine hello --param who=decree) || return 1
  assert_file_exists ".decree/inbox/$id.md" || return 1
  decree process >/dev/null || return 1
  assert_eq done "$(run_state "$id")" || return 1
  assert_file_contains ".decree/runs/$id/0001-greet-greet.log" "hello decree from hello/greet trigger=emit attempt=1" || return 1
  assert_file_contains ".decree/runs/$id/events.jsonl" '"type":"transition"' || return 1
  out=$(decree status) || return 1
  assert_contains "$out" "$id" || return 1
  out=$(decree status "$id") || return 1
  assert_contains "$out" "greet"
}

test_emit_rejects_unknown_machine_and_param() {
  init_project || return 1
  local rc=0
  echo body | decree emit --machine nope >/dev/null 2>&1 || rc=$?
  assert_exit_code 1 "$rc" "unknown machine" || return 1
  rc=0
  echo body | decree emit --machine hello --param color=red >/dev/null 2>&1 || rc=$?
  assert_exit_code 1 "$rc" "unknown param"
}

test_process_dry_run_runs_nothing() {
  init_project || return 1
  local id out
  id=$(echo "# Say hello" | decree emit --machine hello) || return 1
  out=$(decree process --dry-run) || return 1
  assert_contains "$out" "$id" || return 1
  assert_file_exists ".decree/inbox/$id.md" || return 1
  [[ ! -e ".decree/runs/$id" ]] || { echo "dry run created a run"; return 1; }
}

test_migrations_run_once_in_order() {
  init_project || return 1
  printf -- '---\nmachine: hello\nparams:\n  who: one\n---\nFirst.\n' > .decree/migrations/01-first.md
  printf -- '---\nmachine: hello\nparams:\n  who: two\n---\nSecond.\n' > .decree/migrations/02-second.md
  decree process >/dev/null || return 1
  assert_eq $'01-first.md\n02-second.md' "$(cat .decree/processed.md)" || return 1
  assert_eq migration "$(sed -n 's/^trigger: //p' .decree/runs/01-first/message.md)" || return 1
  decree process >/dev/null || return 1
  assert_eq 2 "$(wc -l < .decree/processed.md | tr -d ' ')" "processed once"
}

test_routine_key_is_read_as_machine() {
  init_project || return 1
  printf -- '---\nroutine: hello\n---\nAn old migration.\n' > .decree/migrations/01-old.md
  decree process >/dev/null || return 1
  assert_eq done "$(run_state 01-old)"
}

test_script_printed_event_picks_the_transition() {
  init_project || return 1
  machine verify <<'YAML'
# Graph: ../graph/verify.md
name: verify
description: A script that prints its event.
initial: verify
states:
  verify:
    invoke: verify
    transitions: { pass: passed, fail: failed }
  passed: { final: true }
  failed: { final: true }
YAML
  script verify/verify <<'SH'
#!/usr/bin/env bash
echo "checking"
echo '{"event":"pass"}'
SH
  local id
  id=$(echo "# Verify" | decree emit --machine verify) || return 1
  decree process >/dev/null || return 1
  assert_eq passed "$(run_state "$id")"
}

# ================================================================
# failure, retry, tail
# ================================================================

test_failed_run_stops_process_and_retry_finishes_it() {
  init_project || return 1
  # greet fails until the flag file exists
  script hello/greet <<'SH'
#!/usr/bin/env bash
[ -f fixed ] || { echo "not fixed yet" >&2; exit 1; }
echo "fixed"
SH
  local id rc=0
  id=$(echo "# Say hello" | decree emit --machine hello) || return 1
  decree process >/dev/null 2>&1 || rc=$?
  assert_exit_code 1 "$rc" || return 1
  assert_eq failed "$(run_state "$id")" || return 1
  assert_file_contains ".decree/runs/$id/0003-greet-greet.log" "[stderr] not fixed yet" "three attempts" || return 1
  touch fixed
  decree retry "$id" >/dev/null || return 1
  decree process >/dev/null || return 1
  assert_eq done "$(run_state "$id")"
}

# tail follows live output only: a run that has stopped has nothing to follow.
test_tail_of_a_stopped_run_exits_0_and_no_run_exits_1() {
  init_project || return 1
  local rc=0 id
  decree tail >/dev/null 2>&1 || rc=$?
  assert_exit_code 1 "$rc" "no run" || return 1
  id=$(echo "# Say hello" | decree emit --machine hello) || return 1
  decree process >/dev/null || return 1
  decree tail "$id" >/dev/null
}

test_sigint_interrupts_and_retry_continues() {
  init_project || return 1
  script hello/greet <<'SH'
#!/usr/bin/env bash
[ -f fast ] || sleep 30
echo "greeted"
SH
  local id pid rc=0
  id=$(echo "# Say hello" | decree emit --machine hello) || return 1
  decree process >/dev/null 2>&1 &
  pid=$!
  for _ in $(seq 100); do
    [[ -f ".decree/runs/$id/.running" ]] && break
    sleep 0.1
  done
  kill -INT "$pid"
  wait "$pid" || rc=$?
  assert_exit_code 130 "$rc" || return 1
  assert_file_contains ".decree/runs/$id/events.jsonl" '"type":"interrupted"' || return 1
  assert_contains "$(decree status)" "interrupted: 1" || return 1
  touch fast
  decree retry "$id" >/dev/null || return 1
  decree process >/dev/null || return 1
  assert_eq done "$(run_state "$id")"
}

# ================================================================
# choose: person and event
# ================================================================

test_person_choice_waits_and_event_continues() {
  init_project || return 1
  machine approve <<'YAML'
# Graph: ../graph/approve.md
name: approve
description: Ask a person, then finish.
initial: approval
states:
  approval:
    invoke: { choose: person, question: "Ship it?", ask: ask }
    transitions:
      approve: { target: done, description: Ship it. }
      reject:  { target: rejected, description: Do not ship. }
  done:     { final: true }
  rejected: { final: true }
  failed:   { final: true }
YAML
  script ask <<'SH'
#!/usr/bin/env bash
echo "$DECREE_QUESTION wait=$DECREE_WAIT_ID"
SH
  local id out wait
  id=$(echo "# Ship" | decree emit --machine approve) || return 1
  out=$(decree process) || return 1
  assert_contains "$out" "decree event $id" || return 1
  assert_contains "$(decree status)" "waiting: 1" || return 1
  wait=$(grep -o "$id\.w[0-9]*" ".decree/runs/$id/0001-approval-ask.log" | head -n 1)
  [[ -n "$wait" ]] || { echo "ask script got no wait id"; return 1; }
  local rc=0
  decree event "$wait" maybe >/dev/null 2>&1 || rc=$?
  assert_exit_code 1 "$rc" "not an option" || return 1
  decree event "$wait" approve -m "Looks good" >/dev/null || return 1
  decree process >/dev/null || return 1
  assert_eq done "$(run_state "$id")" || return 1
  assert_file_contains ".decree/runs/$id/events.jsonl" '"source":"person"'
}

# ================================================================
# emit from a script, cron, daemon
# ================================================================

test_script_emits_a_follow_up() {
  init_project || return 1
  machine chain <<'YAML'
# Graph: ../graph/chain.md
name: chain
description: Queue a hello for later.
initial: hand_off
states:
  hand_off:
    invoke: hand_off
    emits: [hello]
    transitions: { done: done }
  done:   { final: true }
  failed: { final: true }
YAML
  script chain/hand_off <<'SH'
#!/usr/bin/env bash
set -euo pipefail
echo "# Follow-up" | decree emit --machine hello --param who=follow-up
SH
  local id child
  id=$(echo "# Start" | decree emit --machine chain) || return 1
  decree process >/dev/null || return 1
  child=$(grep -l "^parent: $id" .decree/runs/*/message.md | head -n 1)
  [[ -n "$child" ]] || { echo "no follow-up run"; return 1; }
  assert_file_contains "$child" "depth: 1" || return 1
  assert_file_contains "$child" "state: done"
}

test_status_cron_lists_cron_files() {
  init_project || return 1
  printf -- '---\ncron: "0 3 * * *"\nmachine: hello\n---\nNightly hello.\n' > .decree/cron/nightly.md
  decree check >/dev/null 2>&1 || { decree check; return 1; }
  local out
  out=$(decree status --cron) || return 1
  assert_contains "$out" "nightly"
}

test_daemon_drains_inbox_and_stops_on_sigterm() {
  init_project || return 1
  local id pid rc=0
  id=$(echo "# Say hello" | decree emit --machine hello) || return 1
  decree daemon --interval 1 >/dev/null 2>&1 &
  pid=$!
  for _ in $(seq 100); do
    [[ "$(run_state "$id" 2>/dev/null)" == done ]] && break
    sleep 0.1
  done
  kill -TERM "$pid"
  wait "$pid" || rc=$?
  assert_exit_code 0 "$rc" || return 1
  assert_eq done "$(run_state "$id")"
}

# ================================================================
# run
# ================================================================

echo "decree test suite"
echo "================="
echo ""

TESTS=(
  # CLI basics
  "version flag"                           test_version_flag
  "help lists every command"               test_help_lists_every_command
  "unknown subcommand exit 2"              test_unknown_subcommand_exit_2
  "commands w/o project fail"              test_commands_without_project_fail

  # init and check
  "init writes the layout"                 test_init_writes_the_layout
  "init refuses existing project"          test_init_refuses_an_existing_project
  "check passes on init output"            test_check_passes_on_init_output_without_warning
  "check reports a bad machine"            test_check_reports_a_bad_machine

  # graph
  "graph writes a file per machine"        test_graph_writes_a_file_per_machine
  "check warns until graph rewritten"      test_check_warns_until_graph_is_rewritten

  # emit, process, status
  "emit, process, status"                  test_emit_process_status
  "emit rejects bad machine and param"     test_emit_rejects_unknown_machine_and_param
  "process --dry-run runs nothing"         test_process_dry_run_runs_nothing
  "migrations run once in order"           test_migrations_run_once_in_order
  "routine: key read as machine:"          test_routine_key_is_read_as_machine
  "script event picks transition"          test_script_printed_event_picks_the_transition

  # failure, retry, tail
  "failed run, retry finishes it"          test_failed_run_stops_process_and_retry_finishes_it
  "tail stopped run / no run"              test_tail_of_a_stopped_run_exits_0_and_no_run_exits_1
  "SIGINT interrupts, retry continues"     test_sigint_interrupts_and_retry_continues

  # choose: person and event
  "person choice waits, event continues"   test_person_choice_waits_and_event_continues

  # emit from a script, cron, daemon
  "script emits a follow-up"               test_script_emits_a_follow_up
  "status --cron lists cron files"         test_status_cron_lists_cron_files
  "daemon drains inbox, stops on TERM"     test_daemon_drains_inbox_and_stops_on_sigterm
)

# Run tests in pairs (name, function)
i=0
while [[ $i -lt ${#TESTS[@]} ]]; do
  name="${TESTS[$i]}"
  func="${TESTS[$((i+1))]}"
  run_test "$name" "$func"
  i=$((i + 2))
done

# ---------- SUMMARY ----------

echo ""
echo "================="
echo "Results: $PASSED passed, $FAILED failed, $SKIPPED skipped"
echo ""

if [[ ${#FAILURES[@]} -gt 0 ]]; then
  echo "Failed tests:"
  for f in "${FAILURES[@]}"; do
    echo "  - $f"
  done
  echo ""
fi

if [[ $FAILED -gt 0 ]]; then
  exit 1
else
  echo "All tests passed."
  exit 0
fi
