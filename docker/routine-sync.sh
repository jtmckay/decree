#!/usr/bin/env bash
set -euo pipefail

# routine-sync.sh — Bidirectional routine synchronization between
# /routines (shared host directory) and /work/.decree/routines/ (container).
#
# Environment:
#   DECREE_CONTAINER — container identity (required)
#   ROUTINES_DIR     — override for /routines (used by tests)
#   WORK_DIR         — override for /work (used by tests)

ROUTINES_DIR="${ROUTINES_DIR:-/routines}"
WORK_DIR="${WORK_DIR:-/work}"
LOCAL_DIR="$WORK_DIR/.decree/routines"
YML="$ROUTINES_DIR/routines.yml"
LOCKFILE="$ROUTINES_DIR/.lock"

# --- Validation ---

DECREE_CONTAINER="${DECREE_CONTAINER:-}"

if [[ -z "$DECREE_CONTAINER" ]]; then
  echo "ERROR: DECREE_CONTAINER must not be empty" >&2
  exit 1
fi
if [[ "$DECREE_CONTAINER" == *"__"* ]]; then
  echo "ERROR: DECREE_CONTAINER must not contain '__': $DECREE_CONTAINER" >&2
  exit 1
fi
if ! [[ "$DECREE_CONTAINER" =~ ^[a-zA-Z0-9_-]+$ ]]; then
  echo "ERROR: DECREE_CONTAINER contains invalid characters: $DECREE_CONTAINER" >&2
  exit 1
fi

# --- Helpers ---

file_hash() {
  sha256sum "$1" 2>/dev/null | cut -d' ' -f1
}

ensure_yml() {
  if [[ ! -f "$YML" ]]; then
    echo "routines: {}" > "$YML"
  fi
}

# Get routine names from yml as newline-separated list
yml_routine_names() {
  yq -r '.routines // {} | keys | .[]' "$YML" 2>/dev/null || true
}

# Check if routine is deprecated
is_deprecated() {
  local name="$1"
  local val
  val=$(yq -r ".routines.\"$name\".deprecated // false" "$YML" 2>/dev/null)
  [[ "$val" == "true" ]]
}

# Check if container is in ignore list
is_ignored() {
  local name="$1"
  local container="$2"
  local result
  result=$(yq -r ".routines.\"$name\".ignore // [] | .[] | select(. == \"$container\")" "$YML" 2>/dev/null)
  [[ -n "$result" ]]
}

# Get variant filename for this container, or empty
get_variant() {
  local name="$1"
  local container="$2"
  yq -r ".routines.\"$name\".variants.\"$container\" // \"\"" "$YML" 2>/dev/null
}

# --- Ensure local routines dir exists ---
mkdir -p "$LOCAL_DIR"

# --- Tracking files for sync state ---
MANAGED_FILE="$WORK_DIR/.decree/.routine-sync-managed"
HASHES_FILE="$WORK_DIR/.decree/.routine-sync-hashes"
OLD_MANAGED=""
if [[ -f "$MANAGED_FILE" ]]; then
  OLD_MANAGED=$(cat "$MANAGED_FILE")
fi

# Get the last-synced hash for a routine (empty if not recorded)
get_synced_hash() {
  local name="$1"
  grep "^${name}:" "$HASHES_FILE" 2>/dev/null | cut -d: -f2 || true
}

# --- Snapshot routine names before sync (for concurrent cleanup) ---
ensure_yml
SNAPSHOT_NAMES=$(yml_routine_names)

