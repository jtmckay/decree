---
name: order
machine:
  description: At daemon start, run each machine's order/ files one at a time in file-name order, once each, and again when a file changes.
  state_files:
    running: The id of the order run working now, so a second one ends with `other`. idle writes it.
  initial: settle
  states:
    settle:
      description: The next file starts only once everything before it has been processed, its follow-ups included, as an empty inbox and no other run executing or pending show.
      invoke: idle
      transitions:
        idle: next
        busy: pause
        other: done
    pause:
      description: A timer while the daemon works through the inbox, 1s at first and doubling to 30s while it stays busy (idle writes order.deadline), so a quick follow-up goes on soon and a long run costs a look every 30s; nothing answers it.
      invoke:
        reply:
          question: Nothing to answer; order waits for the inbox to empty.
          notify: wait
          timeout_at: order.deadline
          attention: false
      transitions:
        ready:
          target: settle
          description: Never sent; the timer ends the wait.
        again:
          target: settle
          description: Never sent; the timer ends the wait.
        error: settle
    next:
      description: Find the first file that has not run, or changed since it ran, and write it to next.md.
      invoke: next
      transitions:
        found: run
        none: done
    run:
      description: Run that file's message as a child run, and wait for it.
      invoke:
        machine:
          message: next.md
      transitions:
        done: record
        error: failed
    record:
      description: Record the file and its hash in its machine's state/order.git.txt.
      invoke: record
      transitions:
        done: settle
    done:
      final: true
    failed:
      final: true
