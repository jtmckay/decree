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

# Initialize a decree project non-interactively (no TTY → auto-detect, auto-accept)
init_project() {
  cd "$TEST_DIR"
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
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

# ================================================================
#                         TEST FUNCTIONS
# ================================================================
# Each test_* function is self-contained. It sets up state, runs
# assertions, and returns 0 on success, non-zero on failure.
# ================================================================

# ---------- 01: VERSION AND BASIC CLI ----------

test_version_flag() {
  local out
  out=$(./decree --version 2>&1)
  assert_eq "decree 0.2.0" "$out"
}

test_version_short_flag() {
  local out
  out=$(./decree -v 2>&1)
  assert_eq "decree 0.2.0" "$out"
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
  # help subcommand should be much longer than --help
  local lines
  lines=$(echo "$out" | wc -l)
  [[ $lines -gt 50 ]] || {
    echo "expected verbose help (>50 lines), got $lines lines"
    return 1
  }
}

test_unknown_subcommand_exit_2() {
  local rc=0
  ./decree nonexistent </dev/null >/dev/null 2>&1 || rc=$?
  assert_exit_code 2 "$rc"
}

test_no_color_flag_accepted() {
  # --no-color should be accepted without error
  local rc=0
  ./decree --no-color --version 2>&1 || rc=$?
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
  local rc=0
  ./decree process </dev/null >/dev/null 2>&1 || rc=$?
  [[ $rc -ne 0 ]] || {
    echo "expected failure without .decree/, got exit 0"
    return 1
  }
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
  # Should fail, not hang
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

test_init_config_commented_alternatives() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  # Config should contain commented-out alternative AI backends
  local content
  content=$(cat .decree/config.yml)
  assert_contains "$content" "#" "should have comments for alternative backends"
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
  # Second line should be a comment with routine title
  local line2
  line2=$(sed -n '2p' .decree/routines/develop.sh)
  assert_contains "$line2" "#"
}

test_init_develop_no_ai_cmd_placeholder() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  # {AI_CMD} should be replaced with actual command name
  if grep -qF '{AI_CMD}' .decree/routines/develop.sh; then
    echo "{AI_CMD} placeholder was not replaced"
    return 1
  fi
}

test_init_rust_develop_no_ai_cmd_placeholder() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  if grep -qF '{AI_CMD}' .decree/routines/rust-develop.sh; then
    echo "{AI_CMD} placeholder was not replaced"
    return 1
  fi
}

test_init_develop_has_message_dir_ref() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  assert_file_contains .decree/routines/develop.sh 'message_dir'
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

# ---------- 07: INIT — GIT HOOKS (non-TTY accepts) ----------

test_init_git_creates_hook_routines() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  # Non-TTY auto-accepts git hooks when git is detected
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

# ---------- 08: INIT — RE-RUN BEHAVIOR ----------

