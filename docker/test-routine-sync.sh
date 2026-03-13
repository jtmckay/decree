#!/usr/bin/env bash
set -euo pipefail

# test-routine-sync.sh — Standalone test harness for routine-sync.sh
# Runs locally with temp dirs, no Docker required. Requires: yq, flock

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SYNC_SCRIPT="$SCRIPT_DIR/routine-sync.sh"

PASS=0
FAIL=0
ERRORS=""

# --- Helpers ---

setup() {
  TEST_DIR=$(mktemp -d)
  export ROUTINES_DIR="$TEST_DIR/routines"
  export WORK_DIR="$TEST_DIR/work"
  mkdir -p "$ROUTINES_DIR" "$WORK_DIR/.decree/routines"
}

teardown() {
  rm -rf "$TEST_DIR"
}

run_sync() {
  bash "$SYNC_SCRIPT"
}

assert_file_exists() {
  if [[ ! -f "$1" ]]; then
    echo "  FAIL: expected file to exist: $1" >&2
    return 1
  fi
}

assert_file_not_exists() {
  if [[ -f "$1" ]]; then
    echo "  FAIL: expected file NOT to exist: $1" >&2
    return 1
  fi
}

assert_executable() {
  if [[ ! -x "$1" ]]; then
    echo "  FAIL: expected file to be executable: $1" >&2
    return 1
  fi
}

assert_file_contains() {
  if ! grep -q "$2" "$1" 2>/dev/null; then
    echo "  FAIL: expected '$1' to contain '$2'" >&2
    return 1
  fi
}

assert_file_content_equals() {
  local actual
  actual=$(cat "$1")
  if [[ "$actual" != "$2" ]]; then
    echo "  FAIL: expected content of '$1' to be '$2', got '$actual'" >&2
    return 1
  fi
}

assert_yml_value() {
  local path="$1"
  local expected="$2"
  local actual
  actual=$(yq -r "$path" "$ROUTINES_DIR/routines.yml" 2>/dev/null)
  if [[ "$actual" != "$expected" ]]; then
    echo "  FAIL: yq '$path' = '$actual', expected '$expected'" >&2
    return 1
  fi
}

run_test() {
  local name="$1"
  local func="$2"
  setup
  if $func; then
    echo "PASS: $name"
    PASS=$((PASS + 1))
  else
    echo "FAIL: $name"
    FAIL=$((FAIL + 1))
    ERRORS="$ERRORS\n  - $name"
  fi
  teardown
}

# ============================================================
# Test Cases
# ============================================================

test_first_run_empty_state() {
  export DECREE_CONTAINER="test1"
  echo '#!/bin/bash' > "$ROUTINES_DIR/deploy.sh"
  echo '#!/bin/bash' > "$ROUTINES_DIR/notify.sh"
  chmod +x "$ROUTINES_DIR/deploy.sh" "$ROUTINES_DIR/notify.sh"

  run_sync

  assert_file_exists "$ROUTINES_DIR/routines.yml" &&
  assert_yml_value '.routines.deploy.file' 'deploy.sh' &&
  assert_yml_value '.routines.notify.file' 'notify.sh' &&
  assert_file_exists "$WORK_DIR/.decree/routines/deploy.sh" &&
  assert_file_exists "$WORK_DIR/.decree/routines/notify.sh"
}

test_auto_discovery() {
  export DECREE_CONTAINER="test1"
  echo '#!/bin/bash' > "$ROUTINES_DIR/deploy.sh"
  chmod +x "$ROUTINES_DIR/deploy.sh"
  run_sync

  # Add a new routine between runs
  echo '#!/bin/bash' > "$ROUTINES_DIR/audit.sh"
  chmod +x "$ROUTINES_DIR/audit.sh"
  run_sync

  assert_yml_value '.routines.audit.file' 'audit.sh' &&
  assert_file_exists "$WORK_DIR/.decree/routines/audit.sh"
}

test_sync_out() {
  export DECREE_CONTAINER="test1"
  echo '#!/bin/bash
echo original' > "$ROUTINES_DIR/deploy.sh"
  chmod +x "$ROUTINES_DIR/deploy.sh"
  run_sync

  # Modify locally
  echo '#!/bin/bash
echo modified' > "$WORK_DIR/.decree/routines/deploy.sh"
  run_sync

  assert_file_exists "$ROUTINES_DIR/deploy__test1.sh" &&
  assert_file_contains "$ROUTINES_DIR/deploy__test1.sh" "modified" &&
  assert_yml_value '.routines.deploy.variants.test1' 'deploy__test1.sh'
}

