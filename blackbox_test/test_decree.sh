#!/usr/bin/env bash
# test_decree.sh — Black-box test suite for the decree CLI binary
#
# Usage:
#   ./test_decree.sh           # run all tests
#   ./test_decree.sh -v        # verbose (show pass details)
#   ./test_decree.sh -f PAT    # filter tests by pattern
#
# Each test runs in an isolated temp directory with a fresh ./decree binary.
set -uo pipefail

DECREE_BIN="$(cd "$(dirname "$0")" && pwd)/decree"
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

# ---------- helpers ----------

setup_tmpdir() {
  TEST_DIR="$(mktemp -d)"
  cp "$DECREE_BIN" "$TEST_DIR/decree"
  chmod +x "$TEST_DIR/decree"
  cd "$TEST_DIR"
}

teardown_tmpdir() {
  cd /
  rm -rf "$TEST_DIR"
}

# Initialize a decree project non-interactively (no TTY -> auto-detect, auto-accept)
init_project() {
  cd "$TEST_DIR"
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
}

# Initialize and clear hooks for clean processing tests
init_project_no_hooks() {
  init_project
  sed -i 's/beforeEach:.*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach:.*/afterEach: ""/' .decree/config.yml
  sed -i 's/beforeAll:.*/beforeAll: ""/' .decree/config.yml
  sed -i 's/afterAll:.*/afterAll: ""/' .decree/config.yml
}

# Create a simple routine that succeeds and echoes env vars
create_echo_routine() {
  local name="${1:-develop}"
  cat > ".decree/routines/${name}.sh" <<'SCRIPT'
#!/usr/bin/env bash
# Echo Routine
#
# Simple echo routine for testing.
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
my_custom="${my_custom:-}"
echo "FILE=$message_file"
echo "ID=$message_id"
echo "DIR=$message_dir"
echo "CHAIN=$chain"
echo "SEQ=$seq"
echo "CUSTOM=$my_custom"
SCRIPT
  chmod +x ".decree/routines/${name}.sh"
}

# Create a routine that always fails
create_failing_routine() {
  local name="${1:-develop}"
  cat > ".decree/routines/${name}.sh" <<'SCRIPT'
#!/usr/bin/env bash
# Failing Routine
#
# Always fails for testing.
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "failing attempt"
exit 1
SCRIPT
  chmod +x ".decree/routines/${name}.sh"
}

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
    echo "        $output" | head -5
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
    echo "actual: ${haystack:0:200}"
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

assert_dir_exists() {
  local path="$1" msg="${2:-}"
  if [[ ! -d "$path" ]]; then
    echo "directory not found: '$path' ${msg:+($msg)}"
    return 1
  fi
}

assert_file_contains() {
  local path="$1" needle="$2" msg="${3:-}"
  if [[ ! -f "$path" ]]; then
    echo "file not found: '$path' ${msg:+($msg)}"
    return 1
  fi
  if ! grep -qF "$needle" "$path"; then
    echo "file '$path' does not contain: '$needle' ${msg:+($msg)}"
    return 1
  fi
}

assert_file_not_contains() {
  local path="$1" needle="$2" msg="${3:-}"
  if [[ ! -f "$path" ]]; then
    return 0
  fi
  if grep -qF "$needle" "$path"; then
    echo "file '$path' should not contain: '$needle' ${msg:+($msg)}"
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

assert_executable() {
  local path="$1" msg="${2:-}"
  if [[ ! -x "$path" ]]; then
    echo "file not executable: '$path' ${msg:+($msg)}"
    return 1
  fi
}

assert_match() {
  local pattern="$1" actual="$2" msg="${3:-}"
  if [[ ! "$actual" =~ $pattern ]]; then
    echo "expected to match: '$pattern' ${msg:+($msg)}"
    echo "actual: ${actual:0:200}"
    return 1
  fi
}

# ================================================================
#                         TEST FUNCTIONS
# ================================================================

# ---------- 01: VERSION AND BASIC CLI ----------

test_version_flag() {
  local out
  out=$(./decree --version 2>&1)
  assert_match '^decree [0-9]+\.[0-9]+\.[0-9]+$' "$out" "version should be semver"
}

test_version_short_flag() {
  local out
  out=$(./decree -v 2>&1)
  assert_match '^decree [0-9]+\.[0-9]+\.[0-9]+$' "$out" "version should be semver"
}

test_help_flag() {
  local out rc=0
  out=$(./decree --help 2>&1) || rc=$?
  assert_exit_code 0 "$rc" &&
  assert_contains "$out" "Usage:" &&
  assert_contains "$out" "Commands:"
}

test_help_subcommand() {
  local out rc=0
  out=$(./decree help 2>&1) || rc=$?
  assert_exit_code 0 "$rc" &&
  assert_contains "$out" "Core workflow:" &&
  assert_contains "$out" "decree process" &&
  assert_contains "$out" "Message Format:" &&
  assert_contains "$out" "Processing Pipeline:" &&
  assert_contains "$out" "Defining Routines:" &&
  assert_contains "$out" "Lifecycle Hooks" &&
  assert_contains "$out" "Cron Scheduling:" &&
  assert_contains "$out" "Getting Started:"
}

test_help_subcommand_verbose() {
  local out
  out=$(./decree help 2>&1)
  local lines
  lines=$(echo "$out" | wc -l)
  [[ $lines -gt 50 ]] || {
    echo "expected verbose help (>50 lines), got $lines lines"
    return 1
  }
}

test_help_mentions_routine_sync() {
  local out
  out=$(./decree help 2>&1)
  assert_contains "$out" "routine-sync"
}

test_help_mentions_routine_registry() {
  local out
  out=$(./decree help 2>&1)
  assert_contains "$out" "Routine Registry"
}

test_unknown_subcommand_exit_2() {
  local rc=0
  ./decree nonexistent </dev/null >/dev/null 2>&1 || rc=$?
  assert_exit_code 2 "$rc"
}

test_no_color_flag_accepted() {
  local rc=0
  ./decree --no-color --version 2>&1 || rc=$?
  assert_exit_code 0 "$rc"
}

test_no_color_env_var() {
  local rc=0
  NO_COLOR=1 ./decree --version 2>&1 || rc=$?
  assert_exit_code 0 "$rc"
}

test_version_no_project_needed() {
  # --version should work without a .decree/ directory
  local rc=0
  ./decree --version 2>&1 || rc=$?
  assert_exit_code 0 "$rc"
}

test_help_no_project_needed() {
  local rc=0
  ./decree help 2>&1 || rc=$?
  assert_exit_code 0 "$rc"
}

# ---------- 02: REQUIRES .decree/ DIRECTORY ----------

test_bare_decree_without_project_fails() {
  local rc=0
  ./decree </dev/null >/dev/null 2>&1 || rc=$?
  [[ $rc -ne 0 ]] || {
    echo "expected failure without .decree/, got exit 0"
    return 1
  }
}

test_process_without_project_fails() {
  local out rc=0
  out=$(./decree process </dev/null 2>&1) || rc=$?
  [[ $rc -ne 0 ]] || {
    echo "expected failure without .decree/, got exit 0"
    return 1
  }
  assert_contains "$out" "decree init"
}

test_status_without_project_fails() {
  local rc=0
  ./decree status </dev/null >/dev/null 2>&1 || rc=$?
  [[ $rc -ne 0 ]] || {
    echo "expected failure without .decree/, got exit 0"
    return 1
  }
}

test_routine_without_project_fails() {
  local rc=0
  ./decree routine </dev/null >/dev/null 2>&1 || rc=$?
  [[ $rc -ne 0 ]] || {
    echo "expected failure without .decree/, got exit 0"
    return 1
  }
}

test_verify_without_project_fails() {
  local rc=0
  ./decree verify </dev/null >/dev/null 2>&1 || rc=$?
  [[ $rc -ne 0 ]] || {
    echo "expected failure without .decree/, got exit 0"
    return 1
  }
}

test_log_without_project_fails() {
  local rc=0
  ./decree log </dev/null >/dev/null 2>&1 || rc=$?
  [[ $rc -ne 0 ]] || {
    echo "expected failure without .decree/, got exit 0"
    return 1
  }
}

test_daemon_without_project_fails() {
  local rc=0
  timeout 2 ./decree daemon </dev/null >/dev/null 2>&1 || rc=$?
  [[ $rc -ne 0 ]] || {
    echo "expected failure without .decree/, got exit 0"
    return 1
  }
}

test_prompt_without_project_fails() {
  local rc=0
  ./decree prompt </dev/null >/dev/null 2>&1 || rc=$?
  [[ $rc -ne 0 ]] || {
    echo "expected failure without .decree/, got exit 0"
    return 1
  }
}

test_routine_sync_without_project_fails() {
  local rc=0
  ./decree routine-sync </dev/null >/dev/null 2>&1 || rc=$?
  [[ $rc -ne 0 ]] || {
    echo "expected failure without .decree/, got exit 0"
    return 1
  }
}

# ---------- 03: INIT — DIRECTORY STRUCTURE ----------

test_init_creates_decree_dir() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_dir_exists .decree
}

test_init_creates_routines_dir() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_dir_exists .decree/routines
}