scripts:
  emit_if_changed.sh: |
    #!/usr/bin/env bash
    # emit_if_changed <key> [decree emit options...] < message
    # Queue the message on stdin with `decree emit` only when it differs from the last one
    # this key queued: reconcile, not repeat. The latest hash is in the calling machine's
    # $DECREE_STATE_DIR/<key>.txt, which git ignores; emit_if_changed_git keeps it in
    # <key>.git.txt, committed with the change that made it. Run it from a script:
    #   .decree/machines/order/scripts/emit_if_changed.sh weekly < "$DECREE_RUN_DIR/issue.md"
    set -euo pipefail
    suffix=${EMIT_IF_CHANGED_SUFFIX:-.txt}
    key=${1:?usage: emit_if_changed <key> [decree emit options] < message}
    shift
    case ${key} in
      "" | */* | .*)
        echo "emit_if_changed: key '${key}' must be a plain name" >&2
        exit 2
        ;;
    esac
    dir=${DECREE_STATE_DIR:?emit_if_changed runs from a decree script}
    record="${dir}/${key}${suffix}"
    message=$(cat; echo x)
    message=${message%x}
    hash=$(printf '%s' "${message}" | sha256sum | cut -d' ' -f1)
    if [ -f "${record}" ] && [ "$(cat "${record}")" = "${hash}" ]; then
      echo "${key}: unchanged since it was queued; not queued again"
      exit 0
    fi
    printf '%s' "${message}" | "${DECREE:-decree}" emit "$@"
    mkdir -p "${dir}"
    echo "${hash}" > "${record}"
  emit_if_changed_git.sh: |
    #!/usr/bin/env bash
    # emit_if_changed_git <key> [decree emit options...] < message
    # emit_if_changed, with the latest hash in $DECREE_STATE_DIR/<key>.git.txt, which git
    # tracks: commit it with the change that made the message, and a fresh clone queues
    # nothing that already ran.
    set -euo pipefail
    EMIT_IF_CHANGED_SUFFIX=.git.txt exec "$(dirname "${BASH_SOURCE[0]}")/emit_if_changed.sh" "$@"
  idle.sh: |
    #!/usr/bin/env bash
    # order's idle: `idle` when inbox/ is empty and no other run is executing or about to
    # (active or pending: machines run at once), so the next file starts only once
    # everything before it has been processed, its follow-ups included; `busy` otherwise. A
    # run waiting for a reply or a timer does not count. One order run works at a time: if
    # another is still going (active, waiting or pending), this one ends with `other`.
    set -euo pipefail
    lock="${DECREE_STATE_DIR}/running"
    if [ -s "${lock}" ] && [ "$(cat "${lock}")" != "${DECREE_MESSAGE_ID}" ]; then
      other=$(cat "${lock}")
      status=$("${DECREE:-decree}" status "${other}" --format json 2>/dev/null | jq -r '.status // empty' || true)
      case ${status} in
        active | waiting | pending)
          echo "order run ${other} is ${status}; it runs the order files"
          echo other > "${DECREE_EVENT_FILE}"
          exit 0
          ;;
      esac
    fi
    echo "${DECREE_MESSAGE_ID}" > "${lock}"
    # Busy: pause until order.deadline, 1s after the first look and doubling up to 30s
    # while the daemon stays busy; the count is reset once it is idle. Says why only on the
    # first look of a pause, so a long run adds no line per look.
    pauses="${DECREE_RUN_DIR}/order.pauses"
    busy() {
      local k t ms
      k=$(cat "${pauses}" 2>/dev/null || echo 0)
      (( k > 0 )) || echo "$1"
      t="${EPOCHREALTIME//[!0-9]/}"
      ms=$(( 10#$t / 1000 + (k >= 5 ? 30000 : 1000 << k) ))
      jq -nr --argjson ms "${ms}" '($ms / 1000 | floor | strftime("%Y-%m-%dT%H:%M:%S")) + "." + ("00" + ($ms % 1000 | tostring))[-3:] + "Z"' \
        > "${DECREE_RUN_DIR}/order.deadline"
      echo $(( k + 1 )) > "${pauses}"
      echo busy > "${DECREE_EVENT_FILE}"
    }
    if find .decree/inbox -maxdepth 1 -name '*.md' 2>/dev/null | grep -q .; then
      busy "messages are queued in inbox/"
      exit 0
    fi
    others=$("${DECREE:-decree}" status --format json |
      jq -r --arg me "${DECREE_MESSAGE_ID}" '[.runs.active[], .runs.pending[]] | map(select(.id != $me)) | length')
    if [ "${others}" != 0 ]; then
      busy "${others} other run(s) executing or about to"
    else
      rm -f "${pauses}"
      echo idle > "${DECREE_EVENT_FILE}"
    fi
  next.sh: |
    #!/usr/bin/env bash
    # order's next: the first file, across every .decree/machines/<machine>/order/, that has
    # not run, or changed since it ran: by file name, the machine's name breaking a tie. What
    # ran is in that machine's state/order.git.txt, a line `<file> <sha256>` each. Writes the
    # file to $DECREE_RUN_DIR/next.md, its `machine` the folder's unless it names one, with
    # `order: <machine>/<file>` added, and names the event: `found`, or `none`. With
    # ORDER_MAX set, it stops after that many files; the rest wait for the next start.
    set -euo pipefail
    machines=.decree/machines
    max=${ORDER_MAX:-0}
    count_file="${DECREE_RUN_DIR}/order-count"
    count=$(cat "${count_file}" 2>/dev/null || echo 0)
    if [ "${max}" -gt 0 ] && [ "${count}" -ge "${max}" ]; then
      echo "ran ${count} order file(s), ORDER_MAX=${max}: the rest wait for the next start"
      echo none > "${DECREE_EVENT_FILE}"
      exit 0
    fi
    list() {
      for dir in "${machines}"/*/order; do
        [ -d "${dir}" ] || continue
        machine=$(basename "$(dirname "${dir}")")
        for path in "${dir}"/*.md; do
          [ -f "${path}" ] && printf '%s\t%s\t%s\n' "$(basename "${path}")" "${machine}" "${path}"
        done
      done | LC_ALL=C sort -t "$(printf '\t')" -k1,1 -k2,2
    }
    while IFS="$(printf '\t')" read -r file machine path; do
      hash=$(sha256sum "${path}" | cut -d' ' -f1)
      record="${machines}/${machine}/state/order.git.txt"
      if [ -f "${record}" ] && grep -qxF "${file} ${hash}" "${record}"; then
        continue
      fi
      {
        echo '---'
        echo "order: ${machine}/${file}"
        if head -n 1 "${path}" | grep -qx -- '---'; then
          frontmatter=$(awk 'NR > 1 && /^---[[:space:]]*$/ { exit } NR > 1 { print }' "${path}")
          printf '%s\n' "${frontmatter}" | grep -q '^machine:' || echo "machine: ${machine}"
          tail -n +2 "${path}"
        else
          echo "machine: ${machine}"
          echo '---'
          cat "${path}"
        fi
      } > "${DECREE_RUN_DIR}/next.md"
      printf '%s\t%s\t%s\n' "${machine}" "${file}" "${hash}" > "${DECREE_RUN_DIR}/next"
      echo "$((count + 1))" > "${count_file}"
      echo "next: ${machine}/order/${file}"
      echo found > "${DECREE_EVENT_FILE}"
      exit 0
    done < <(list)
    echo "every order file has run, as it is now"
    echo none > "${DECREE_EVENT_FILE}"
  record.sh: |
    #!/usr/bin/env bash
    # order's record: the file next found ran; put `<file> <sha256>` in its machine's
    # state/order.git.txt, in place of any line for that file, sorted. Commit that file:
    # a fresh clone then runs nothing that already ran.
    #   record.sh                  the file in $DECREE_RUN_DIR/next (order's own state)
    #   record.sh <machine> <file> that order file, hashed now: a `commit` script calls it for a
    #                              run with `order:` so the line is in the run's one commit
    set -euo pipefail
    if [ $# -ge 2 ]; then
      machine=$1
      file=$2
      path=".decree/machines/${machine}/order/${file}"
      [ -f "${path}" ] || exit 0
      hash=$(sha256sum "${path}" | cut -d' ' -f1)
    else
      IFS="$(printf '\t')" read -r machine file hash < "${DECREE_RUN_DIR}/next"
    fi
    record=".decree/machines/${machine}/state/order.git.txt"
    mkdir -p "$(dirname "${record}")"
    {
      [ -f "${record}" ] && awk -v f="${file}" '$1 != f' "${record}"
      echo "${file} ${hash}"
    } | LC_ALL=C sort > "${record}.tmp"
    mv "${record}.tmp" "${record}"
    echo "recorded ${machine}/order/${file}"
  wait.sh: |
    #!/usr/bin/env bash
    # order's pause needs no answer: its timeout is the timer.
    true
cron:
  boot.md: |
    ---
    cron: '@reboot'
    ---
    Run each machine's order/ files that have not run, or changed since they ran.
---
# order

Ordered work for every machine, run when the daemon starts. A machine's `order/` folder, `.decree/machines/<machine>/order/`, holds messages to run in file-name order, one at a time: the declared state of that machine's domain ("this is how things should be"), not commands. `decree init` adds this plugin; delete `.decree/plugins/order.md` to go without it.

Each time `decree daemon` starts (and `decree daemon --once`, which CI and a system crontab run), its cron file `boot.md` (`@reboot`) starts this machine, which:

1. waits until `.decree/inbox/` is empty and no other run is executing or pending (machines run at once; a run waiting for a reply or a timer does not count), so a file starts only once everything before it has been processed, its follow-ups included;
2. finds the first file, across every machine's `order/`, by file name (the machine's name breaks a tie), that has not run, or changed since it ran;
3. runs it as a child run (`machine: { message: next.md }`): its `machine` is the folder's unless it names another, and it gets `order: <machine>/<file>` so its scripts know which file it is;
4. records `<file> <sha256>` in that machine's `state/order.git.txt` and goes back to 1.

A file whose run fails stops the files after it: this run ends in `failed`, with an attention file, and the next start runs that file again. A file you edit runs again, so write each one to be safe to re-run: check, then change only what differs. Name files so they sort in the order to run (`A01-setup.md`, `A02-…`); a machine with no `order/` has none.

- **Commit** each `order/` folder and each `state/order.git.txt` (`decree init`'s `.gitignore` keeps both in git), so a fresh clone runs only what has not run.
- **One file per start:** set `ORDER_MAX=1` for the daemon (`ORDER_MAX=1 decree daemon --once`); the rest wait for the next start.
- **Needs:** `jq` and `sha256sum` on `PATH`.

## Queue a message only when it changed

Two scripts here let any machine reconcile rather than repeat: give a message a key, and it is queued only when it differs from the last one queued with that key.

```bash
# In a script of machine `newsletter`: queue this week's issue once, and again only if it changes.
.decree/machines/order/scripts/emit_if_changed.sh weekly --machine publish < "$DECREE_RUN_DIR/issue.md"
```

- `emit_if_changed.sh <key> [decree emit options] < message` keeps the latest hash in the calling machine's `$DECREE_STATE_DIR/<key>.txt`, which git ignores.
- `emit_if_changed_git.sh` keeps it in `<key>.git.txt`, which git tracks: commit it with the change that made the message, and a fresh clone queues nothing that already ran.

A plugin that uses them lists `order` in its `needs`.