test_sync_in_with_variant() {
  export DECREE_CONTAINER="test1"
  echo '#!/bin/bash
echo base' > "$ROUTINES_DIR/deploy.sh"
  echo '#!/bin/bash
echo variant' > "$ROUTINES_DIR/deploy__test1.sh"
  chmod +x "$ROUTINES_DIR/deploy.sh" "$ROUTINES_DIR/deploy__test1.sh"

  # Pre-populate yml with variant
  cat > "$ROUTINES_DIR/routines.yml" <<'EOF'
routines:
  deploy:
    file: deploy.sh
    variants:
      test1: deploy__test1.sh
    ignore: []
EOF

  run_sync

  assert_file_contains "$WORK_DIR/.decree/routines/deploy.sh" "variant"
}

test_variant_update() {
  export DECREE_CONTAINER="test1"
  echo '#!/bin/bash
echo base' > "$ROUTINES_DIR/deploy.sh"
  chmod +x "$ROUTINES_DIR/deploy.sh"
  run_sync

  # First modification -> creates variant
  echo '#!/bin/bash
echo v1' > "$WORK_DIR/.decree/routines/deploy.sh"
  run_sync
  assert_file_contains "$ROUTINES_DIR/deploy__test1.sh" "v1" || return 1

  # Second modification -> updates variant
  echo '#!/bin/bash
echo v2' > "$WORK_DIR/.decree/routines/deploy.sh"
  run_sync
  assert_file_contains "$ROUTINES_DIR/deploy__test1.sh" "v2"
}

test_project_local_ignore() {
  export DECREE_CONTAINER="test1"
  # Base routine exists in shared
  echo '#!/bin/bash
echo shared' > "$ROUTINES_DIR/develop.sh"
  chmod +x "$ROUTINES_DIR/develop.sh"

  # Project already has its own version (before first sync)
  echo '#!/bin/bash
echo project-local' > "$WORK_DIR/.decree/routines/develop.sh"

  run_sync

  # Local should be preserved
  assert_file_contains "$WORK_DIR/.decree/routines/develop.sh" "project-local" &&
  assert_yml_value '.routines.develop.ignore[0]' 'test1'
}

test_deprecation() {
  export DECREE_CONTAINER="test1"
  echo '#!/bin/bash' > "$ROUTINES_DIR/deploy.sh"
  chmod +x "$ROUTINES_DIR/deploy.sh"
  run_sync

  # Remove base file
  rm "$ROUTINES_DIR/deploy.sh"
  run_sync

  assert_yml_value '.routines.deploy.deprecated' 'true'
}

test_undeprecation() {
  export DECREE_CONTAINER="test1"
  echo '#!/bin/bash' > "$ROUTINES_DIR/deploy.sh"
  chmod +x "$ROUTINES_DIR/deploy.sh"
  run_sync

  # Remove and re-run to mark deprecated
  rm "$ROUTINES_DIR/deploy.sh"
  run_sync
  assert_yml_value '.routines.deploy.deprecated' 'true' || return 1

  # Restore
  echo '#!/bin/bash' > "$ROUTINES_DIR/deploy.sh"
  chmod +x "$ROUTINES_DIR/deploy.sh"
  run_sync

  # deprecated flag should be removed (yq returns null for missing keys)
  local val
  val=$(yq -r '.routines.deploy.deprecated // "null"' "$ROUTINES_DIR/routines.yml")
  if [[ "$val" != "null" && "$val" != "false" ]]; then
    echo "  FAIL: expected deprecated to be removed, got '$val'" >&2
    return 1
  fi
}

test_cleanup_on_entry_removal() {
  export DECREE_CONTAINER="test1"
  echo '#!/bin/bash' > "$ROUTINES_DIR/deploy.sh"
  echo '#!/bin/bash' > "$ROUTINES_DIR/notify.sh"
  chmod +x "$ROUTINES_DIR/deploy.sh" "$ROUTINES_DIR/notify.sh"
  run_sync

  assert_file_exists "$WORK_DIR/.decree/routines/deploy.sh" || return 1
  assert_file_exists "$WORK_DIR/.decree/routines/notify.sh" || return 1

  # Remove deploy completely: base file AND entry (simulating user action)
  rm "$ROUTINES_DIR/deploy.sh"
  yq -i 'del(.routines.deploy)' "$ROUTINES_DIR/routines.yml"
  run_sync

  assert_file_not_exists "$WORK_DIR/.decree/routines/deploy.sh" &&
  assert_file_exists "$WORK_DIR/.decree/routines/notify.sh"
}

test_concurrent_safety() {
  export DECREE_CONTAINER="test1"
  echo '#!/bin/bash' > "$ROUTINES_DIR/deploy.sh"
  chmod +x "$ROUTINES_DIR/deploy.sh"

  # Run two syncs concurrently
  DECREE_CONTAINER="containerA" bash "$SYNC_SCRIPT" &
  local pid1=$!
  DECREE_CONTAINER="containerB" bash "$SYNC_SCRIPT" &
  local pid2=$!
  wait "$pid1" "$pid2"

  # routines.yml should be valid YAML
  yq '.' "$ROUTINES_DIR/routines.yml" >/dev/null 2>&1
}