test_init_creates_prompts_dir() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_dir_exists .decree/prompts
}

test_init_creates_cron_dir() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_dir_exists .decree/cron
}

test_init_creates_inbox_dir() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_dir_exists .decree/inbox
}

test_init_creates_inbox_dead_dir() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_dir_exists .decree/inbox/dead
}

test_init_creates_outbox_dir() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_dir_exists .decree/outbox
}

test_init_creates_outbox_dead_dir() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_dir_exists .decree/outbox/dead
}

test_init_creates_runs_dir() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_dir_exists .decree/runs
}

test_init_creates_migrations_dir() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_dir_exists .decree/migrations
}

test_init_creates_config_yml() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_exists .decree/config.yml
}

test_init_creates_gitignore() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_exists .decree/.gitignore
}

test_init_creates_processed_md() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_exists .decree/processed.md
}

test_init_processed_md_empty() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  local content
  content=$(cat .decree/processed.md)
  assert_eq "" "$content" "processed.md should be empty on init"
}

test_init_creates_router_md() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_exists .decree/router.md
}

test_init_router_not_in_prompts() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  if [[ -f .decree/prompts/router.md ]]; then
    echo "router.md should NOT be in prompts/"
    return 1
  fi
}

# ---------- 04: INIT — CONFIG FILE ----------

test_init_config_no_ai_command() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_not_contains .decree/config.yml "ai_command:"
}

test_init_config_has_ai_router() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_contains .decree/config.yml "ai_router:"
}

test_init_config_has_ai_interactive() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_contains .decree/config.yml "ai_interactive:"
}

test_init_config_max_retries_3() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_contains .decree/config.yml "max_retries: 3"
}

test_init_config_max_depth_10() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_contains .decree/config.yml "max_depth: 10"
}

test_init_config_max_log_size() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_contains .decree/config.yml "max_log_size: 2097152"
}

test_init_config_default_routine_develop() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_contains .decree/config.yml "default_routine: develop"
}

test_init_config_has_hooks_section() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_contains .decree/config.yml "hooks:" &&
  assert_file_contains .decree/config.yml "beforeAll:" &&
  assert_file_contains .decree/config.yml "afterAll:" &&
  assert_file_contains .decree/config.yml "beforeEach:" &&
  assert_file_contains .decree/config.yml "afterEach:"
}

test_init_config_has_routines_section() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_contains .decree/config.yml "routines:" &&
  assert_file_contains .decree/config.yml "develop:" &&
  assert_file_contains .decree/config.yml "enabled: true"
}

test_init_config_commented_alternatives() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  local content
  content=$(cat .decree/config.yml)
  assert_contains "$content" "#" "should have comments for alternative backends"
}

test_init_config_routine_source_commented() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_contains .decree/config.yml "routine_source"
}

# ---------- 05: INIT — TEMPLATES ----------

test_init_creates_develop_sh() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_exists .decree/routines/develop.sh
}

test_init_creates_rust_develop_sh() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_exists .decree/routines/rust-develop.sh
}

test_init_routines_executable() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_executable .decree/routines/develop.sh &&
  assert_executable .decree/routines/rust-develop.sh
}

test_init_develop_has_shebang() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  local first_line
  first_line=$(head -1 .decree/routines/develop.sh)
  assert_eq "#!/usr/bin/env bash" "$first_line"
}

test_init_develop_has_precheck() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_contains .decree/routines/develop.sh "DECREE_PRE_CHECK"
}

test_init_develop_has_description() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  local line2
  line2=$(sed -n '2p' .decree/routines/develop.sh)
  assert_contains "$line2" "#"
}

test_init_develop_no_ai_placeholder() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  # Placeholders like {AI_CMD}, {ai_name}, {ai_invoke} should be replaced
  if grep -qE '\{AI_CMD\}|\{ai_name\}|\{ai_invoke\}' .decree/routines/develop.sh; then
    echo "AI placeholder was not replaced"
    return 1
  fi
}

test_init_rust_develop_no_ai_placeholder() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  if grep -qE '\{AI_CMD\}|\{ai_name\}|\{ai_invoke\}' .decree/routines/rust-develop.sh; then
    echo "AI placeholder was not replaced"
    return 1
  fi
}

test_init_develop_has_message_dir_ref() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_contains .decree/routines/develop.sh 'message_dir'
}

test_init_develop_references_ai_tool() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  # Routine should reference at least one known AI tool
  local content
  content=$(cat .decree/routines/develop.sh)
  if [[ "$content" != *"opencode"* ]] && [[ "$content" != *"claude"* ]] && [[ "$content" != *"copilot"* ]]; then
    echo "develop.sh should reference a detected AI tool"
    return 1
  fi
}

test_init_creates_prompt_templates() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_exists .decree/prompts/migration.md &&
  assert_file_exists .decree/prompts/sow.md &&
  assert_file_exists .decree/prompts/routine.md
}

test_init_migration_prompt_has_placeholders() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_contains .decree/prompts/migration.md "{migrations}" &&
  assert_file_contains .decree/prompts/migration.md "{processed}"
}

test_init_routine_prompt_has_placeholder() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_contains .decree/prompts/routine.md "{routines}"
}

test_init_router_has_placeholders() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_contains .decree/router.md "{routines}" &&
  assert_file_contains .decree/router.md "{message}"
}

# ---------- 06: INIT — GITIGNORE ----------

test_init_gitignore_inbox() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_contains .decree/.gitignore "inbox/"
}

test_init_gitignore_outbox() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_contains .decree/.gitignore "outbox/"
}

test_init_gitignore_runs() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_contains .decree/.gitignore "runs/"
}

# ---------- 07: INIT — GIT HOOKS ----------

test_init_git_creates_hook_routines() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_exists .decree/routines/git-baseline.sh &&
  assert_file_exists .decree/routines/git-stash-changes.sh
}

test_init_git_hooks_executable() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_executable .decree/routines/git-baseline.sh &&
  assert_executable .decree/routines/git-stash-changes.sh
}

test_init_git_baseline_has_precheck() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_contains .decree/routines/git-baseline.sh "DECREE_PRE_CHECK"
}

test_init_git_stash_has_precheck() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_contains .decree/routines/git-stash-changes.sh "DECREE_PRE_CHECK"
}

test_init_git_baseline_has_hook_env() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_contains .decree/routines/git-baseline.sh "DECREE_ATTEMPT"
}

test_init_git_stash_has_hook_env() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_contains .decree/routines/git-stash-changes.sh "DECREE_ROUTINE_EXIT_CODE"
}

# ---------- 08: INIT — RE-RUN BEHAVIOR ----------

test_init_rerun_overwrites_non_tty() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  echo "# modified" >> .decree/config.yml
  ./decree init </dev/null >/dev/null 2>&1
  if grep -qF "# modified" .decree/config.yml; then
    echo "re-run should overwrite in non-TTY mode"
    return 1
  fi
}

test_init_rerun_prints_warning() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  local out
  out=$(./decree init </dev/null 2>&1)
  assert_contains "$out" "already" || assert_contains "$out" "Overwriting"
}

# ---------- 09: STATUS ----------

test_status_empty_project() {
  init_project
  local out rc=0
  out=$(./decree status --no-color 2>&1) || rc=$?
  assert_exit_code 0 "$rc" &&
  assert_contains "$out" "Migrations:"
}

test_status_shows_migration_count() {
  init_project
  mkdir -p .decree/migrations
  echo "Do task A" > .decree/migrations/01-task-a.md
  echo "Do task B" > .decree/migrations/02-task-b.md
  echo "01-task-a.md" > .decree/processed.md
  local out
  out=$(./decree status --no-color 2>&1)
  assert_contains "$out" "1" &&
  assert_contains "$out" "2"
}

test_status_shows_next_migration() {
  init_project
  mkdir -p .decree/migrations
  echo "Do task A" > .decree/migrations/01-task-a.md
  echo "Do task B" > .decree/migrations/02-task-b.md
  echo "01-task-a.md" > .decree/processed.md
  local out
  out=$(./decree status --no-color 2>&1)
  assert_contains "$out" "02-task-b.md"
}

test_status_shows_inbox_count() {
  init_project
  mkdir -p .decree/inbox
  cat > .decree/inbox/D0001-1432-test-0.md <<'EOF'
---
id: D0001-1432-test-0
chain: D0001-1432-test
seq: 0
routine: develop
---
Test message.
EOF
  local out
  out=$(./decree status --no-color 2>&1)
  assert_contains "$out" "Inbox:"
}

test_status_shows_dead_letter_count() {
  init_project
  mkdir -p .decree/inbox/dead
  echo "dead msg" > .decree/inbox/dead/D0001-1432-test-0.md
  local out
  out=$(./decree status --no-color 2>&1)
  assert_contains "$out" "Dead" || assert_contains "$out" "dead"
}

test_status_shows_recent_activity() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  ./decree process --no-color >/dev/null 2>&1
  local out
  out=$(./decree status --no-color 2>&1)
  assert_contains "$out" "Recent Activity" &&
  assert_contains "$out" "01-test"
}

test_status_no_color_flag() {
  init_project
  local rc=0
  ./decree status --no-color >/dev/null 2>&1 || rc=$?
  assert_exit_code 0 "$rc"
}

