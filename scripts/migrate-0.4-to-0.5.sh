#!/usr/bin/env bash
# Upgrade a decree 0.4 project's .decree/ to the 0.5 layout, once (docs/0.5-spec.md, M5.4).
#
# Run from the project root: scripts/migrate-0.4-to-0.5.sh
#
# - Leaves migrations/ and processed.md untouched.
# - Moves pending outbox/*.md into inbox/ (pending inbox/*.md stay where they are).
# - config.yml: default_routine -> default_machine, routine_source -> shared_source,
#   and removes commands, hooks, routines and shared_routines.
# - Moves the removed paths (outbox/, inbox/dead/, dead/, router.md, routines/, prompts/)
#   and 0.4 run folders (runs/<id>/ without events.jsonl) into .decree/legacy-0.4/,
#   keeping their paths.
# - Rewrites .decree/.gitignore to inbox/ and runs/.
# - Writes no machines. Lists each machine that a pending migration, inbox message or
#   cron file asks for but machines/ lacks, with the files that ask for it.
#
# Exit codes: 0 nothing to list, 1 machines listed, 2 nothing changed because of an error
# (no .decree/, or a move would overwrite a file).

set -euo pipefail
export LC_ALL=C
shopt -s nullglob

D=.decree
LEGACY=$D/legacy-0.4

die() {
    echo "migrate-0.4-to-0.5: $*" >&2
    exit 2
}

[[ -d $D ]] || die "no $D/ in $(pwd); run this from the project root"

# --- Plan every move, then check none overwrites anything before changing a file. ---

sources=()
targets=()
plan() {
    sources+=("$1")
    targets+=("$2")
}

for f in "$D"/outbox/*.md; do
    [[ -f $f ]] && plan "$f" "$D/inbox/$(basename "$f")"
done
for p in outbox inbox/dead dead router.md routines prompts; do
    [[ -e $D/$p ]] && plan "$D/$p" "$LEGACY/$p"
done
for r in "$D"/runs/*/; do
    r=${r%/}
    [[ -e $r/events.jsonl ]] || plan "$r" "$LEGACY/runs/$(basename "$r")"
done

for i in "${!targets[@]}"; do
    [[ -e ${targets[$i]} ]] && die "${targets[$i]} exists; not moving ${sources[$i]} over it. Nothing was changed."
done

# --- Move. Pending outbox files go first, so outbox/ is archived without them. ---

for i in "${!sources[@]}"; do
    mkdir -p "$(dirname "${targets[$i]}")"
    mv -n "${sources[$i]}" "${targets[$i]}"
    echo "moved ${sources[$i]} -> ${targets[$i]}"
done

# --- config.yml: rename two keys, drop the removed ones with their indented blocks. ---

if [[ -f $D/config.yml ]]; then
    awk '
        /^[^ \t#]/ {
            key = $0
            sub(/[ \t]*:.*/, "", key)
            gsub(/["\047]/, "", key)
            skip = (key == "commands" || key == "hooks" || key == "routines" || key == "shared_routines")
            if (skip) next
            sub(/^default_routine:/, "default_machine:")
            sub(/^"default_routine":/, "default_machine:")
            sub(/^routine_source:/, "shared_source:")
            sub(/^"routine_source":/, "shared_source:")
        }
        skip && (/^[ \t]/ || /^[ \t\r]*$/) { next }
        { print }
    ' "$D/config.yml" >"$D/.config.yml.tmp"
    mv "$D/.config.yml.tmp" "$D/config.yml"
    echo "rewrote $D/config.yml"
fi

printf 'inbox/\nruns/\n' >"$D/..gitignore.tmp"
mv "$D/..gitignore.tmp" "$D/.gitignore"
echo "rewrote $D/.gitignore"

# --- List the machines that pending messages need but machines/ lacks. ---

# Print the value of a top-level frontmatter key (section 4, Parsing), or nothing.
frontmatter_value() {
    awk -v want="$2" -v bom="$(printf '\357\273\277')" '
        { sub(/\r$/, "") }
        NR == 1 && index($0, bom) == 1 { $0 = substr($0, 4) }
        { t = $0; sub(/[ \t]+$/, "", t) }
        NR == 1 { if (t != "---") exit; next }
        t == "---" { exit }
        index($0, want ":") == 1 {
            v = substr($0, length(want) + 2)
            sub(/[ \t]+#.*/, "", v)
            gsub(/^[ \t]+|[ \t]+$/, "", v)
            if (v ~ /^".*"$/ || v ~ /^\047.*\047$/) v = substr(v, 2, length(v) - 2)
            if (v != "~" && v != "null") print v
            exit
        }
    ' "$1"
}

default_machine=""
if [[ -f $D/config.yml ]]; then
    default_machine=$(awk '
        /^default_machine:/ {
            v = substr($0, 17)
            sub(/[ \t]+#.*/, "", v)
            gsub(/^[ \t]+|[ \t]+$/, "", v)
            if (v ~ /^".*"$/ || v ~ /^\047.*\047$/) v = substr(v, 2, length(v) - 2)
            if (v != "~" && v != "null") print v
            exit
        }
    ' "$D/config.yml")
fi

declare -A processed=()
if [[ -f $D/processed.md ]]; then
    while IFS= read -r line || [[ -n $line ]]; do
        line=${line%$'\r'}
        line=${line#"${line%%[![:space:]]*}"}
        line=${line%"${line##*[![:space:]]}"}
        [[ -n $line ]] && processed[$line]=1
    done <"$D/processed.md"
fi

pending=()
for f in "$D"/migrations/*.md; do
    [[ -n ${processed[$(basename "$f")]:-} ]] || pending+=("$f")
done
# Globs skip dot files, as decree ignores them.
pending+=("$D"/inbox/*.md "$D"/cron/*.md)

declare -A askers=()
for f in "${pending[@]}"; do
    [[ -f $f ]] || continue
    name=$(frontmatter_value "$f" machine)
    [[ -n $name ]] || name=$(frontmatter_value "$f" routine)
    [[ -n $name ]] || name=$default_machine
    [[ -n $name ]] || continue
    if [[ ! $name =~ ^[a-z][a-z0-9_]*$ || ! -f $D/machines/$name.yml ]]; then
        askers[$name]+="    $f"$'\n'
    fi
done

[[ ${#askers[@]} -eq 0 ]] && exit 0

echo
echo "Machines that pending messages ask for but $D/machines/ lacks:"
mapfile -t names < <(printf '%s\n' "${!askers[@]}" | sort)
for name in "${names[@]}"; do
    if [[ $name =~ ^[a-z][a-z0-9_]*$ ]]; then
        echo "  $name"
    else
        echo "  $name: not a valid machine name (^[a-z][a-z0-9_]*\$); finish these under decree 0.4 first, since migrations are immutable"
    fi
    printf '%s' "${askers[$name]}"
done
exit 1