test_permissions() {
  export DECREE_CONTAINER="test1"
  echo '#!/bin/bash' > "$ROUTINES_DIR/deploy.sh"
  chmod +x "$ROUTINES_DIR/deploy.sh"
  run_sync

  assert_executable "$WORK_DIR/.decree/routines/deploy.sh"
}

test_invalid_container_double_underscore() {
  export DECREE_CONTAINER="my__bad"
  echo '#!/bin/bash' > "$ROUTINES_DIR/deploy.sh"
  chmod +x "$ROUTINES_DIR/deploy.sh"

  if run_sync 2>/dev/null; then
    echo "  FAIL: expected error for DECREE_CONTAINER='my__bad'" >&2
    return 1
  fi
}

test_invalid_container_slash() {
  export DECREE_CONTAINER="my/bad"
  echo '#!/bin/bash' > "$ROUTINES_DIR/deploy.sh"
  chmod +x "$ROUTINES_DIR/deploy.sh"

  if run_sync 2>/dev/null; then
    echo "  FAIL: expected error for DECREE_CONTAINER='my/bad'" >&2
    return 1
  fi
}

test_invalid_container_whitespace() {
  export DECREE_CONTAINER="my bad"
  echo '#!/bin/bash' > "$ROUTINES_DIR/deploy.sh"
  chmod +x "$ROUTINES_DIR/deploy.sh"

  if run_sync 2>/dev/null; then
    echo "  FAIL: expected error for DECREE_CONTAINER='my bad'" >&2
    return 1
  fi
}

test_external_base_update() {
  export DECREE_CONTAINER="test1"
  echo '#!/bin/bash
echo v1' > "$ROUTINES_DIR/deploy.sh"
  chmod +x "$ROUTINES_DIR/deploy.sh"
  run_sync

  assert_file_contains "$WORK_DIR/.decree/routines/deploy.sh" "v1" || return 1

  # Update the base externally (no local modification — local matches old base)
  echo '#!/bin/bash
echo v2' > "$ROUTINES_DIR/deploy.sh"
  run_sync

  assert_file_contains "$WORK_DIR/.decree/routines/deploy.sh" "v2"
}

test_multi_container_isolation() {
  # Container A syncs and modifies
  export DECREE_CONTAINER="containerA"
  echo '#!/bin/bash
echo base' > "$ROUTINES_DIR/deploy.sh"
  chmod +x "$ROUTINES_DIR/deploy.sh"
  run_sync

  echo '#!/bin/bash
echo modified-by-A' > "$WORK_DIR/.decree/routines/deploy.sh"
  run_sync

  assert_file_exists "$ROUTINES_DIR/deploy__containerA.sh" || return 1

  # Container B syncs — should get base, not A's variant
  export DECREE_CONTAINER="containerB"
  # Create a fresh work dir for container B
  local work_b="$TEST_DIR/work_b"
  mkdir -p "$work_b/.decree/routines"
  export WORK_DIR="$work_b"
  run_sync

  assert_file_contains "$work_b/.decree/routines/deploy.sh" "base" &&
  assert_file_not_exists "$ROUTINES_DIR/deploy__containerB.sh"
}

# ============================================================
# Run all tests
# ============================================================

echo "=== routine-sync.sh test suite ==="
echo ""

run_test "1.  First run, empty state" test_first_run_empty_state
run_test "2.  Auto-discovery" test_auto_discovery
run_test "3.  Sync-out" test_sync_out
run_test "4.  Sync-in with variant" test_sync_in_with_variant
run_test "5.  Variant update" test_variant_update
run_test "6.  Project-local ignore" test_project_local_ignore
run_test "7.  Deprecation" test_deprecation
run_test "8.  Un-deprecation" test_undeprecation
run_test "9.  Cleanup on entry removal" test_cleanup_on_entry_removal
run_test "10. Concurrent safety" test_concurrent_safety
run_test "11. Permissions" test_permissions
run_test "12a. Invalid DECREE_CONTAINER (double underscore)" test_invalid_container_double_underscore
run_test "12b. Invalid DECREE_CONTAINER (slash)" test_invalid_container_slash
run_test "12c. Invalid DECREE_CONTAINER (whitespace)" test_invalid_container_whitespace
run_test "13. External base update" test_external_base_update
run_test "14. Multi-container isolation" test_multi_container_isolation

echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="

if [[ $FAIL -gt 0 ]]; then
  echo -e "\nFailed tests:$ERRORS"
  exit 1
fi