# ---------- 10: LOG ----------

test_log_no_runs() {
  init_project
  local out rc=0
  out=$(./decree log --no-color 2>&1) || rc=$?
  assert_contains "$out" "No runs" || assert_contains "$out" "no runs" || assert_contains "$out" "No log"
}

test_log_shows_recent_run() {
  init_project
  mkdir -p .decree/runs/D0001-1432-test-0
  echo "test log output" > .decree/runs/D0001-1432-test-0/routine.log
  cat > .decree/runs/D0001-1432-test-0/message.md <<'EOF'
---
id: D0001-1432-test-0
chain: D0001-1432-test
seq: 0
routine: develop
---
Test.
EOF
  local out
  out=$(./decree log --no-color 2>&1)
  assert_contains "$out" "test log output"
}

test_log_specific_run() {
  init_project
  mkdir -p .decree/runs/D0001-1432-test-0
  echo "specific log content" > .decree/runs/D0001-1432-test-0/routine.log
  cat > .decree/runs/D0001-1432-test-0/message.md <<'EOF'
---
id: D0001-1432-test-0
chain: D0001-1432-test
seq: 0
routine: develop
---
Test.
EOF
  local out
  out=$(./decree log --no-color D0001-1432-test-0 2>&1)
  assert_contains "$out" "specific log content"
}

test_log_prefix_match() {
  init_project
  mkdir -p .decree/runs/D0001-1432-test-0
  echo "prefix match content" > .decree/runs/D0001-1432-test-0/routine.log
  cat > .decree/runs/D0001-1432-test-0/message.md <<'EOF'
---
id: D0001-1432-test-0
chain: D0001-1432-test
seq: 0
routine: develop
---
Test.
EOF
  local out
  out=$(./decree log --no-color D0001 2>&1)
  assert_contains "$out" "prefix match content"
}

test_log_multiple_attempts() {
  init_project
  mkdir -p .decree/runs/D0001-1432-test-0
  echo "attempt 1 output" > .decree/runs/D0001-1432-test-0/routine.log
  echo "attempt 2 output" > .decree/runs/D0001-1432-test-0/routine-2.log
  cat > .decree/runs/D0001-1432-test-0/message.md <<'EOF'
---
id: D0001-1432-test-0
chain: D0001-1432-test
seq: 0
routine: develop
---
Test.
EOF
  local out
  out=$(./decree log --no-color D0001-1432-test-0 2>&1)
  assert_contains "$out" "attempt 1 output" &&
  assert_contains "$out" "attempt 2 output"
}

test_log_nonexistent_run_fails() {
  init_project
  local rc=0
  ./decree log --no-color nonexistent >/dev/null 2>&1 || rc=$?
  [[ $rc -ne 0 ]] || {
    echo "expected error for nonexistent message ID"
    return 1
  }
}

test_log_shows_timestamps() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  ./decree process --no-color >/dev/null 2>&1
  local out
  out=$(./decree log --no-color 2>&1)
  assert_contains "$out" "[decree] start" &&
  assert_contains "$out" "[decree] duration"
}

# ---------- 11: ROUTINE LIST AND DETAIL ----------

test_routine_list_non_tty() {
  init_project
  local out rc=0
  out=$(./decree routine --no-color 2>&1) || rc=$?
  assert_exit_code 0 "$rc" &&
  assert_contains "$out" "develop"
}

test_routine_list_shows_descriptions() {
  init_project
  local out
  out=$(./decree routine --no-color 2>&1)
  # Should show descriptions alongside names
  assert_contains "$out" "develop" &&
  assert_contains "$out" "routine"
}

test_routine_list_shows_rust_develop() {
  init_project
  local out
  out=$(./decree routine --no-color 2>&1)
  assert_contains "$out" "rust-develop"
}

test_routine_list_shows_git_hooks() {
  init_project
  local out
  out=$(./decree routine --no-color 2>&1)
  assert_contains "$out" "git-baseline" &&
  assert_contains "$out" "git-stash-changes"
}

test_routine_detail_named() {
  init_project
  local out rc=0
  out=$(./decree routine develop --no-color 2>&1) || rc=$?
  assert_exit_code 0 "$rc" &&
  assert_contains "$out" "develop"
}

test_routine_detail_shows_path() {
  init_project
  local out
  out=$(./decree routine develop --no-color 2>&1)
  assert_contains "$out" ".decree/routines/develop.sh"
}

test_routine_detail_shows_description() {
  init_project
  local out
  out=$(./decree routine develop --no-color 2>&1)
  local lines
  lines=$(echo "$out" | wc -l)
  [[ $lines -gt 1 ]] || {
    echo "expected multi-line detail, got $lines lines"
    return 1
  }
}

test_routine_unknown_fails() {
  init_project
  local out rc=0
  out=$(./decree routine zzzzzzzzz --no-color 2>&1) || rc=$?
  [[ $rc -ne 0 ]] || {
    echo "expected error for unknown routine"
    return 1
  }
  assert_contains "$out" "unknown routine" || assert_contains "$out" "Available"
}

test_routine_no_routines() {
  init_project
  rm -rf .decree/routines/*
  # Clear routines from config so they don't show as deprecated
  sed -i '/^routines:/,/^[^ ]/{/^routines:/!{/^[^ ]/!d}}' .decree/config.yml
  sed -i '/^routines:$/d' .decree/config.yml
  local out
  out=$(./decree routine --no-color 2>&1)
  assert_contains "$out" "No routines" || assert_contains "$out" "no routines"
}

test_routine_nested() {
  init_project
  mkdir -p .decree/routines/deploy
  cat > .decree/routines/deploy/staging.sh <<'SCRIPT'
#!/usr/bin/env bash
# Deploy Staging
#
# Deploy to the staging environment.
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "deploying to staging"
SCRIPT
  chmod +x .decree/routines/deploy/staging.sh
  # Sync to discover nested routine
  ./decree routine-sync --no-color >/dev/null 2>&1
  local out
  out=$(./decree routine --no-color 2>&1)
  assert_contains "$out" "deploy/staging"
}

# ---------- 12: VERIFY ----------

test_verify_all_pass() {
  init_project
  cat > .decree/routines/simple.sh <<'SCRIPT'
#!/usr/bin/env bash
# Simple
#
# A simple routine.
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "ok"
SCRIPT
  chmod +x .decree/routines/simple.sh
  rm -f .decree/routines/develop.sh .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach:.*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach:.*/afterEach: ""/' .decree/config.yml
  local out rc=0
  out=$(./decree verify --no-color 2>&1) || rc=$?
  assert_exit_code 0 "$rc" &&
  assert_contains "$out" "PASS"
}

test_verify_some_fail_exit_3() {
  init_project
  cat > .decree/routines/failing.sh <<'SCRIPT'
#!/usr/bin/env bash
# Failing
#
# A routine that fails pre-check.
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    echo "missing_tool not found" >&2
    exit 1
fi
echo "ok"
SCRIPT
  chmod +x .decree/routines/failing.sh
  rm -f .decree/routines/develop.sh .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  local rc=0
  ./decree verify --no-color >/dev/null 2>&1 || rc=$?
  assert_exit_code 3 "$rc"
}

test_verify_shows_fail_reason() {
  init_project
  cat > .decree/routines/failing.sh <<'SCRIPT'
#!/usr/bin/env bash
# Failing
#
# Fails pre-check.
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    echo "xyzzy_tool not found" >&2
    exit 1
fi
SCRIPT
  chmod +x .decree/routines/failing.sh
  rm -f .decree/routines/develop.sh .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  local out
  out=$(./decree verify --no-color 2>&1)
  assert_contains "$out" "FAIL" &&
  assert_contains "$out" "xyzzy_tool not found"
}

test_verify_reports_count() {
  init_project
  cat > .decree/routines/good.sh <<'SCRIPT'
#!/usr/bin/env bash
# Good
#
# Passes.
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
SCRIPT
  chmod +x .decree/routines/good.sh
  cat > .decree/routines/bad.sh <<'SCRIPT'
#!/usr/bin/env bash
# Bad
#
# Fails.
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 1; fi
SCRIPT
  chmod +x .decree/routines/bad.sh
  rm -f .decree/routines/develop.sh .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach:.*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach:.*/afterEach: ""/' .decree/config.yml
  local out
  out=$(./decree verify --no-color 2>&1)
  assert_contains "$out" "1 of 2" || assert_contains "$out" "routines ready"
}

test_verify_no_routines() {
  init_project
  rm -rf .decree/routines/*
  sed -i 's/beforeEach:.*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach:.*/afterEach: ""/' .decree/config.yml
  local out
  out=$(./decree verify --no-color 2>&1)
  assert_contains "$out" "No routines" || assert_contains "$out" "no routines" || assert_contains "$out" "0"
}

