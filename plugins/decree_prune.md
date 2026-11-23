---
name: decree_prune
machine:
  description: Each week, delete the folders of finished runs older than PRUNE_OLDER_THAN (default 30d).
  initial: prune
  states:
    prune:
      description: Delete every finished run older than the age, but a child run whose parent is not finished, and one a process holds.
      invoke: prune
      transitions:
        done: done
    done:
      final: true
    failed:
      final: true
scripts:
  prune.sh: |
    #!/usr/bin/env bash
    # prune's only script: delete .decree/runs/<id>/ for every finished run (its last event
    # is `run_finished`) whose `run_finished` is older than PRUNE_OLDER_THAN, a duration
    # (30d, 12h, 90m, 90s; default 30d), and its file in .decree/attention/. Never deleted:
    # a run that is not finished; a child run (its message.md has `parent`) whose parent still
    # exists and is not finished, since the parent may still read it; a run whose .lock names
    # a live process. Machines' state/ folders are never touched. With PRUNE_DRY_RUN=1 it
    # lists what it would delete and deletes nothing.
    set -euo pipefail
    age=${PRUNE_OLDER_THAN:-30d}
    case ${age} in
      *[0-9]s) seconds=${age%s} ;;
      *[0-9]m) seconds=$((${age%m} * 60)) ;;
      *[0-9]h) seconds=$((${age%h} * 3600)) ;;
      *[0-9]d) seconds=$((${age%d} * 86400)) ;;
      *) echo "prune: PRUNE_OLDER_THAN=${age} is not a duration (30d, 12h, 90m, 90s)" >&2; exit 2 ;;
    esac
    [[ ${seconds} =~ ^[0-9]+$ ]] || { echo "prune: PRUNE_OLDER_THAN=${age} is not a duration" >&2; exit 2; }
    cutoff=$(($(date +%s) - seconds))
    runs=.decree/runs
    finished() { tail -n 1 "${runs}/$1/events.jsonl" 2>/dev/null | jq -e 'select(.type == "run_finished")' >/dev/null 2>&1; }
    pruned=0
    for dir in "${runs}"/*/; do
      [ -d "${dir}" ] || continue
      id=$(basename "${dir}")
      finished "${id}" || continue
      ts=$(tail -n 1 "${dir}events.jsonl" | jq -r '.ts')
      when=$(date -d "${ts}" +%s 2>/dev/null) || continue
      [ "${when}" -lt "${cutoff}" ] || continue
      parent=$(sed -n '2,/^---$/s/^parent: //p' "${dir}message.md" 2>/dev/null | head -n 1)
      if [ -n "${parent}" ] && [ -d "${runs}/${parent}" ] && ! finished "${parent}"; then
        continue
      fi
      if [ -f "${dir}.lock" ] && kill -0 "$(cat "${dir}.lock" 2>/dev/null || echo 0)" 2>/dev/null; then
        continue
      fi
      machine=$(tail -n 1 "${dir}events.jsonl" | jq -r '.machine')
      state=$(tail -n 1 "${dir}events.jsonl" | jq -r '.state')
      if [ "${PRUNE_DRY_RUN:-}" = 1 ]; then
        echo "would prune ${id}  ${machine}  ${state}  finished ${ts}"
      else
        rm -rf "${dir}"
        rm -f ".decree/attention/${id}.md"
        echo "pruned ${id}  ${machine}  ${state}  finished ${ts}"
      fi
      pruned=$((pruned + 1))
    done
    echo "pruned ${pruned} run(s) finished before ${age} ago${PRUNE_DRY_RUN:+ (dry run)}"
cron:
  weekly.md: |
    ---
    cron: 0 3 * * Sun
    ---
    Delete the finished runs older than PRUNE_OLDER_THAN.
---
# decree_prune

Deletes the folders of finished runs, `.decree/runs/<id>/`, that finished longer ago than `PRUNE_OLDER_THAN` (a duration, `30d` by default), every Sunday at 03:00 (its cron file, `.decree/machines/decree_prune/cron/weekly.md`), while `decree daemon` runs. Nothing else deletes a run. `decree init` adds it; delete `.decree/plugins/decree_prune.md` to keep every run.

It never deletes a run that is not finished, a child run whose parent is not finished (the parent may still read it), or a run a process holds, and it never touches a machine's `state/`. Ship runs to a log store before they go if you keep history ([observability.md](https://github.com/jtmckay/decree/blob/main/docs/reference/observability.md)).

- **Another age:** set `PRUNE_OLDER_THAN` in `.decree/.env` (`PRUNE_OLDER_THAN=7d`).
- **Another day:** edit the installed cron file, `.decree/machines/decree_prune/cron/weekly.md`: once installed, a cron file is the project's, and no install replaces it (`decree install --cron` takes the plugin's back). To keep a schedule in a committed file, extend `decree_prune` with a plugin of your own holding `cron/weekly.md`; it runs instead of `decree_prune`'s.
- **Now:** `echo "Prune." | decree emit --machine decree_prune && decree daemon --once`; with `PRUNE_DRY_RUN=1` it lists what it would delete and deletes nothing.
- **Needs:** `jq` and GNU `date` on `PATH`.
