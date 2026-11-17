#!/usr/bin/env bash
# Check or apply each line of every machine's requirements, as Ansible applies a play:
# `check` reports each resource ok or missing and changes nothing; `apply` installs a
# missing one and checks it again (changed), and stops at the first that fails, naming it.
# The requirements are one list: infra's own first, then each other
# .decree/scripts/<machine>/requirements in name order. Each line reported ends with the
# file it came from. A kind is kinds/<kind>.sh, beside this script.
# Usage: run.sh check|apply
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
scripts=$(dirname "${here}")
mode=${1:?usage: run.sh check|apply}
case ${mode} in check | apply) ;; *) echo "usage: run.sh check|apply" >&2; exit 2 ;; esac
# One stream, so a failure prints after the lines before it.
exec 2>&1

files=()
[ -f "${here}/requirements" ] && files+=("${here}/requirements")
for f in "${scripts}"/*/requirements; do
  [ -f "${f}" ] && [ "${f}" != "${here}/requirements" ] && files+=("${f}")
done

missing=0
for file in "${files[@]}"; do
  from="scripts/${file#"${scripts}/"}"
  while read -r kind name version source; do
    [[ -z ${kind} || ${kind} == \#* ]] && continue
    label="${kind} ${name}${version:+ ${version}}"
    [ -f "${here}/kinds/${kind}.sh" ] || {
      echo "FAILED   ${label}: no kinds/${kind}.sh (${from})" >&2
      exit 1
    }
    # A subshell keeps one kind's functions from the next.
    # shellcheck source=/dev/null
    if (. "${here}/kinds/${kind}.sh" && check "${name}" "${version}" "${source}"); then
      echo "ok       ${label} (${from})"
    elif [ "${mode}" = check ]; then
      echo "missing  ${label} (${from})"
      missing=1
    elif (. "${here}/kinds/${kind}.sh" && install "${name}" "${version}" "${source}" &&
      check "${name}" "${version}" "${source}"); then
      echo "changed  ${label} (${from})"
    else
      echo "FAILED   ${label} (${from})" >&2
      exit 1
    fi
  done < "${file}"
done
exit "${missing}"