test_verify_includes_hook_prechecks() {
  init_project
  cat > .decree/routines/my-hook.sh <<'SCRIPT'
#!/usr/bin/env bash
# My Hook
#
# A hook routine.
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
SCRIPT
  chmod +x .decree/routines/my-hook.sh
  sed -i 's/beforeAll:.*/beforeAll: "my-hook"/' .decree/config.yml
  sed -i 's/beforeEach:.*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach:.*/afterEach: ""/' .decree/config.yml
  rm -f .decree/routines/develop.sh .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  local out rc=0
  out=$(./decree verify --no-color 2>&1) || rc=$?
  assert_exit_code 0 "$rc" &&
  assert_contains "$out" "Hook pre-checks" &&
  assert_contains "$out" "my-hook" &&
  assert_contains "$out" "PASS"
}

# ---------- 13: PROCESS — DRY RUN ----------

test_process_dry_run_no_migrations() {
  init_project
  local out rc=0
  out=$(./decree process --dry-run --no-color 2>&1) || rc=$?
  assert_contains "$out" "No" || assert_contains "$out" "no" || assert_contains "$out" "0"
}

test_process_dry_run_lists_migrations() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  cat > .decree/migrations/01-add-auth.md <<'EOF'
---
routine: develop
---
Add authentication module.
EOF
  local out
  out=$(./decree process --dry-run --no-color 2>&1)
  assert_contains "$out" "01-add-auth.md"
}

test_process_dry_run_shows_routine() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  cat > .decree/migrations/01-test.md <<'EOF'
---
routine: develop
---
Test.
EOF
  local out
  out=$(./decree process --dry-run --no-color 2>&1)
  assert_contains "$out" "develop"
}

test_process_dry_run_shows_precheck_status() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  local out
  out=$(./decree process --dry-run --no-color 2>&1)
  assert_contains "$out" "PASS"
}

test_process_dry_run_no_files_created() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "Do auth" > .decree/migrations/01-add-auth.md
  ./decree process --dry-run --no-color >/dev/null 2>&1
  local inbox_count
  inbox_count=$(find .decree/inbox -maxdepth 1 -name '*.md' 2>/dev/null | wc -l)
  assert_eq "0" "$inbox_count" "dry-run should not create inbox messages"
  local processed
  processed=$(cat .decree/processed.md)
  assert_eq "" "$processed" "dry-run should not mark processed"
}

# ---------- 14: PROCESS — BASIC EXECUTION ----------

test_process_simple_migration() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  cat > .decree/migrations/01-test.md <<'EOF'
---
routine: develop
---
Run a simple test.
EOF
  local out rc=0
  out=$(./decree process --no-color 2>&1) || rc=$?
  assert_exit_code 0 "$rc" "process should succeed" &&
  assert_file_contains .decree/processed.md "01-test.md" "migration should be marked processed"
}

test_process_creates_run_directory() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  ./decree process --no-color >/dev/null 2>&1
  local run_count
  run_count=$(find .decree/runs -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l)
  [[ $run_count -gt 0 ]] || {
    echo "expected at least one run directory, found $run_count"
    return 1
  }
}

test_process_run_dir_has_message() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  ./decree process --no-color >/dev/null 2>&1
  local msg_file
  msg_file=$(find .decree/runs -name 'message.md' 2>/dev/null | head -1)
  [[ -n "$msg_file" ]] || {
    echo "expected message.md in run directory"
    return 1
  }
}

test_process_run_dir_has_log() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  ./decree process --no-color >/dev/null 2>&1
  local log_file
  log_file=$(find .decree/runs -name 'routine.log' 2>/dev/null | head -1)
  [[ -n "$log_file" ]] || {
    echo "expected routine.log in run directory"
    return 1
  }
}

test_process_log_has_timestamps() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  ./decree process --no-color >/dev/null 2>&1
  local log_file
  log_file=$(find .decree/runs -name 'routine.log' 2>/dev/null | head -1)
  if [[ -n "$log_file" ]]; then
    assert_file_contains "$log_file" "[decree] start" &&
    assert_file_contains "$log_file" "[decree] duration"
  else
    echo "no log file found"
    return 1
  fi
}

test_process_removes_from_inbox() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  ./decree process --no-color >/dev/null 2>&1
  local inbox_count
  inbox_count=$(find .decree/inbox -maxdepth 1 -name '*.md' 2>/dev/null | wc -l)
  assert_eq "0" "$inbox_count" "inbox should be empty after successful process"
}

test_process_env_vars_available() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "Test env" > .decree/migrations/01-test.md
  ./decree process --no-color >/dev/null 2>&1
  local log_file
  log_file=$(find .decree/runs -name 'routine.log' 2>/dev/null | head -1)
  if [[ -n "$log_file" ]]; then
    assert_file_contains "$log_file" "FILE=" &&
    assert_file_contains "$log_file" "ID=" &&
    assert_file_contains "$log_file" "DIR=" &&
    assert_file_contains "$log_file" "CHAIN=" &&
    assert_file_contains "$log_file" "SEQ="
  else
    echo "no log file found"
    return 1
  fi
}

test_process_env_vars_nonempty() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  ./decree process --no-color >/dev/null 2>&1
  local log_file
  log_file=$(find .decree/runs -name 'routine.log' 2>/dev/null | head -1)
  if [[ -n "$log_file" ]]; then
    # Env vars should not be empty
    local content
    content=$(cat "$log_file")
    assert_not_contains "$content" "ID=$" "message_id should not be empty" &&
    assert_not_contains "$content" "CHAIN=$" "chain should not be empty"
  else
    echo "no log file found"
    return 1
  fi
}

test_process_custom_fields_as_env() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  cat > .decree/migrations/01-test.md <<'EOF'
---
routine: develop
my_custom: hello_world
---
Test custom fields.
EOF
  ./decree process --no-color >/dev/null 2>&1
  local log_file
  log_file=$(find .decree/runs -name 'routine.log' 2>/dev/null | head -1)
  if [[ -n "$log_file" ]]; then
    assert_file_contains "$log_file" "CUSTOM=hello_world"
  else
    echo "no log file found"
    return 1
  fi
}

test_process_output_shows_migration_name() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  local out
  out=$(./decree process --no-color 2>&1)
  assert_contains "$out" "01-test.md"
}

test_process_output_shows_routine_name() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  cat > .decree/migrations/01-test.md <<'EOF'
---
routine: develop
---
Test.
EOF
  local out
  out=$(./decree process --no-color 2>&1)
  assert_contains "$out" "develop"
}

test_process_output_shows_message_id() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  local out
  out=$(./decree process --no-color 2>&1)
  # Output should contain a D-prefixed message ID
  assert_match 'D[0-9]{4}-[0-9]{4}' "$out"
}

# ---------- 15: PROCESS — RETRY AND DEAD-LETTER ----------

test_process_retry_creates_multiple_logs() {
  init_project_no_hooks
  create_failing_routine
  mkdir -p .decree/migrations
  echo "Test retry" > .decree/migrations/01-test.md
  ./decree process --no-color >/dev/null 2>&1 || true
  local log_count
  log_count=$(find .decree/runs -name 'routine*.log' 2>/dev/null | wc -l)
  [[ $log_count -gt 1 ]] || {
    echo "expected multiple log files from retries, found $log_count"
    return 1
  }
}

test_process_retry_count_matches_config() {
  init_project_no_hooks
  create_failing_routine
  # Set max_retries to 2
  sed -i 's/max_retries:.*/max_retries: 2/' .decree/config.yml
  mkdir -p .decree/migrations
  echo "Test retry" > .decree/migrations/01-test.md
  ./decree process --no-color >/dev/null 2>&1 || true
  local log_count
  log_count=$(find .decree/runs -name 'routine*.log' 2>/dev/null | wc -l)
  assert_eq "2" "$log_count" "should have exactly max_retries log files"
}

test_process_dead_letters_on_exhaustion() {
  init_project_no_hooks
  create_failing_routine
  mkdir -p .decree/migrations
  echo "Test dead letter" > .decree/migrations/01-test.md
  ./decree process --no-color >/dev/null 2>&1 || true
  local dead_count
  dead_count=$(find .decree/inbox/dead -name '*.md' 2>/dev/null | wc -l)
  [[ $dead_count -gt 0 ]] || {
    echo "expected dead-lettered message, found $dead_count"
    return 1
  }
}

test_process_dead_letter_marked_processed() {
  init_project_no_hooks
  create_failing_routine
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  ./decree process --no-color >/dev/null 2>&1 || true
  assert_file_contains .decree/processed.md "01-test.md" \
    "dead-lettered migration should still be marked processed"
}

test_process_dead_letter_output_warning() {
  init_project_no_hooks
  create_failing_routine
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  local out
  out=$(./decree process --no-color 2>&1) || true
  assert_contains "$out" "max retries" || assert_contains "$out" "exhausted" || assert_contains "$out" "dead"
}

# ---------- 16: PROCESS — FOLLOW-UPS (OUTBOX) ----------

test_process_outbox_follow_ups() {
  init_project_no_hooks
  mkdir -p .decree/migrations
  cat > .decree/migrations/01-test.md <<'EOF'
---
routine: develop
---
Initial task.
EOF
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
#
# Creates follow-ups via outbox.
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
if [ "$seq" = "0" ]; then
  cat > .decree/outbox/follow-up.md <<FOLLOWUP
---
routine: develop
---
Follow-up task.
FOLLOWUP
fi
echo "processed seq=$seq"
SCRIPT
  chmod +x .decree/routines/develop.sh
  ./decree process --no-color >/dev/null 2>&1
  local run_count
  run_count=$(find .decree/runs -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l)
  [[ $run_count -ge 2 ]] || {
    echo "expected at least 2 run directories (original + follow-up), found $run_count"
    return 1
  }
}

