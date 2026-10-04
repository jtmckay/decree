#!/usr/bin/env bash
# Upgrade a decree 0.4 project's .decree/ to the 0.5 layout, once (docs/0.5-spec.md, M5.4).
#
# Run from the project root: scripts/migrate-0.4-to-0.5.sh
#
# - Leaves migrations/ and processed.md untouched.
# - Moves pending outbox/*.md into inbox/ (pending inbox/*.md stay where they are).
# - Moves the removed paths (config.yml, outbox/, inbox/dead/, dead/, router.md, routines/,
#   prompts/) and 0.4 run folders (runs/<id>/ without events.jsonl) into
#   .decree/legacy-0.4/, keeping their paths. 0.5 has no configuration file (section 3).
# - Rewrites .decree/.gitignore to inbox/ and runs/.
# - Writes no machines. Lists each machine that a pending migration, inbox message or
#   cron file asks for but machines/ lacks, with the files that ask for it, and each such
#   file that names no machine.
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
for p in config.yml outbox inbox/dead dead router.md routines prompts; do
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
unnamed=""
for f in "${pending[@]}"; do
    [[ -f $f ]] || continue
    # A reply (section 4, Replies) names the run it answers, not a machine.
    [[ -n $(frontmatter_value "$f" to) ]] && continue
    name=$(frontmatter_value "$f" machine)
    [[ -n $name ]] || name=$(frontmatter_value "$f" routine)
    if [[ -z $name ]]; then
        unnamed+="    $f"$'\n'
        continue
    fi
    if [[ ! $name =~ ^[a-z][a-z0-9_]*$ || ! -f $D/machines/$name.yml ]]; then
        askers[$name]+="    $f"$'\n'
    fi
done

[[ ${#askers[@]} -eq 0 && -z $unnamed ]] && exit 0

if [[ -n $unnamed ]]; then
    echo
    echo "Pending messages that name no machine (0.5 has no default; add machine: to inbox"
    echo "and cron files, and finish migrations under decree 0.4 first, since migrations are"
    echo "immutable):"
    printf '%s' "$unnamed"
fi
[[ ${#askers[@]} -eq 0 ]] && exit 1

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