# ============================================================
# Step 1 — Auto-discover (under flock)
# ============================================================
(
  flock 9

  ensure_yml

  # Scan base .sh files (exclude variants with __)
  for f in "$ROUTINES_DIR"/*.sh; do
    [[ -f "$f" ]] || continue
    base=$(basename "$f")
    # Skip variant files
    [[ "$base" == *"__"* ]] && continue
    name="${base%.sh}"

    # Add if not in yml
    existing=$(yq -r ".routines.\"$name\" // \"null\"" "$YML" 2>/dev/null)
    if [[ "$existing" == "null" ]]; then
      yq -i ".routines.\"$name\" = {\"file\": \"$base\", \"variants\": {}, \"ignore\": []}" "$YML"
    fi
  done

  # Mark deprecated / un-deprecate
  for name in $(yml_routine_names); do
    base_file=$(yq -r ".routines.\"$name\".file // \"\"" "$YML" 2>/dev/null)
    if [[ -z "$base_file" ]]; then
      continue
    fi
    if [[ ! -f "$ROUTINES_DIR/$base_file" ]]; then
      # Mark deprecated
      yq -i ".routines.\"$name\".deprecated = true" "$YML"
    else
      # Remove deprecated flag if present
      if is_deprecated "$name"; then
        yq -i "del(.routines.\"$name\".deprecated)" "$YML"
      fi
    fi
  done

) 9>"$LOCKFILE"

# ============================================================
# Step 2 — Sync-out (under flock)
# ============================================================
(
  flock 9

  for name in $(yml_routine_names); do
    # Skip deprecated
    is_deprecated "$name" && continue
    # Skip if in ignore list
    is_ignored "$name" "$DECREE_CONTAINER" && continue
    # Skip if local file doesn't exist
    [[ -f "$LOCAL_DIR/$name.sh" ]] || continue
    # Only sync out routines that were previously managed (synced in)
    if ! echo "$OLD_MANAGED" | grep -qx "$name"; then
      continue
    fi

    # Skip if local file hasn't been modified since last sync-in
    local_hash=$(file_hash "$LOCAL_DIR/$name.sh")
    synced_hash=$(get_synced_hash "$name")
    if [[ -n "$synced_hash" && "$local_hash" == "$synced_hash" ]]; then
      continue
    fi

    # Determine comparison source
    variant=$(get_variant "$name" "$DECREE_CONTAINER")
    if [[ -n "$variant" && -f "$ROUTINES_DIR/$variant" ]]; then
      source_file="$ROUTINES_DIR/$variant"
    else
      base_file=$(yq -r ".routines.\"$name\".file" "$YML")
      source_file="$ROUTINES_DIR/$base_file"
    fi

    # Compare local against source
    source_hash=$(file_hash "$source_file")

    if [[ "$local_hash" != "$source_hash" ]]; then
      variant_file="${name}__${DECREE_CONTAINER}.sh"
      cp "$LOCAL_DIR/$name.sh" "$ROUTINES_DIR/$variant_file"
      chmod +x "$ROUTINES_DIR/$variant_file"
      yq -i ".routines.\"$name\".variants.\"$DECREE_CONTAINER\" = \"$variant_file\"" "$YML"
    fi
  done

) 9>"$LOCKFILE"

# ============================================================
# Step 3 — Sync-in
# ============================================================
: > "$MANAGED_FILE"  # Clear tracking files for this run
: > "$HASHES_FILE"

for name in $(yml_routine_names); do
  # Skip deprecated
  is_deprecated "$name" && continue
  # Skip if in ignore list
  is_ignored "$name" "$DECREE_CONTAINER" && continue

  # Determine source
  variant=$(get_variant "$name" "$DECREE_CONTAINER")
  if [[ -n "$variant" && -f "$ROUTINES_DIR/$variant" ]]; then
    source_file="$ROUTINES_DIR/$variant"
    has_variant=true
  else
    base_file=$(yq -r ".routines.\"$name\".file" "$YML")
    source_file="$ROUTINES_DIR/$base_file"
    has_variant=false
  fi

  # Project-local detection: only for routines not previously managed.
  # If the routine was managed before (in OLD_MANAGED), the local file is a
  # synced copy, not a project-local file — always update it.
  if [[ -f "$LOCAL_DIR/$name.sh" && "$has_variant" == "false" ]]; then
    if ! echo "$OLD_MANAGED" | grep -qx "$name"; then
      local_hash=$(file_hash "$LOCAL_DIR/$name.sh")
      source_hash=$(file_hash "$source_file")
      if [[ "$local_hash" != "$source_hash" ]]; then
        # Add to ignore list (under flock)
        (
          flock 9
          yq -i ".routines.\"$name\".ignore += [\"$DECREE_CONTAINER\"]" "$YML"
        ) 9>"$LOCKFILE"
        continue
      fi
    fi
  fi

  # Track as managed (only routines that are actually synced, not project-local)
  echo "$name" >> "$MANAGED_FILE"

  # Copy source to local
  cp "$source_file" "$LOCAL_DIR/$name.sh"
  chmod +x "$LOCAL_DIR/$name.sh"

  # Record the hash of what was synced in (for sync-out change detection)
  echo "$name:$(file_hash "$LOCAL_DIR/$name.sh")" >> "$HASHES_FILE"
done

# ============================================================
# Step 4 — Cleanup (entries removed from routines.yml)
# ============================================================
CURRENT_NAMES=$(yml_routine_names)

# Part A: concurrent cleanup — entries removed during this run
for name in $SNAPSHOT_NAMES; do
  if ! echo "$CURRENT_NAMES" | grep -qx "$name"; then
    rm -f "$LOCAL_DIR/$name.sh"
  fi
done

# Part B: between-run cleanup — entries removed since last run
for name in $OLD_MANAGED; do
  if ! echo "$CURRENT_NAMES" | grep -qx "$name"; then
    rm -f "$LOCAL_DIR/$name.sh"
  fi
done