test_process_drains_inbox_after_migration() {
  # When process runs, it processes migrations first, then drains inbox
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "Migration task" > .decree/migrations/01-test.md
  # Also put a direct inbox message
  cat > .decree/inbox/extra-msg.md <<'EOF'
---
routine: develop
---
Extra inbox message.
EOF
  ./decree process --no-color >/dev/null 2>&1
  # Both migration and inbox message should be processed
  local run_count
  run_count=$(find .decree/runs -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l)
  [[ $run_count -ge 2 ]] || {
    echo "expected at least 2 run directories (migration + inbox), found $run_count"
    return 1
  }
}

test_process_follow_up_increments_seq() {
  init_project_no_hooks
  mkdir -p .decree/migrations
  cat > .decree/migrations/01-test.md <<'EOF'
---
routine: develop
---
Chain test.
EOF
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
#
# Creates follow-up with incrementing seq.
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "SEQ=$seq"
if [ "$seq" = "0" ]; then
  cat > .decree/outbox/follow-up.md <<FOLLOWUP
---
routine: develop
---
Follow-up.
FOLLOWUP
fi
SCRIPT
  chmod +x .decree/routines/develop.sh
  ./decree process --no-color >/dev/null 2>&1
  # Check the second run's log has seq=1
  local found_seq1=false
  for log_file in $(find .decree/runs -name 'routine.log' 2>/dev/null); do
    if grep -q "SEQ=1" "$log_file" 2>/dev/null; then
      found_seq1=true
      break
    fi
  done
  $found_seq1 || {
    echo "expected follow-up with seq=1"
    return 1
  }
}

# ---------- 17: PROCESS — MULTIPLE MIGRATIONS ----------

test_process_multiple_migrations_in_order() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "First" > .decree/migrations/01-first.md
  echo "Second" > .decree/migrations/02-second.md
  echo "Third" > .decree/migrations/03-third.md
  ./decree process --no-color >/dev/null 2>&1
  assert_file_contains .decree/processed.md "01-first.md" &&
  assert_file_contains .decree/processed.md "02-second.md" &&
  assert_file_contains .decree/processed.md "03-third.md"
}

test_process_alphabetical_order() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "Third" > .decree/migrations/03-third.md
  echo "First" > .decree/migrations/01-first.md
  echo "Second" > .decree/migrations/02-second.md
  local out
  out=$(./decree process --no-color 2>&1)
  # First migration label should appear before second
  local pos1 pos2 pos3
  pos1=$(echo "$out" | grep -n "01-first" | head -1 | cut -d: -f1)
  pos2=$(echo "$out" | grep -n "02-second" | head -1 | cut -d: -f1)
  pos3=$(echo "$out" | grep -n "03-third" | head -1 | cut -d: -f1)
  [[ -n "$pos1" && -n "$pos2" && -n "$pos3" ]] || {
    echo "could not find all three migrations in output"
    return 1
  }
  [[ $pos1 -lt $pos2 && $pos2 -lt $pos3 ]] || {
    echo "expected alphabetical order: 01($pos1) < 02($pos2) < 03($pos3)"
    return 1
  }
}

test_process_skips_already_processed() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "First" > .decree/migrations/01-first.md
  echo "Second" > .decree/migrations/02-second.md
  echo "01-first.md" > .decree/processed.md
  ./decree process --no-color >/dev/null 2>&1
  local run_count
  run_count=$(find .decree/runs -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l)
  assert_eq "1" "$run_count" "should only process unprocessed migration"
}

test_process_idempotent_rerun() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  ./decree process --no-color >/dev/null 2>&1
  # Second run should process 0 migrations
  local out
  out=$(./decree process --no-color 2>&1)
  assert_contains "$out" "0 migration" || assert_contains "$out" "Processed 0"
}

# ---------- 18: PROCESS — LIFECYCLE HOOKS ----------