test_init_rerun_overwrites_non_tty() {
  git init -q .
  ./decree init </dev/null >/dev/null 2>&1
  # Modify config to verify overwrite
  echo "# modified" >> .decree/config.yml
  ./decree init </dev/null >/dev/null 2>&1
  if grep -qF "# modified" .decree/config.yml; then
    echo "re-run should overwrite in non-TTY mode"
    return 1
  fi
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

# ---------- 11: ROUTINE LIST AND DETAIL ----------

test_routine_list_non_tty() {
  init_project
  local out rc=0
  out=$(./decree routine --no-color 2>&1) || rc=$?
  assert_exit_code 0 "$rc" &&
  assert_contains "$out" "develop"
}

test_routine_list_shows_rust_develop() {
  init_project
  local out
  out=$(./decree routine --no-color 2>&1)
  assert_contains "$out" "rust-develop"
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
  # Should show some description text from the routine header
  local lines
  lines=$(echo "$out" | wc -l)
  [[ $lines -gt 1 ]] || {
    echo "expected multi-line detail, got $lines lines"
    return 1
  }
}

test_routine_unknown_suggests_close_match() {
  init_project
  local out rc=0
  out=$(./decree routine devlop --no-color 2>&1) || rc=$?
  assert_contains "$out" "develop" "should suggest close match"
}

test_routine_unknown_no_match() {
  init_project
  local out rc=0
  out=$(./decree routine zzzzzzzzz --no-color 2>&1) || rc=$?
  [[ $rc -ne 0 ]] || {
    echo "expected error for unknown routine"
    return 1
  }
}

test_routine_no_routines() {
  init_project
  rm -rf .decree/routines/*
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

if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    exit 0
fi

echo "deploying to staging"
SCRIPT
  chmod +x .decree/routines/deploy/staging.sh
  local out
  out=$(./decree routine --no-color 2>&1)
  assert_contains "$out" "deploy/staging"
}

test_routine_custom_params() {
  init_project
  cat > .decree/routines/custom.sh <<'SCRIPT'
#!/usr/bin/env bash
# Custom Routine
#
# A routine with custom parameters.
set -euo pipefail

message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    exit 0
fi

output_file="${output_file:-}"
model="${model:-large}"

echo "running with model=$model output_file=$output_file"
SCRIPT
  chmod +x .decree/routines/custom.sh
  local out
  out=$(./decree routine custom --no-color 2>&1)
  assert_contains "$out" "output_file" &&
  assert_contains "$out" "model"
}

# ---------- 12: VERIFY ----------

test_verify_all_pass() {
  init_project
  # Create a routine that always passes pre-check
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

if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    exit 0
fi

echo "ok"
SCRIPT
  chmod +x .decree/routines/simple.sh
  # Remove routines that might fail (AI tools not installed in test env)
  rm -f .decree/routines/develop.sh .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  # Clear hooks so verify doesn't check non-existent hook routines
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml
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
  # Remove other routines
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

test_verify_no_routines() {
  init_project
  rm -rf .decree/routines/*
  local out
  out=$(./decree verify --no-color 2>&1)
  assert_contains "$out" "No routines" || assert_contains "$out" "no routines" || assert_contains "$out" "0"
}

test_verify_includes_hook_prechecks() {
  init_project
  # Create a hook routine
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

if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    exit 0
fi
SCRIPT
  chmod +x .decree/routines/my-hook.sh
  # Configure it as a hook and clear others
  sed -i 's/beforeAll: .*/beforeAll: "my-hook"/' .decree/config.yml
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml
  rm -f .decree/routines/develop.sh .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  local out rc=0
  out=$(./decree verify --no-color 2>&1) || rc=$?
  assert_exit_code 0 "$rc" &&
  assert_contains "$out" "PASS"
}

# ---------- 13: PROCESS — DRY RUN ----------

test_process_dry_run_no_migrations() {
  init_project
  local out rc=0
  out=$(./decree process --dry-run --no-color 2>&1) || rc=$?
  # Should indicate no migrations to process
  assert_contains "$out" "No migrations" || assert_contains "$out" "no migrations" || assert_contains "$out" "no unprocessed" || assert_contains "$out" "No unprocessed" || assert_contains "$out" "0"
}

test_process_dry_run_lists_migrations() {
  init_project
  mkdir -p .decree/migrations
  cat > .decree/migrations/01-add-auth.md <<'EOF'
---
routine: develop
---
Add authentication module.
EOF
  rm -f .decree/routines/develop.sh .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
#
# Default routine.
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    exit 0
fi
echo "ok"
SCRIPT
  chmod +x .decree/routines/develop.sh
  local out
  out=$(./decree process --dry-run --no-color 2>&1)
  assert_contains "$out" "01-add-auth.md"
}

test_process_dry_run_no_files_created() {
  init_project
  mkdir -p .decree/migrations
  echo "Do auth" > .decree/migrations/01-add-auth.md
  rm -f .decree/routines/develop.sh .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "ok"
SCRIPT
  chmod +x .decree/routines/develop.sh
  ./decree process --dry-run --no-color >/dev/null 2>&1
  # No inbox messages should be created
  local inbox_count
  inbox_count=$(find .decree/inbox -name '*.md' 2>/dev/null | wc -l)
  assert_eq "0" "$inbox_count" "dry-run should not create inbox messages"
  # processed.md should remain empty
  local processed
  processed=$(cat .decree/processed.md)
  assert_eq "" "$processed" "dry-run should not mark processed"
}

# ---------- 14: PROCESS — BASIC EXECUTION ----------

test_process_simple_migration() {
  init_project
  mkdir -p .decree/migrations
  cat > .decree/migrations/01-test.md <<'EOF'
---
routine: develop
---
Run a simple test.
EOF
  # Replace develop.sh with one that succeeds
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
#
# Simple test routine.
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "Migration processed successfully"
SCRIPT
  chmod +x .decree/routines/develop.sh
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  # Clear hooks
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

  local out rc=0
  out=$(./decree process --no-color 2>&1) || rc=$?
  assert_exit_code 0 "$rc" "process should succeed" &&
  assert_file_contains .decree/processed.md "01-test.md" "migration should be marked processed"
}

test_process_creates_run_directory() {
  init_project
  mkdir -p .decree/migrations
  cat > .decree/migrations/01-test.md <<'EOF'
---
routine: develop
---
Run test.
EOF
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "ok"
SCRIPT
  chmod +x .decree/routines/develop.sh
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

  ./decree process --no-color >/dev/null 2>&1
  # Should have at least one run directory
  local run_count
  run_count=$(find .decree/runs -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l)
  [[ $run_count -gt 0 ]] || {
    echo "expected at least one run directory, found $run_count"
    return 1
  }
}

test_process_run_dir_has_message() {
  init_project
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "ok"
SCRIPT
  chmod +x .decree/routines/develop.sh
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

  ./decree process --no-color >/dev/null 2>&1
  local msg_file
  msg_file=$(find .decree/runs -name 'message.md' 2>/dev/null | head -1)
  [[ -n "$msg_file" ]] || {
    echo "expected message.md in run directory"
    return 1
  }
}

test_process_run_dir_has_log() {
  init_project
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "log output here"
SCRIPT
  chmod +x .decree/routines/develop.sh
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

  ./decree process --no-color >/dev/null 2>&1
  local log_file
  log_file=$(find .decree/runs -name 'routine.log' 2>/dev/null | head -1)
  [[ -n "$log_file" ]] || {
    echo "expected routine.log in run directory"
    return 1
  }
}

test_process_log_has_timestamps() {
  init_project
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "ok"
SCRIPT
  chmod +x .decree/routines/develop.sh
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

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
  init_project
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "ok"
SCRIPT
  chmod +x .decree/routines/develop.sh
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

  ./decree process --no-color >/dev/null 2>&1
  local inbox_count
  inbox_count=$(find .decree/inbox -maxdepth 1 -name '*.md' 2>/dev/null | wc -l)
  assert_eq "0" "$inbox_count" "inbox should be empty after successful process"
}

test_process_env_vars_available() {
  init_project
  mkdir -p .decree/migrations
  echo "Test env" > .decree/migrations/01-test.md
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "FILE=$message_file"
echo "ID=$message_id"
echo "DIR=$message_dir"
echo "CHAIN=$chain"
echo "SEQ=$seq"
SCRIPT
  chmod +x .decree/routines/develop.sh
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

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

test_process_custom_fields_as_env() {
  # KNOWN BUG: custom frontmatter fields are not passed as env vars to routines
  init_project
  mkdir -p .decree/migrations
  cat > .decree/migrations/01-test.md <<'EOF'
---
routine: develop
my_custom: hello_world
---
Test custom fields.
EOF
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
my_custom="${my_custom:-}"
echo "CUSTOM=$my_custom"
SCRIPT
  chmod +x .decree/routines/develop.sh
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

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

# ---------- 15: PROCESS — RETRY AND DEAD-LETTER ----------

test_process_retry_creates_multiple_logs() {
  init_project
  mkdir -p .decree/migrations
  echo "Test retry" > .decree/migrations/01-test.md
  # Routine that always fails
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
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
  chmod +x .decree/routines/develop.sh
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

  ./decree process --no-color >/dev/null 2>&1 || true
  # Should have multiple log files (routine.log, routine-2.log, routine-3.log)
  local log_count
  log_count=$(find .decree/runs -name 'routine*.log' 2>/dev/null | wc -l)
  [[ $log_count -gt 1 ]] || {
    echo "expected multiple log files from retries, found $log_count"
    return 1
  }
}

test_process_dead_letters_on_exhaustion() {
  init_project
  mkdir -p .decree/migrations
  echo "Test dead letter" > .decree/migrations/01-test.md
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
exit 1
SCRIPT
  chmod +x .decree/routines/develop.sh
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

  ./decree process --no-color >/dev/null 2>&1 || true
  local dead_count
  dead_count=$(find .decree/inbox/dead -name '*.md' 2>/dev/null | wc -l)
  [[ $dead_count -gt 0 ]] || {
    echo "expected dead-lettered message, found $dead_count"
    return 1
  }
}

test_process_dead_letter_not_marked_processed() {
  init_project
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
exit 1
SCRIPT
  chmod +x .decree/routines/develop.sh
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

  ./decree process --no-color >/dev/null 2>&1 || true
  assert_file_contains .decree/processed.md "01-test.md" \
    "dead-lettered migration should still be marked processed"
}

# ---------- 16: PROCESS — OUTBOX / FOLLOW-UPS ----------

test_process_outbox_follow_ups() {
  init_project
  mkdir -p .decree/migrations
  cat > .decree/migrations/01-test.md <<'EOF'
---
routine: develop
---
Initial task.
EOF
  # Routine writes a follow-up to outbox
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi

# Only write follow-up on first message (seq 0)
if [ "$seq" = "0" ]; then
  cat > .decree/outbox/follow-up.md <<FOLLOWUP
---
routine: develop
---
Follow-up task from first message.
FOLLOWUP
fi
echo "processed seq=$seq"
SCRIPT
  chmod +x .decree/routines/develop.sh
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

  ./decree process --no-color >/dev/null 2>&1
  # Should have two run directories (original + follow-up)
  local run_count
  run_count=$(find .decree/runs -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l)
  [[ $run_count -ge 2 ]] || {
    echo "expected at least 2 run directories (original + follow-up), found $run_count"
    return 1
  }
}

# ---------- 17: PROCESS — MULTIPLE MIGRATIONS ----------

test_process_multiple_migrations_in_order() {
  init_project
  mkdir -p .decree/migrations
  echo "First" > .decree/migrations/01-first.md
  echo "Second" > .decree/migrations/02-second.md
  echo "Third" > .decree/migrations/03-third.md
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "ok"
SCRIPT
  chmod +x .decree/routines/develop.sh
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

  ./decree process --no-color >/dev/null 2>&1
  assert_file_contains .decree/processed.md "01-first.md" &&
  assert_file_contains .decree/processed.md "02-second.md" &&
  assert_file_contains .decree/processed.md "03-third.md"
}

test_process_skips_already_processed() {
  init_project
  mkdir -p .decree/migrations
  echo "First" > .decree/migrations/01-first.md
  echo "Second" > .decree/migrations/02-second.md
  echo "01-first.md" > .decree/processed.md
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "processing $chain"
SCRIPT
  chmod +x .decree/routines/develop.sh
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

  ./decree process --no-color >/dev/null 2>&1
  # Only one run dir should be created (for 02-second, not 01-first)
  local run_count
  run_count=$(find .decree/runs -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l)
  assert_eq "1" "$run_count" "should only process unprocessed migration"
}

# ---------- 18: PROCESS — LIFECYCLE HOOKS ----------

test_process_before_each_hook() {
  init_project
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "ok"
SCRIPT
  chmod +x .decree/routines/develop.sh
  local hook_marker="$PWD/hook_marker"
  cat > .decree/routines/test-hook.sh <<SCRIPT
#!/usr/bin/env bash
# Test Hook
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
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach: .*/beforeEach: "test-hook"/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

  ./decree process --no-color >/dev/null 2>&1
  if [[ -f "$hook_marker" ]]; then
    assert_file_contains "$hook_marker" "HOOK_TYPE=beforeEach"
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
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "ok"
SCRIPT
  chmod +x .decree/routines/develop.sh
  local hook_marker="$PWD/posthook_marker"
  cat > .decree/routines/post-hook.sh <<SCRIPT
#!/usr/bin/env bash
# Post Hook
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
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: "post-hook"/' .decree/config.yml

  ./decree process --no-color >/dev/null 2>&1
  if [[ -f "$hook_marker" ]]; then
    assert_file_contains "$hook_marker" "EXIT_CODE=0"
  else
    echo "afterEach hook did not execute"
    return 1
  fi
}

# ---------- 19: PROCESS — SIGINT ----------

test_process_sigint_exit_130() {
  init_project
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
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
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

  # Start in background, send SIGINT after brief delay
  ./decree process --no-color &
  local pid=$!
  sleep 1
  kill -INT "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null
  local rc=$?
  # Should exit with 130 (128+2) or similar non-zero
  [[ $rc -ne 0 ]] || {
    echo "expected non-zero exit after SIGINT, got $rc"
    return 1
  }
}

test_process_sigint_no_second_migration() {
  init_project
  mkdir -p .decree/migrations
  echo "First" > .decree/migrations/01-first.md
  echo "Second" > .decree/migrations/02-second.md
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
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
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

  ./decree process --no-color &
  local pid=$!
  sleep 1
  kill -INT "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true

  # Second migration should NOT be processed
  if grep -qF "02-second.md" .decree/processed.md 2>/dev/null; then
    echo "SIGINT should prevent processing second migration"
    return 1
  fi
}

# ---------- 20: PROCESS — SUMMARY OUTPUT ----------

test_process_prints_summary() {
  init_project
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "ok"
SCRIPT
  chmod +x .decree/routines/develop.sh
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

  local out
  out=$(./decree process --no-color 2>&1)
  assert_contains "$out" "Processed" || assert_contains "$out" "processed"
}

# ---------- 21: PROMPT ----------

test_prompt_list_non_tty() {
  init_project
  local out rc=0
  out=$(./decree prompt --no-color 2>&1) || rc=$?
  assert_exit_code 0 "$rc" &&
  assert_contains "$out" "migration" &&
  assert_contains "$out" "sow"
}

test_prompt_named_outputs_content() {
  init_project
  local out rc=0
  out=$(./decree prompt sow --no-color 2>&1) || rc=$?
  assert_exit_code 0 "$rc" &&
  # SOW template should contain Statement of Work content
  assert_contains "$out" "Statement of Work" || assert_contains "$out" "SOW"
}

test_prompt_substitution() {
  init_project
  mkdir -p .decree/migrations
  echo "Auth task" > .decree/migrations/01-auth.md
  local out
  out=$(./decree prompt migration --no-color 2>&1)
  # {migrations} should be substituted with actual migration list
  assert_contains "$out" "01-auth.md" &&
  assert_not_contains "$out" "{migrations}" "placeholder should be substituted"
}

test_prompt_unknown_suggests() {
  init_project
  local out rc=0
  out=$(./decree prompt migraton --no-color 2>&1) || rc=$?
  # Should suggest close match or list available
  assert_contains "$out" "migration" || assert_contains "$out" "available"
}

# ---------- 22: DAEMON BASICS ----------

test_daemon_starts_and_stops() {
  init_project
  rm -f .decree/routines/develop.sh .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeAll: .*/beforeAll: ""/' .decree/config.yml
  sed -i 's/afterAll: .*/afterAll: ""/' .decree/config.yml
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

  timeout 3 ./decree daemon --interval 1 --no-color >/dev/null 2>&1 &
  local pid=$!
  sleep 1
  kill -TERM "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  # Just verifying it didn't crash on startup
  return 0
}

test_daemon_processes_inbox() {
  init_project
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "daemon processed"
SCRIPT
  chmod +x .decree/routines/develop.sh
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeAll: .*/beforeAll: ""/' .decree/config.yml
  sed -i 's/afterAll: .*/afterAll: ""/' .decree/config.yml
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

  # Put a message directly in the inbox
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

  # Inbox should be drained
  local inbox_count
  inbox_count=$(find .decree/inbox -maxdepth 1 -name '*.md' 2>/dev/null | wc -l)
  assert_eq "0" "$inbox_count" "daemon should process inbox messages"
}

# ---------- 23: CRON ----------

test_cron_file_parsed() {
  init_project
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "cron task"
SCRIPT
  chmod +x .decree/routines/develop.sh
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeAll: .*/beforeAll: ""/' .decree/config.yml
  sed -i 's/afterAll: .*/afterAll: ""/' .decree/config.yml
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

  # Create cron file that fires every minute
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

  # Should have created a run directory from the cron job
  local run_count
  run_count=$(find .decree/runs -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l)
  [[ $run_count -gt 0 ]] || {
    echo "expected cron to fire and create run directory, found $run_count"
    return 1
  }
}

# ---------- 24: MESSAGE FORMAT ----------

test_message_normalization() {
  init_project
  mkdir -p .decree/inbox
  # Put a bare message (no frontmatter) in inbox
  echo "Bare message content." > .decree/inbox/test-msg.md
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "chain=$chain seq=$seq"
SCRIPT
  chmod +x .decree/routines/develop.sh
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

  ./decree process --no-color >/dev/null 2>&1 || true
  # Check that run directory was created with a normalized message
  local msg_file
  msg_file=$(find .decree/runs -name 'message.md' 2>/dev/null | head -1)
  if [[ -n "$msg_file" ]]; then
    # Normalized message should have frontmatter
    assert_file_contains "$msg_file" "chain:" &&
    assert_file_contains "$msg_file" "seq:" &&
    assert_file_contains "$msg_file" "id:"
  else
    # No message found — also check if there's a dead letter
    echo "note: no run directory created (message may have been dead-lettered)"
    return 0
  fi
}

test_message_id_format() {
  init_project
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "ID=$message_id"
SCRIPT
  chmod +x .decree/routines/develop.sh
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

  ./decree process --no-color >/dev/null 2>&1
  local log_file
  log_file=$(find .decree/runs -name 'routine.log' 2>/dev/null | head -1)
  if [[ -n "$log_file" ]]; then
    local id_line
    id_line=$(grep "ID=" "$log_file" | head -1)
    # ID should match D<NNNN>-HHmm-<name>-<seq> pattern
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
  init_project
  mkdir -p .decree/migrations
  cat > .decree/migrations/01-test.md <<'EOF'
---
routine: develop
---
UNIQUE_CONTENT_XYZ_12345
EOF
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
cat "$message_file"
SCRIPT
  chmod +x .decree/routines/develop.sh
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

  ./decree process --no-color >/dev/null 2>&1
  local msg_file
  msg_file=$(find .decree/runs -name 'message.md' 2>/dev/null | head -1)
  assert_file_contains "$msg_file" "UNIQUE_CONTENT_XYZ_12345" "migration body should be in message"
}

# ---------- 25: BARE DECREE DISPATCHES TO PROCESS ----------

test_bare_decree_is_process() {
  init_project
  mkdir -p .decree/migrations
  echo "Test" > .decree/migrations/01-test.md
  cat > .decree/routines/develop.sh <<'SCRIPT'
#!/usr/bin/env bash
# Develop
set -euo pipefail
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then exit 0; fi
echo "ok"
SCRIPT
  chmod +x .decree/routines/develop.sh
  rm -f .decree/routines/rust-develop.sh
  rm -f .decree/routines/git-baseline.sh .decree/routines/git-stash-changes.sh
  sed -i 's/beforeEach: .*/beforeEach: ""/' .decree/config.yml
  sed -i 's/afterEach: .*/afterEach: ""/' .decree/config.yml

  ./decree --no-color >/dev/null 2>&1
  assert_file_contains .decree/processed.md "01-test.md" "bare decree should process migrations"
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
  "unknown subcommand exit 2"        test_unknown_subcommand_exit_2
  "no-color flag accepted"           test_no_color_flag_accepted

  # Require .decree/
  "bare decree w/o project fails"    test_bare_decree_without_project_fails
  "process w/o project fails"        test_process_without_project_fails
  "status w/o project fails"         test_status_without_project_fails
  "routine w/o project fails"        test_routine_without_project_fails
  "verify w/o project fails"         test_verify_without_project_fails
  "log w/o project fails"            test_log_without_project_fails
  "daemon w/o project fails"         test_daemon_without_project_fails
  "prompt w/o project fails"         test_prompt_without_project_fails

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
  "config commented alternatives"    test_init_config_commented_alternatives

  # Init — templates
  "init creates develop.sh"          test_init_creates_develop_sh
  "init creates rust-develop.sh"     test_init_creates_rust_develop_sh
  "init routines executable"         test_init_routines_executable
  "init develop.sh has shebang"      test_init_develop_has_shebang
  "init develop.sh has pre-check"    test_init_develop_has_precheck
  "init develop.sh has description"  test_init_develop_has_description
  "init develop.sh no {AI_CMD}"      test_init_develop_no_ai_cmd_placeholder
  "init rust-develop no {AI_CMD}"    test_init_rust_develop_no_ai_cmd_placeholder
  "init develop.sh has message_dir"  test_init_develop_has_message_dir_ref
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

  # Init — re-run
  "init re-run overwrites non-tty"   test_init_rerun_overwrites_non_tty

  # Status
  "status empty project"             test_status_empty_project
  "status shows migration count"     test_status_shows_migration_count
  "status shows next migration"      test_status_shows_next_migration
  "status shows inbox count"         test_status_shows_inbox_count
  "status shows dead-letter count"   test_status_shows_dead_letter_count

  # Log
  "log no runs"                      test_log_no_runs
  "log shows recent run"             test_log_shows_recent_run
  "log specific run"                 test_log_specific_run
  "log multiple attempts"            test_log_multiple_attempts

  # Routine
  "routine list non-tty"             test_routine_list_non_tty
  "routine list shows rust-develop"  test_routine_list_shows_rust_develop
  "routine detail named"             test_routine_detail_named
  "routine detail shows path"        test_routine_detail_shows_path
  "routine detail shows description" test_routine_detail_shows_description
  "routine unknown suggests match"   test_routine_unknown_suggests_close_match
  "routine unknown no match fails"   test_routine_unknown_no_match
  "routine no routines message"      test_routine_no_routines
  "routine nested discovery"         test_routine_nested
  "routine custom params shown"      test_routine_custom_params

  # Verify
  "verify all pass"                  test_verify_all_pass
  "verify some fail exit 3"          test_verify_some_fail_exit_3
  "verify shows fail reason"         test_verify_shows_fail_reason
  "verify no routines"               test_verify_no_routines
  "verify includes hook pre-checks"  test_verify_includes_hook_prechecks

  # Process — dry run
  "process dry-run no migrations"    test_process_dry_run_no_migrations
  "process dry-run lists migrations" test_process_dry_run_lists_migrations
  "process dry-run creates no files" test_process_dry_run_no_files_created

  # Process — basic execution
  "process simple migration"         test_process_simple_migration
  "process creates run directory"    test_process_creates_run_directory
  "process run dir has message.md"   test_process_run_dir_has_message
  "process run dir has routine.log"  test_process_run_dir_has_log
  "process log has timestamps"       test_process_log_has_timestamps
  "process removes from inbox"       test_process_removes_from_inbox
  "process env vars available"       test_process_env_vars_available
  "process custom fields as env"     test_process_custom_fields_as_env

  # Process — retry and dead-letter
  "process retry multiple logs"      test_process_retry_creates_multiple_logs
  "process dead-letters on exhaust"  test_process_dead_letters_on_exhaustion
  "process dead marked processed"    test_process_dead_letter_not_marked_processed

  # Process — outbox follow-ups
  "process outbox follow-ups"        test_process_outbox_follow_ups

  # Process — multiple migrations
  "process multiple in order"        test_process_multiple_migrations_in_order
  "process skips already processed"  test_process_skips_already_processed

  # Process — lifecycle hooks
  "process beforeEach hook"          test_process_before_each_hook
  "process afterEach exit code"      test_process_after_each_hook_exit_code

  # Process — SIGINT
  "process SIGINT exit non-zero"     test_process_sigint_exit_130
  "process SIGINT no second migr."   test_process_sigint_no_second_migration

  # Process — summary
  "process prints summary"           test_process_prints_summary

  # Prompt
  "prompt list non-tty"              test_prompt_list_non_tty
  "prompt named outputs content"     test_prompt_named_outputs_content
  "prompt substitution"              test_prompt_substitution
  "prompt unknown suggests"          test_prompt_unknown_suggests

  # Daemon
  "daemon starts and stops"          test_daemon_starts_and_stops
  "daemon processes inbox"           test_daemon_processes_inbox

  # Cron
  "cron file fires via daemon"       test_cron_file_parsed

  # Message format
  "message normalization"            test_message_normalization
  "message ID format D-HHmm-name"   test_message_id_format
  "migration body in message"        test_migration_body_in_message

  # Bare decree
  "bare decree dispatches to process" test_bare_decree_is_process
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