test_process_before_each_hook() {
  init_project
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  create_echo_routine
  local hook_marker="$PWD/hook_marker"
  cat > .decree/routines/test-hook.sh <<SCRIPT
#!/usr/bin/env bash
# Test Hook
#
# Logs hook type.
set -euo pipefail
message_file="\${message_file:-}"
message_id="\${message_id:-}"
message_dir="\${message_dir:-}"
chain="\${chain:-}"
seq="\${seq:-}"
if [ "\${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "HOOK_TYPE=\$DECREE_HOOK ATTEMPT=\$DECREE_ATTEMPT" >> $hook_marker
SCRIPT
  chmod +x .decree/routines/test-hook.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach:.*/beforeEach: "test-hook"/' .decree/config.yml
  sed -i 's/afterEach:.*/afterEach: ""/' .decree/config.yml
  sed -i 's/beforeAll:.*/beforeAll: ""/' .decree/config.yml
  sed -i 's/afterAll:.*/afterAll: ""/' .decree/config.yml

  ./decree process --no-color >/dev/null 2>&1
  if [[ -f "$hook_marker" ]]; then
    assert_file_contains "$hook_marker" "HOOK_TYPE=beforeEach" &&
    assert_file_contains "$hook_marker" "ATTEMPT=1"
  else
    echo "hook did not execute (marker file missing)"
    return 1
  fi
}

test_process_after_each_hook_exit_code() {
  init_project
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  create_echo_routine
  local hook_marker="$PWD/posthook_marker"
  cat > .decree/routines/post-hook.sh <<SCRIPT
#!/usr/bin/env bash
# Post Hook
#
# Logs exit code.
set -euo pipefail
message_file="\${message_file:-}"
message_id="\${message_id:-}"
message_dir="\${message_dir:-}"
chain="\${chain:-}"
seq="\${seq:-}"
if [ "\${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "EXIT_CODE=\$DECREE_ROUTINE_EXIT_CODE" >> $hook_marker
SCRIPT
  chmod +x .decree/routines/post-hook.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach:.*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach:.*/afterEach: "post-hook"/' .decree/config.yml
  sed -i 's/beforeAll:.*/beforeAll: ""/' .decree/config.yml
  sed -i 's/afterAll:.*/afterAll: ""/' .decree/config.yml

  ./decree process --no-color >/dev/null 2>&1
  if [[ -f "$hook_marker" ]]; then
    assert_file_contains "$hook_marker" "EXIT_CODE=0"
  else
    echo "afterEach hook did not execute"
    return 1
  fi
}

test_process_before_all_hook() {
  init_project
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  create_echo_routine
  local hook_marker="$PWD/beforeall_marker"
  cat > .decree/routines/all-hook.sh <<SCRIPT
#!/usr/bin/env bash
# All Hook
#
# Logs beforeAll.
set -euo pipefail
message_file="\${message_file:-}"
message_id="\${message_id:-}"
message_dir="\${message_dir:-}"
chain="\${chain:-}"
seq="\${seq:-}"
if [ "\${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "HOOK=\$DECREE_HOOK" >> $hook_marker
SCRIPT
  chmod +x .decree/routines/all-hook.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeAll:.*/beforeAll: "all-hook"/' .decree/config.yml
  sed -i 's/afterAll:.*/afterAll: ""/' .decree/config.yml
  sed -i 's/beforeEach:.*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach:.*/afterEach: ""/' .decree/config.yml

  ./decree process --no-color >/dev/null 2>&1
  if [[ -f "$hook_marker" ]]; then
    assert_file_contains "$hook_marker" "HOOK=beforeAll"
  else
    echo "beforeAll hook did not execute"
    return 1
  fi
}

test_process_after_all_hook() {
  init_project
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  create_echo_routine
  local hook_marker="$PWD/afterall_marker"
  cat > .decree/routines/all-hook.sh <<SCRIPT
#!/usr/bin/env bash
# All Hook
#
# Logs afterAll.
set -euo pipefail
message_file="\${message_file:-}"
message_id="\${message_id:-}"
message_dir="\${message_dir:-}"
chain="\${chain:-}"
seq="\${seq:-}"
if [ "\${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "HOOK=\$DECREE_HOOK" >> $hook_marker
SCRIPT
  chmod +x .decree/routines/all-hook.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeAll:.*/beforeAll: ""/' .decree/config.yml
  sed -i 's/afterAll:.*/afterAll: "all-hook"/' .decree/config.yml
  sed -i 's/beforeEach:.*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach:.*/afterEach: ""/' .decree/config.yml

  ./decree process --no-color >/dev/null 2>&1
  if [[ -f "$hook_marker" ]]; then
    assert_file_contains "$hook_marker" "HOOK=afterAll"
  else
    echo "afterAll hook did not execute"
    return 1
  fi
}

# ---------- 19: PROCESS — SIGINT ----------

test_process_sigint_exit_130() {
  init_project_no_hooks
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
#
# Slow routine for SIGINT test.
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
sleep 10
SCRIPT
  chmod +x .decree/routines/develop.sh

  ./decree process --no-color &
  local pid=$!
  sleep 1
  kill -INT "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null
  local rc=$?
  [[ $rc -ne 0 ]] || {
    echo "expected non-zero exit after SIGINT, got $rc"
    return 1
  }
}

test_process_sigint_no_second_migration() {
  init_project_no_hooks
  mkdir -p .decree/migrations
  echo "First" > .decree/migrations/01-first.md
  echo "Second" > .decree/migrations/02-second.md
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
#
# Slow.
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
sleep 10
SCRIPT
  chmod +x .decree/routines/develop.sh

  ./decree process --no-color &
  local pid=$!
  sleep 1
  kill -INT "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true

  if grep -qF "02-second.md" .decree/processed.md 2>/dev/null; then
    echo "SIGINT should prevent processing second migration"
    return 1
  fi
}

# ---------- 20: PROCESS — SUMMARY OUTPUT ----------

test_process_prints_summary() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  local out
  out=$(./decree process --no-color 2>&1)
  assert_contains "$out" "Processed" || assert_contains "$out" "processed"
}

test_process_prints_migration_count() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "A" > .decree/migrations/01-a.md
  echo "B" > .decree/migrations/02-b.md
  local out
  out=$(./decree process --no-color 2>&1)
  assert_contains "$out" "2 migration"
}

test_process_prints_duration() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  local out
  out=$(./decree process --no-color 2>&1)
  assert_match '[0-9]+s' "$out" "should show duration"
}

# ---------- 21: PROCESS — DIRECT INBOX ----------

test_process_drains_inbox() {
  init_project_no_hooks
  create_echo_routine
  cat > .decree/inbox/direct-msg.md <<'EOF'
---
routine: develop
---
A direct inbox message.
EOF
  ./decree process --no-color >/dev/null 2>&1
  local inbox_count
  inbox_count=$(find .decree/inbox -maxdepth 1 -name '*.md' 2>/dev/null | wc -l)
  assert_eq "0" "$inbox_count" "inbox should be empty after draining"
  local run_count
  run_count=$(find .decree/runs -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l)
  [[ $run_count -gt 0 ]] || {
    echo "expected run directory from inbox message"
    return 1
  }
}

test_process_bare_inbox_message_normalized() {
  init_project_no_hooks
  create_echo_routine
  echo "Bare message content." > .decree/inbox/test-msg.md
  ./decree process --no-color >/dev/null 2>&1 || true
  local msg_file
  msg_file=$(find .decree/runs -name 'message.md' 2>/dev/null | head -1)
  if [[ -n "$msg_file" ]]; then
    assert_file_contains "$msg_file" "chain:" &&
    assert_file_contains "$msg_file" "seq:" &&
    assert_file_contains "$msg_file" "id:"
  else
    echo "note: no run directory created"
    return 0
  fi
}

# ---------- 22: PROCESS — DEFAULT ROUTINE ----------

test_process_uses_default_routine() {
  init_project_no_hooks
  create_echo_routine
  sed -i 's/default_routine:.*/default_routine: develop/' .decree/config.yml
  mkdir -p .decree/migrations
  # Migration with no routine frontmatter
  echo "No routine specified." > .decree/migrations/01-no-routine.md
  local out
  out=$(./decree process --dry-run --no-color 2>&1)
  assert_contains "$out" "develop"
}

# ---------- 23: PROMPT ----------

test_prompt_list_non_tty() {
  init_project
  local out rc=0
  out=$(./decree prompt --no-color 2>&1) || rc=$?
  assert_exit_code 0 "$rc" &&
  assert_contains "$out" "migration" &&
  assert_contains "$out" "sow" &&
  assert_contains "$out" "routine"
}

test_prompt_named_outputs_content() {
  init_project
  local out rc=0
  out=$(./decree prompt sow --no-color 2>&1) || rc=$?
  assert_exit_code 0 "$rc" &&
  (assert_contains "$out" "Statement of Work" || assert_contains "$out" "SOW")
}

test_prompt_substitution_migrations() {
  init_project
  mkdir -p .decree/migrations
  echo "Auth task" > .decree/migrations/01-auth.md
  local out
  out=$(./decree prompt migration --no-color 2>&1)
  assert_contains "$out" "01-auth.md" &&
  assert_not_contains "$out" "{migrations}" "placeholder should be substituted"
}

test_prompt_substitution_processed() {
  init_project
  echo "01-done.md" > .decree/processed.md
  local out
  out=$(./decree prompt migration --no-color 2>&1)
  assert_contains "$out" "01-done.md" &&
  assert_not_contains "$out" "{processed}" "placeholder should be substituted"
}

test_prompt_substitution_routines() {
  init_project
  local out
  out=$(./decree prompt routine --no-color 2>&1)
  assert_contains "$out" "develop" &&
  assert_not_contains "$out" "{routines}" "placeholder should be substituted"
}

test_prompt_substitution_config() {
  init_project
  # Create a prompt that uses {config}
  cat > .decree/prompts/config-test.md <<'EOF'
## Config
{config}
EOF
  local out
  out=$(./decree prompt config-test --no-color 2>&1)
  assert_contains "$out" "max_retries" &&
  assert_not_contains "$out" "{config}" "placeholder should be substituted"
}

test_prompt_unknown_fails() {
  init_project
  local out rc=0
  out=$(./decree prompt nonexistent --no-color 2>&1) || rc=$?
  [[ $rc -ne 0 ]] || {
    echo "expected error for unknown prompt"
    return 1
  }
  assert_contains "$out" "unknown prompt" || assert_contains "$out" "Available"
}

test_prompt_unknown_lists_available() {
  init_project
  local out rc=0
  out=$(./decree prompt nonexistent --no-color 2>&1) || rc=$?
  assert_contains "$out" "migration" &&
  assert_contains "$out" "routine" &&
  assert_contains "$out" "sow"
}

test_prompt_custom_template() {
  init_project
  cat > .decree/prompts/custom.md <<'EOF'
# Custom Prompt
This is a custom prompt template.
EOF
  local out
  out=$(./decree prompt custom --no-color 2>&1)
  assert_contains "$out" "Custom Prompt"
}

# ---------- 24: DAEMON BASICS ----------

test_daemon_starts_and_stops() {
  init_project_no_hooks
  timeout 3 ./decree daemon --interval 1 --no-color >/dev/null 2>&1 &
  local pid=$!
  sleep 1
  kill -TERM "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  return 0
}

test_daemon_prints_polling_message() {
  init_project_no_hooks
  local out
  out=$(timeout 3 ./decree daemon --interval 1 --no-color 2>&1) || true
  assert_contains "$out" "polling" || assert_contains "$out" "daemon"
}

test_daemon_processes_inbox() {
  init_project_no_hooks
  create_echo_routine
  cat > .decree/inbox/D0001-0900-test-0.md <<'EOF'
---
id: D0001-0900-test-0
chain: D0001-0900-test
seq: 0
routine: develop
---
Daemon test message.
EOF

  timeout 5 ./decree daemon --interval 1 --no-color >/dev/null 2>&1 &
  local pid=$!
  sleep 3
  kill -TERM "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true

  local inbox_count
  inbox_count=$(find .decree/inbox -maxdepth 1 -name '*.md' 2>/dev/null | wc -l)
  assert_eq "0" "$inbox_count" "daemon should process inbox messages"
}

test_daemon_custom_interval() {
  init_project_no_hooks
  local out
  out=$(timeout 3 ./decree daemon --interval 2 --no-color 2>&1) || true
  assert_contains "$out" "2s" || assert_contains "$out" "polling every 2"
}

# ---------- 25: CRON ----------

test_cron_file_fires_via_daemon() {
  init_project_no_hooks
  create_echo_routine
  cat > .decree/cron/every-minute.md <<'EOF'
---
cron: "* * * * *"
routine: develop
---
Every minute task.
EOF

  timeout 5 ./decree daemon --interval 1 --no-color >/dev/null 2>&1 &
  local pid=$!
  sleep 3
  kill -TERM "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true

  local run_count
  run_count=$(find .decree/runs -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l)
  [[ $run_count -gt 0 ]] || {
    echo "expected cron to fire and create run directory, found $run_count"
    return 1
  }
}

test_cron_daemon_output_mentions_cron() {
  init_project_no_hooks
  create_echo_routine
  cat > .decree/cron/test-cron.md <<'EOF'
---
cron: "* * * * *"
routine: develop
---
Test.
EOF

  local out
  out=$(timeout 5 ./decree daemon --interval 1 --no-color 2>&1) || true
  assert_contains "$out" "cron" || assert_contains "$out" "test-cron"
}

# ---------- 26: MESSAGE FORMAT ----------

test_message_id_format() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  ./decree process --no-color >/dev/null 2>&1
  local log_file
  log_file=$(find .decree/runs -name 'routine.log' 2>/dev/null | head -1)
  if [[ -n "$log_file" ]]; then
    local id_line
    id_line=$(grep "ID=" "$log_file" | head -1)
    if [[ "$id_line" =~ ID=D[0-9]{4}-[0-9]{4}-.+-[0-9]+ ]]; then
      return 0
    else
      echo "message ID does not match expected format: $id_line"
      return 1
    fi
  else
    echo "no log file found"
    return 1
  fi
}

test_migration_body_in_message() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  cat > .decree/migrations/01-test.md <<'EOF'
---
routine: develop
---
UNIQUE_CONTENT_XYZ_12345
EOF
  ./decree process --no-color >/dev/null 2>&1
  local msg_file
  msg_file=$(find .decree/runs -name 'message.md' 2>/dev/null | head -1)
  assert_file_contains "$msg_file" "UNIQUE_CONTENT_XYZ_12345" "migration body should be in message"
}

test_message_frontmatter_has_required_fields() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  ./decree process --no-color >/dev/null 2>&1
  local msg_file
  msg_file=$(find .decree/runs -name 'message.md' 2>/dev/null | head -1)
  if [[ -n "$msg_file" ]]; then
    assert_file_contains "$msg_file" "id:" &&
    assert_file_contains "$msg_file" "chain:" &&
    assert_file_contains "$msg_file" "seq:" &&
    assert_file_contains "$msg_file" "routine:" &&
    assert_file_contains "$msg_file" "migration:"
  else
    echo "no message file found"
    return 1
  fi
}

# ---------- 27: ROUTINE-SYNC ----------

test_routine_sync_discovers_new_routine() {
  init_project
  cat > .decree/routines/new-routine.sh <<'SCRIPT'
#!/usr/bin/env bash
# New Routine
#
# A newly added routine.
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "new"
SCRIPT
  chmod +x .decree/routines/new-routine.sh
  local out
  out=$(./decree routine-sync --no-color 2>&1)
  assert_contains "$out" "new-routine" &&
  assert_file_contains .decree/config.yml "new-routine:"
}

test_routine_sync_new_project_routine_enabled() {
  init_project
  cat > .decree/routines/fresh.sh <<'SCRIPT'
#!/usr/bin/env bash
# Fresh
#
# Fresh routine.
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
SCRIPT
  chmod +x .decree/routines/fresh.sh
  ./decree routine-sync --no-color >/dev/null 2>&1
  assert_file_contains .decree/config.yml "fresh:" &&
  assert_file_contains .decree/config.yml "enabled: true"
}

test_routine_sync_shows_enabled_status() {
  init_project
  local out
  out=$(./decree routine-sync --no-color 2>&1)
  assert_contains "$out" "enabled" &&
  assert_contains "$out" "develop"
}

test_routine_sync_shared_source() {
  init_project
  # Create shared routines directory
  local shared_dir="$PWD/shared_routines"
  mkdir -p "$shared_dir"
  cat > "$shared_dir/shared-echo.sh" <<'SCRIPT'
#!/usr/bin/env bash
# Shared Echo
#
# A shared routine.
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "shared"
SCRIPT
  chmod +x "$shared_dir/shared-echo.sh"
  local out
  out=$(./decree routine-sync --source "$shared_dir" --no-color 2>&1)
  assert_contains "$out" "shared-echo" &&
  assert_contains "$out" "Shared routines"
}

test_routine_sync_shared_routine_disabled_by_default() {
  init_project
  local shared_dir="$PWD/shared_routines"
  mkdir -p "$shared_dir"
  cat > "$shared_dir/shared-test.sh" <<'SCRIPT'
#!/usr/bin/env bash
# Shared Test
#
# A shared routine.
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
SCRIPT
  chmod +x "$shared_dir/shared-test.sh"
  ./decree routine-sync --source "$shared_dir" --no-color >/dev/null 2>&1
  local out
  out=$(./decree routine-sync --source "$shared_dir" --no-color 2>&1)
  assert_contains "$out" "disabled"
}

test_routine_sync_adds_shared_routines_to_config() {
  init_project
  local shared_dir="$PWD/shared_routines"
  mkdir -p "$shared_dir"
  cat > "$shared_dir/shared-cfg.sh" <<'SCRIPT'
#!/usr/bin/env bash
# Shared Cfg
#
# Test.
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
SCRIPT
  chmod +x "$shared_dir/shared-cfg.sh"
  ./decree routine-sync --source "$shared_dir" --no-color >/dev/null 2>&1
  assert_file_contains .decree/config.yml "shared_routines:" &&
  assert_file_contains .decree/config.yml "shared-cfg:"
}

# ---------- 28: DISABLED ROUTINES ----------

test_disabled_routine_not_listed() {
  init_project
  cat > .decree/routines/disabled-test.sh <<'SCRIPT'
#!/usr/bin/env bash
# Disabled Test
#
# Should not appear when disabled.
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
SCRIPT
  chmod +x .decree/routines/disabled-test.sh
  ./decree routine-sync --no-color >/dev/null 2>&1
  # Disable the routine
  sed -i '/disabled-test:/,/enabled:/{s/enabled: true/enabled: false/}' .decree/config.yml
  local out
  out=$(./decree routine --no-color 2>&1)
  assert_not_contains "$out" "disabled-test" "disabled routine should not be listed"
}

test_disabled_routine_not_in_verify() {
  init_project
  cat > .decree/routines/disabled-verify.sh <<'SCRIPT'
#!/usr/bin/env bash
# Disabled Verify
#
# Should not appear in verify when disabled.
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 1; fi
SCRIPT
  chmod +x .decree/routines/disabled-verify.sh
  ./decree routine-sync --no-color >/dev/null 2>&1
  sed -i '/disabled-verify:/,/enabled:/{s/enabled: true/enabled: false/}' .decree/config.yml
  local out
  out=$(./decree verify --no-color 2>&1)
  assert_not_contains "$out" "disabled-verify"
}

# ---------- 29: BARE DECREE DISPATCHES TO PROCESS ----------

test_bare_decree_is_process() {
  init_project_no_hooks
  create_echo_routine
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  ./decree --no-color >/dev/null 2>&1
  assert_file_contains .decree/processed.md "01-test.md" "bare decree should process migrations"
}

test_bare_decree_no_migrations() {
  init_project_no_hooks
  create_echo_routine
  local out
  out=$(./decree --no-color 2>&1)
  assert_contains "$out" "0 migration" || assert_contains "$out" "Processed 0"
}

# ---------- 30: NO_COLOR ENVIRONMENT VARIABLE ----------

test_no_color_env_var_status() {
  init_project
  local rc=0
  NO_COLOR=1 ./decree status 2>&1 || rc=$?
  assert_exit_code 0 "$rc"
}

test_no_color_env_var_process() {
  init_project_no_hooks
  create_echo_routine
  local rc=0
  NO_COLOR=1 ./decree process 2>&1 || rc=$?
  assert_exit_code 0 "$rc"
}

# ================================================================
#                         RUN ALL TESTS
# ================================================================

echo "decree test suite"
echo "================="
echo ""

# Collect all test functions
TESTS=(
  # CLI basics
  "version flag"                     test_version_flag
  "version short flag (-v)"          test_version_short_flag
  "help flag (--help)"               test_help_flag
  "help subcommand (verbose)"        test_help_subcommand
  "help subcommand length"           test_help_subcommand_verbose
  "help mentions routine-sync"       test_help_mentions_routine_sync
  "help mentions routine registry"   test_help_mentions_routine_registry
  "unknown subcommand exit 2"        test_unknown_subcommand_exit_2
  "no-color flag accepted"           test_no_color_flag_accepted
  "NO_COLOR env var accepted"        test_no_color_env_var
  "version needs no project"         test_version_no_project_needed
  "help needs no project"            test_help_no_project_needed

  # Require .decree/
  "bare decree w/o project fails"    test_bare_decree_without_project_fails
  "process w/o project fails"        test_process_without_project_fails
  "status w/o project fails"         test_status_without_project_fails
  "routine w/o project fails"        test_routine_without_project_fails
  "verify w/o project fails"         test_verify_without_project_fails
  "log w/o project fails"            test_log_without_project_fails
  "daemon w/o project fails"         test_daemon_without_project_fails
  "prompt w/o project fails"         test_prompt_without_project_fails
  "routine-sync w/o project fails"   test_routine_sync_without_project_fails

  # Init — directory structure
  "init creates .decree/"            test_init_creates_decree_dir
  "init creates routines/"           test_init_creates_routines_dir
  "init creates prompts/"            test_init_creates_prompts_dir
  "init creates cron/"               test_init_creates_cron_dir
  "init creates inbox/"              test_init_creates_inbox_dir
  "init creates inbox/dead/"         test_init_creates_inbox_dead_dir
  "init creates outbox/"             test_init_creates_outbox_dir
  "init creates outbox/dead/"        test_init_creates_outbox_dead_dir
  "init creates runs/"               test_init_creates_runs_dir
  "init creates migrations/"         test_init_creates_migrations_dir
  "init creates config.yml"          test_init_creates_config_yml
  "init creates .gitignore"          test_init_creates_gitignore
  "init creates processed.md"        test_init_creates_processed_md
  "init processed.md empty"          test_init_processed_md_empty
  "init creates router.md"           test_init_creates_router_md
  "init router.md not in prompts/"   test_init_router_not_in_prompts

  # Init — config
  "config no ai_command (deprecated)" test_init_config_no_ai_command
  "config has ai_router"             test_init_config_has_ai_router
  "config has ai_interactive"        test_init_config_has_ai_interactive
  "config max_retries 3"             test_init_config_max_retries_3
  "config max_depth 10"              test_init_config_max_depth_10
  "config max_log_size 2MB"          test_init_config_max_log_size
  "config default_routine develop"   test_init_config_default_routine_develop
  "config has hooks section"         test_init_config_has_hooks_section
  "config has routines section"      test_init_config_has_routines_section
  "config commented alternatives"    test_init_config_commented_alternatives
  "config routine_source present"    test_init_config_routine_source_commented

  # Init — templates
  "init creates develop.sh"          test_init_creates_develop_sh
  "init creates rust-develop.sh"     test_init_creates_rust_develop_sh
  "init routines executable"         test_init_routines_executable
  "init develop.sh has shebang"      test_init_develop_has_shebang
  "init develop.sh has pre-check"    test_init_develop_has_precheck
  "init develop.sh has description"  test_init_develop_has_description
  "init develop.sh no AI placeholder" test_init_develop_no_ai_placeholder
  "init rust-develop no AI placeholder" test_init_rust_develop_no_ai_placeholder
  "init develop.sh has message_dir"  test_init_develop_has_message_dir_ref
  "init develop.sh refs AI tool"     test_init_develop_references_ai_tool
  "init creates prompt templates"    test_init_creates_prompt_templates
  "init migration.md placeholders"   test_init_migration_prompt_has_placeholders
  "init routine.md placeholder"      test_init_routine_prompt_has_placeholder
  "init router.md placeholders"      test_init_router_has_placeholders

  # Init — gitignore
  "gitignore inbox/"                 test_init_gitignore_inbox
  "gitignore outbox/"                test_init_gitignore_outbox
  "gitignore runs/"                  test_init_gitignore_runs

  # Init — git hooks
  "init creates git hook routines"   test_init_git_creates_hook_routines
  "init git hooks executable"        test_init_git_hooks_executable
  "init git-baseline pre-check"      test_init_git_baseline_has_precheck
  "init git-stash pre-check"         test_init_git_stash_has_precheck
  "init git-baseline has DECREE_ATTEMPT" test_init_git_baseline_has_hook_env
  "init git-stash has EXIT_CODE"     test_init_git_stash_has_hook_env

  # Init — re-run
  "init re-run overwrites non-tty"   test_init_rerun_overwrites_non_tty
  "init re-run prints warning"       test_init_rerun_prints_warning

  # Status
  "status empty project"             test_status_empty_project
  "status shows migration count"     test_status_shows_migration_count
  "status shows next migration"      test_status_shows_next_migration
  "status shows inbox count"         test_status_shows_inbox_count
  "status shows dead-letter count"   test_status_shows_dead_letter_count
  "status shows recent activity"     test_status_shows_recent_activity
  "status --no-color flag"           test_status_no_color_flag

  # Log
  "log no runs"                      test_log_no_runs
  "log shows recent run"             test_log_shows_recent_run
  "log specific run"                 test_log_specific_run
  "log prefix match"                 test_log_prefix_match
  "log multiple attempts"            test_log_multiple_attempts
  "log nonexistent run fails"        test_log_nonexistent_run_fails
  "log shows timestamps"             test_log_shows_timestamps

  # Routine
  "routine list non-tty"             test_routine_list_non_tty
  "routine list shows descriptions"  test_routine_list_shows_descriptions
  "routine list shows rust-develop"  test_routine_list_shows_rust_develop
  "routine list shows git hooks"     test_routine_list_shows_git_hooks
  "routine detail named"             test_routine_detail_named
  "routine detail shows path"        test_routine_detail_shows_path
  "routine detail shows description" test_routine_detail_shows_description
  "routine unknown fails"            test_routine_unknown_fails
  "routine no routines message"      test_routine_no_routines
  "routine nested discovery"         test_routine_nested

  # Verify
  "verify all pass"                  test_verify_all_pass
  "verify some fail exit 3"          test_verify_some_fail_exit_3
  "verify shows fail reason"         test_verify_shows_fail_reason
  "verify reports count"             test_verify_reports_count
  "verify no routines"               test_verify_no_routines
  "verify includes hook pre-checks"  test_verify_includes_hook_prechecks

  # Process — dry run
  "process dry-run no migrations"    test_process_dry_run_no_migrations
  "process dry-run lists migrations" test_process_dry_run_lists_migrations
  "process dry-run shows routine"    test_process_dry_run_shows_routine
  "process dry-run shows precheck"   test_process_dry_run_shows_precheck_status
  "process dry-run creates no files" test_process_dry_run_no_files_created

  # Process — basic execution
  "process simple migration"         test_process_simple_migration
  "process creates run directory"    test_process_creates_run_directory
  "process run dir has message.md"   test_process_run_dir_has_message
  "process run dir has routine.log"  test_process_run_dir_has_log
  "process log has timestamps"       test_process_log_has_timestamps
  "process removes from inbox"       test_process_removes_from_inbox
  "process env vars available"       test_process_env_vars_available
  "process env vars nonempty"        test_process_env_vars_nonempty
  "process custom fields as env"     test_process_custom_fields_as_env
  "process output shows migration"   test_process_output_shows_migration_name
  "process output shows routine"     test_process_output_shows_routine_name
  "process output shows message ID"  test_process_output_shows_message_id

  # Process — retry and dead-letter
  "process retry multiple logs"      test_process_retry_creates_multiple_logs
  "process retry count matches cfg"  test_process_retry_count_matches_config
  "process dead-letters on exhaust"  test_process_dead_letters_on_exhaustion
  "process dead marked processed"    test_process_dead_letter_marked_processed
  "process dead output warning"      test_process_dead_letter_output_warning

  # Process — follow-ups
  "process outbox follow-ups"        test_process_outbox_follow_ups
  "process drains inbox after migr."  test_process_drains_inbox_after_migration
  "process follow-up increments seq" test_process_follow_up_increments_seq

  # Process — multiple migrations
  "process multiple in order"        test_process_multiple_migrations_in_order
  "process alphabetical order"       test_process_alphabetical_order
  "process skips already processed"  test_process_skips_already_processed
  "process idempotent rerun"         test_process_idempotent_rerun

  # Process — lifecycle hooks
  "process beforeEach hook"          test_process_before_each_hook
  "process afterEach exit code"      test_process_after_each_hook_exit_code
  "process beforeAll hook"           test_process_before_all_hook
  "process afterAll hook"            test_process_after_all_hook

  # Process — SIGINT
  "process SIGINT exit non-zero"     test_process_sigint_exit_130
  "process SIGINT no second migr."   test_process_sigint_no_second_migration

  # Process — summary
  "process prints summary"           test_process_prints_summary
  "process prints migration count"   test_process_prints_migration_count
  "process prints duration"          test_process_prints_duration

  # Process — direct inbox
  "process drains inbox"             test_process_drains_inbox
  "process normalizes bare inbox"    test_process_bare_inbox_message_normalized

  # Process — default routine
  "process uses default routine"     test_process_uses_default_routine

  # Prompt
  "prompt list non-tty"              test_prompt_list_non_tty
  "prompt named outputs content"     test_prompt_named_outputs_content
  "prompt substitution migrations"   test_prompt_substitution_migrations
  "prompt substitution processed"    test_prompt_substitution_processed
  "prompt substitution routines"     test_prompt_substitution_routines
  "prompt substitution config"       test_prompt_substitution_config
  "prompt unknown fails"             test_prompt_unknown_fails
  "prompt unknown lists available"   test_prompt_unknown_lists_available
  "prompt custom template"           test_prompt_custom_template

  # Daemon
  "daemon starts and stops"          test_daemon_starts_and_stops
  "daemon prints polling message"    test_daemon_prints_polling_message
  "daemon processes inbox"           test_daemon_processes_inbox
  "daemon custom interval"           test_daemon_custom_interval

  # Cron
  "cron file fires via daemon"       test_cron_file_fires_via_daemon
  "cron daemon output mentions cron" test_cron_daemon_output_mentions_cron

  # Message format
  "message ID format D-HHmm-name"   test_message_id_format
  "migration body in message"        test_migration_body_in_message
  "message has required frontmatter" test_message_frontmatter_has_required_fields

  # Routine-sync
  "routine-sync discovers new"       test_routine_sync_discovers_new_routine
  "routine-sync new enabled"         test_routine_sync_new_project_routine_enabled
  "routine-sync shows status"        test_routine_sync_shows_enabled_status
  "routine-sync shared source"       test_routine_sync_shared_source
  "routine-sync shared disabled"     test_routine_sync_shared_routine_disabled_by_default
  "routine-sync shared in config"    test_routine_sync_adds_shared_routines_to_config

  # Disabled routines
  "disabled routine not listed"      test_disabled_routine_not_listed
  "disabled routine not in verify"   test_disabled_routine_not_in_verify

  # Bare decree
  "bare decree dispatches to process" test_bare_decree_is_process
  "bare decree no migrations"        test_bare_decree_no_migrations

  # NO_COLOR env var
  "NO_COLOR env status"              test_no_color_env_var_status
  "NO_COLOR env process"             test_no_color_env_var_process
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
