---
name: lease
machine:
  description: One holder per resource at a time, for a limited time; the next holder is picked to keep mode swaps rare. Ask with params resource and mode; the lease is held while its run is in held; give it back with decree event <id> release.
  data:
    resource:
      type: string
      default: ''
    mode:
      type: string
      default: ''
    duration:
      type: string
      default: ''
    holder:
      type: string
      default: ''
  state_files:
    leases: 'One folder per resource: queue (the leases waiting, with when each asked), holder (the lease that holds it, its mode, its expiry), mode (what the resource is set up for), mode_since (when that mode was swapped in, for max_wait), free_since (when it was last given back) and lock. turn, swap, renew and leave read and write it, each under flock.'
  initial: turn
  states:
    turn:
      description: 'Who goes next: checks the request, joins the line, and grants the resource when this lease is the one the rules pick (current mode first, linger, max_wait''s slice per mode); for a held lease, checks its expiry.'
      invoke: turn
      transitions:
        granted: swap
        wait: waiting
        held: held
        expired: done
    waiting:
      description: In line. Looks again every second (the timeout), so a lease is granted soon after the resource frees, and release while waiting always lands on a wait.
      invoke:
        reply:
          question: This lease waits in line for its resource. Give up its place, or ask to keep it longer once held?
          notify: tell
          timeout: 1s
          attention: false
      transitions:
        release:
          target: done
          description: Give up this lease's place in line; it is never held.
        renew:
          target: renew
          description: Refused while the lease is not held yet; the log says so.
        error: turn
    swap:
      description: Puts the resource into this lease's mode with machines/lease/scripts/swap/<resource>.sh, a step with a timeout because unloading a model can take minutes; its failure fails the lease and leaves the mode unknown. Then the hold starts.
      invoke:
        script:
          name: swap
          timeout: 30m
      transitions:
        done: held
    held:
      description: The lease holds the resource. Looks every second (the timeout) whether it has expired, so a lease nobody releases ends at its expiry.
      invoke:
        reply:
          question: This lease holds its resource. Give it back, or keep it for its duration again?
          notify: tell
          timeout: 1s
          attention: false
      transitions:
        release:
          target: done
          description: Give the resource back now.
        renew:
          target: renew
          description: Keep the resource for this lease's duration again, from now; refused while another lease waits for it.
        error: turn
    renew:
      description: Extends a held lease by its duration, only while no other lease waits for the resource; otherwise logs the refusal and keeps the expiry.
      invoke: renew
      transitions:
        held: held
        waiting: waiting
        expired: done
    done:
      final: true
      onentry:
      - leave
    failed:
      final: true
      onentry:
      - leave
scripts:
  lease.sh: |
    # lease's shared code, sourced by its scripts (never run on its own). The rules are in
    # machines/lease/scripts/resources; what the machine keeps between runs is in
    # $DECREE_STATE_DIR/leases/<resource>/, read and written only under flock on its lock file, so
    # several decree processes sharing the project never see half a change:
    #   queue       one line per waiting lease: <asked, ms since the epoch> <lease id> <mode>
    #   holder      <lease id> <mode> <expiry, ms> <swapping|held>, absent when nobody holds it
    #   mode        what the resource is set up for; empty when unknown
    #   free_since  when the resource was last given back, ms
    #   mode_since  when the current mode was swapped in, ms (max_wait's slice starts there)
    # A holder past its expiry holds nothing, whatever state its run is in. Every grant,
    # release, expiry, refused renewal and swap is one line in the run's log, `lease: <what>`
    # then key=value pairs naming the resource, the mode and the lease.
    set -euo pipefail
    lease_rules=.decree/machines/lease/scripts/resources
    me="${DECREE_MESSAGE_ID}"
    decree_bin="${DECREE:-decree}"

    die() { echo "lease: $*" >&2; exit 1; }
    say() { echo "lease: $*"; }

    # Now, in milliseconds since the epoch (bash's EPOCHREALTIME, whatever the locale's decimal sign).
    now_ms() { local t="${EPOCHREALTIME//[!0-9]/}"; echo $(( 10#$t / 1000 )); }

    # A duration in decree's format (15m, 5s, 2h, 1d) in milliseconds; fails on anything else.
    dur_ms() {
      [[ "$1" =~ ^([0-9]{1,9})([smhd])$ ]] || return 1
      local n=$(( 10#${BASH_REMATCH[1]} ))
      case "${BASH_REMATCH[2]}" in
        s) echo $(( n * 1000 )) ;; m) echo $(( n * 60000 )) ;;
        h) echo $(( n * 3600000 )) ;; d) echo $(( n * 86400000 )) ;;
      esac
    }

    # When the reply this run last received reached the queue, in ms, from its id
    # (YYYYMMDDTHHMMSS.ffffffZ-xxxxxx, UTC): the time that counts for a reply, as for decree's
    # own timeout. Empty when there is none.
    reply_ms() {
      local f="${DECREE_RECEIVED:-}" id
      [ -n "$f" ] && [ -f "$f" ] || return 0
      id="$(sed -n 's/^id:[[:space:]]*//p' "$f" | head -n 1)"
      [[ "$id" =~ ^([0-9]{4})([0-9]{2})([0-9]{2})T([0-9]{2})([0-9]{2})([0-9]{2})\.([0-9]{3}) ]] || return 0
      local y=$((10#${BASH_REMATCH[1]})) m=$((10#${BASH_REMATCH[2]})) d=$((10#${BASH_REMATCH[3]}))
      local H=$((10#${BASH_REMATCH[4]})) M=$((10#${BASH_REMATCH[5]})) S=$((10#${BASH_REMATCH[6]})) ms=$((10#${BASH_REMATCH[7]}))
      # Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's days_from_civil).
      (( m <= 2 )) && y=$((y - 1))
      local era=$(( y / 400 )); local yoe=$(( y - era * 400 ))
      local doy=$(( (153 * (m > 2 ? m - 3 : m + 9) + 2) / 5 + d - 1 ))
      local doe=$(( yoe * 365 + yoe / 4 - yoe / 100 + doy ))
      local days=$(( era * 146097 + doe - 719468 ))
      echo $(( ((days * 86400) + H * 3600 + M * 60 + S) * 1000 + ms ))
    }

    # An instant in ms, as RFC 3339 UTC with milliseconds.
    at() { local s; TZ=UTC printf -v s '%(%Y-%m-%dT%H:%M:%S)T' $(( $1 / 1000 )); printf '%s.%03dZ' "$s" $(( $1 % 1000 )); }

    # The rules' lines, comments and blank lines removed.
    rules() { [ -f "$lease_rules" ] || return 0; sed -e 's/#.*//' -e '/^[[:space:]]*$/d' "$lease_rules"; }
    resource_names() { local n; n="$(rules | awk '{ print $1 }' | paste -sd ' ' -)"; echo "${n:-none}"; }

    # Reads the run's params and its resource's rules, or fails naming the problem. Sets
    # res, mode, holder_name, max_lease_ms, linger_ms, max_wait_ms, dur_ms_ (capped) and dir.
    load() {
      res="${DECREE_DATA_RESOURCE:-}"
      mode="${DECREE_DATA_MODE:-}"
      holder_name="${DECREE_DATA_HOLDER:-}"
      [ -n "$res" ] || die "no resource: ask with the params resource and mode. The resources are: $(resource_names) (in $lease_rules)."
      local line
      line="$(rules | awk -v r="$res" '$1 == r' | head -n 1)"
      [ -n "$line" ] || die "unknown resource \`$res\`. The resources are: $(resource_names) (in $lease_rules)."
      [ -n "$mode" ] || die "no mode for resource \`$res\`: ask with the param mode, what the resource must be set up for (such as a model name)."
      [[ "$mode" =~ ^[A-Za-z0-9][A-Za-z0-9._:/@+-]*$ ]] || die "mode \`$mode\` for resource \`$res\` is not one word of letters, digits and ._:/@+-"
      local name max linger wait extra
      read -r name max linger wait extra <<<"$line"
      [ -z "$extra" ] || die "$lease_rules: the line for \`$res\` has more than <name> <max_lease> <linger> <max_wait>"
      max_lease_ms="$(dur_ms "$max")" || die "$lease_rules: max_lease \`$max\` for \`$res\` is not a duration such as 15m"
      linger_ms="$(dur_ms "${linger:-5s}")" || die "$lease_rules: linger \`$linger\` for \`$res\` is not a duration such as 5s"
      max_wait_ms="$(dur_ms "${wait:-30m}")" || die "$lease_rules: max_wait \`$wait\` for \`$res\` is not a duration such as 30m"
      dur_ms_="$max_lease_ms"
      capped=""
      if [ -n "${DECREE_DATA_DURATION:-}" ]; then
        dur_ms_="$(dur_ms "$DECREE_DATA_DURATION")" || die "duration \`$DECREE_DATA_DURATION\` is not a duration such as 15m"
        if (( dur_ms_ > max_lease_ms )); then dur_ms_="$max_lease_ms"; capped="$max"; fi
      fi
      dir="${DECREE_STATE_DIR}/leases/${res}"
      mkdir -p "$dir"
      touch "$dir/queue"
    }

    # Runs "$@" holding the resource's lock.
    locked() { local fd; exec {fd}>"$dir/lock"; flock "$fd"; "$@"; local rc=$?; flock -u "$fd"; exec {fd}>&-; return "$rc"; }

    ids() { echo "resource=$res mode=$1 lease=$2"; }

    # holder file -> h_id h_mode h_exp h_phase (empty when nobody holds it)
    read_holder() { h_id=""; h_mode=""; h_exp=0; h_phase=""; [ -s "$dir/holder" ] && read -r h_id h_mode h_exp h_phase < "$dir/holder" || true; }
    write_holder() { printf '%s %s %s %s\n' "$1" "$2" "$3" "$4" > "$dir/holder.new"; mv "$dir/holder.new" "$dir/holder"; }
    current_mode() { [ -s "$dir/mode" ] && cat "$dir/mode" || true; }
    set_mode() { printf '%s\n' "$1" > "$dir/mode.new"; mv "$dir/mode.new" "$dir/mode"; }
    set_free() { echo "$1" > "$dir/free_since"; }
    unqueue() { grep -v " $1 " "$dir/queue" > "$dir/queue.new" || true; mv "$dir/queue.new" "$dir/queue"; }

    # "<status> <state>" of a run, as decree status reports it; "finished" for an unknown run.
    run_state() { "$decree_bin" status "$1" --format json 2>/dev/null | jq -r '"\(.status) \(.state // "")"' || echo "finished"; }
    # Whether a run can still act: not finished, not interrupted.
    alive() { case "$(run_state "$1")" in finished*|interrupted*) return 1 ;; *) return 0 ;; esac; }

    # This run's own facts: when it first asked (it keeps its age across a retry), and
    # whether it was ever granted (a granted lease never goes back in line).
    asked_ms() { local f="$DECREE_RUN_DIR/lease.asked"; [ -s "$f" ] || now_ms > "$f"; cat "$f"; }
    granted_before() { [ -f "$DECREE_RUN_DIR/lease.granted" ]; }
  leave.sh: |
    #!/usr/bin/env bash
    # The lease is over (done or failed): it leaves the line, and if it holds the resource it
    # gives it back, which starts the linger. A lease that failed while swapping leaves the
    # mode unknown, so the next lease swaps again. Safe to re-run.
    set -euo pipefail
    . .decree/machines/lease/scripts/lease.sh
    ( load ) >/dev/null 2>&1 || exit 0   # a request that never named a real resource holds nothing
    load
    give_back() {
      local was_queued=""
      grep -q " $me " "$dir/queue" && was_queued=1
      unqueue "$me"
      read_holder
      if [ "$h_id" = "$me" ]; then
        local now; now="$(now_ms)"
        rm -f "$dir/holder"
        set_free "$now"
        if [ "$h_phase" = swapping ]; then
          set_mode ""
          say "release $(ids "$h_mode" "$me") at=$(at "$now") reason=\"failed while swapping: mode unknown\""
        else
          say "release $(ids "$h_mode" "$me") at=$(at "$now")"
        fi
      elif [ -n "$was_queued" ]; then
        say "release $(ids "$mode" "$me") reason=\"left the line, never held\""
      fi
    }
    locked give_back
  renew.sh: |
    #!/usr/bin/env bash
    # renew: a held lease gets its duration again from now (etcd's keepalive), but only while
    # no other lease waits for the resource; otherwise the renewal is refused and the expiry
    # stays. Names held, waiting (not held yet) or expired. The renewal counts at the time it
    # reached the queue (its id), so one sent before the expiry is honoured even when decree
    # gets to it after; the new expiry is a duration from now.
    set -euo pipefail
    . .decree/machines/lease/scripts/lease.sh
    load
    decide() {
      local now sent; now="$(now_ms)"; sent="$(reply_ms)"
      [ -n "$sent" ] && (( sent < now )) || sent="$now"
      read_holder
      if [ "$h_id" != "$me" ]; then
        if granted_before; then
          say "expire $(ids "$mode" "$me") reason=\"no longer the holder\""
          echo expired > "$DECREE_EVENT_FILE"
        else
          say "renew refused $(ids "$mode" "$me") reason=\"not held yet\""
          echo waiting > "$DECREE_EVENT_FILE"
        fi
        return 0
      fi
      if (( sent >= h_exp )); then
        rm -f "$dir/holder"; set_free "$now"
        say "expire $(ids "$h_mode" "$me") at=$(at "$h_exp") reason=\"renew sent at $(at "$sent"), after the expiry\""
        echo expired > "$DECREE_EVENT_FILE"
        return 0
      fi
      local others=0 asked id m
      while read -r asked id m; do
        [ -n "$id" ] && [ "$id" != "$me" ] || continue
        if alive "$id"; then others=$((others + 1)); else unqueue "$id"; fi
      done < <(cat "$dir/queue")
      if (( others > 0 )); then
        say "renew refused $(ids "$h_mode" "$me") expires=$(at "$h_exp") reason=\"$others other lease(s) wait for $res\""
      else
        local exp=$(( now + dur_ms_ ))
        write_holder "$me" "$h_mode" "$exp" held
        say "renew $(ids "$h_mode" "$me") expires=$(at "$exp")"
      fi
      echo held > "$DECREE_EVENT_FILE"
    }
    locked decide
  requirements: |
    # What lease's scripts need beyond decree, checked by infra's `run.sh check`.
    # kind    name    version   source
    command   bash
    command   flock
    command   jq
  resources: |
    # The resources leases are for, one per line: <name> <max_lease> <linger> <max_wait>.
    #   max_lease  the longest a lease holds it; a lease's duration is capped at it, and one
    #              asked for without a duration lasts this long.
    #   linger     how long the resource stays in its mode after it is given back with nobody
    #              in that mode waiting, so a lease for that mode asked for meanwhile goes
    #              without a swap (default 5s).
    #   max_wait   a time slice per mode: once the resource has been in its mode this long
    #              since it was swapped in and a lease for another mode waits, the oldest such
    #              lease goes next, with no linger (default 30m). The lease holding it then
    #              keeps it until released or expired, so another mode waits at most
    #              max_wait + max_lease + one swap.
    # Durations are decree's: a whole number and s, m, h or d. A resource's swap is
    # swap/<name>.sh beside this file. An edit is kept by `decree plugin export lease`.
    gpu 15m 5s 30m
  swap.sh: |
    #!/usr/bin/env bash
    # The lease was granted: put the resource into its mode, then start the hold. A swap runs
    # machines/lease/scripts/swap/<resource>.sh <from> <to> (<from> empty when unknown); a missing
    # script means there is nothing to do, and a lease for the mode already set needs none.
    # While the swap runs, the lease's expiry is kept a duration ahead every second, so if
    # this run is interrupted the resource frees one duration later. If the swap fails, the
    # lease fails and the mode is unknown (leave clears the holder).
    set -euo pipefail
    . .decree/machines/lease/scripts/lease.sh
    load
    from=""
    start() {
      read_holder
      [ "$h_id" = "$me" ] || { say "lost $(ids "$mode" "$me") reason=\"another lease holds $res: this lease expired while it was interrupted\"" >&2; return 1; }
      from="$(current_mode)"
      [ "$from" = "$mode" ] || set_mode ""
    }
    locked start
    if [ "$from" != "$mode" ]; then
      script=".decree/machines/lease/scripts/swap/${res}.sh"
      say "swap $(ids "$mode" "$me") from=${from:-unknown} to=$mode"
      if [ -x "$script" ]; then
        beat() { read_holder; [ "$h_id" != "$me" ] || write_holder "$me" "$mode" $(( $(now_ms) + dur_ms_ )) swapping; }
        ( while sleep 1; do locked beat; done ) </dev/null >/dev/null 2>&1 &
        heart=$!
        rc=0
        "$script" "$from" "$mode" || rc=$?
        kill "$heart" 2>/dev/null || true
        wait "$heart" 2>/dev/null || true
        if (( rc != 0 )); then
          say "swap failed $(ids "$mode" "$me") from=${from:-unknown} to=$mode exit=$rc" >&2
          exit 1
        fi
      else
        say "swap $(ids "$mode" "$me") note=\"no $script: nothing to do\""
      fi
    fi
    hold() {
      read_holder
      [ "$h_id" = "$me" ] || { say "lost $(ids "$mode" "$me") reason=\"another lease holds $res\"" >&2; return 1; }
      set_mode "$mode"
      [ "$from" = "$mode" ] && [ -s "$dir/mode_since" ] || now_ms > "$dir/mode_since"
      local exp=$(( $(now_ms) + dur_ms_ ))
      write_holder "$me" "$mode" "$exp" held
      say "held $(ids "$mode" "$me") holder=\"$holder_name\" expires=$(at "$exp")"
    }
    locked hold
  swap/gpu.sh: |
    #!/usr/bin/env bash
    # Puts the GPU into mode $2, from mode $1 (empty when the current mode is unknown). lease
    # runs this as a step with a timeout before a lease for another mode is held; a non-zero
    # exit fails that lease, and the next one swaps again from an unknown mode.
    #
    # This one only says what it would do. To unload ollama's models (GET /api/ps, then
    # POST /api/generate {"model": <name>, "keep_alive": 0} for each) or ComfyUI's (wait for
    # an empty GET /queue, then POST /free {"unload_models": true, "free_memory": true}), see
    # https://github.com/jtmckay/decree/blob/main/docs/services.md. Keep an edit with
    # `decree plugin export lease`.
    set -euo pipefail
    echo "gpu: would unload ${1:-whatever is loaded} and load $2 (edit .decree/machines/lease/scripts/swap/gpu.sh)"
  tell.sh: |
    #!/usr/bin/env bash
    # The ask script of waiting and held: says once, on the first visit, where the lease is and
    # how to answer. The state looks again every second, and says nothing then.
    set -euo pipefail
    [ "${DECREE_VISITS:-1}" = 1 ] || exit 0
    . .decree/machines/lease/scripts/lease.sh
    load
    case "${DECREE_STATE}" in
      waiting) say "wait $(ids "$mode" "$me") ahead=$(( $(grep -c . "$dir/queue" || true) - 1 )) answer=\"decree event $me release\"" ;;
      held) say "hold $(ids "$mode" "$me") answer=\"decree event $me release|renew\"" ;;
    esac
  turn.sh: |
    #!/usr/bin/env bash
    # Who goes next, per resource, at most one holder at a time:
    #   1. a free resource goes to the oldest lease waiting in its current mode;
    #   2. if none waits, the resource lingers in its mode for linger: a lease for that mode
    #      asked for meanwhile goes with no swap;
    #   3. after the linger, or with no mode set, the oldest lease goes, and its mode is swapped in;
    #   4. max_wait is a time slice per mode, from when it was swapped in (round-robin's
    #      quantum): once the mode has run longer than that and a lease in another mode waits,
    #      the oldest such lease goes next, with no linger. While no other mode waits, the
    #      current mode goes on however long it has run.
    # A holder past its expiry holds nothing: its lease is ended here when its run is finished or
    # interrupted (a live run's own look ends it). Names granted, wait, held or expired;
    # fails at once on a bad request, naming the problem and the resources there are.
    set -euo pipefail
    . .decree/machines/lease/scripts/lease.sh
    load
    asked="$(asked_ms)"
    if [ "${DECREE_VISITS:-1}" = 1 ] && [ -n "$capped" ]; then
      say "cap $(ids "$mode" "$me") duration=$DECREE_DATA_DURATION max_lease=$capped"
    fi

    # The lease the rules pick from the line, or nothing (see the top of this file).
    pick() {
      local now="$1" cur free since oldest="" other="" same=""
      cur="$(current_mode)"
      free="$(cat "$dir/free_since" 2>/dev/null || echo 0)"
      since="$(cat "$dir/mode_since" 2>/dev/null || echo "$now")"
      local a id m
      while read -r a id m; do
        [ -n "$id" ] || continue
        [ -n "$oldest" ] || oldest="$id"
        if [ -n "$cur" ] && [ "$m" = "$cur" ]; then
          [ -n "$same" ] || same="$id"
        else
          [ -n "$other" ] || other="$id"
        fi
      done < <(sort -n "$dir/queue")
      if [ -n "$cur" ] && [ -n "$other" ] && (( now - since > max_wait_ms )); then echo "$other"
      elif [ -n "$same" ]; then echo "$same"
      elif [ -n "$cur" ] && (( now < free + linger_ms )); then echo ""
      else echo "$oldest"
      fi
    }

    decide() {
      local now; now="$(now_ms)"
      read_holder
      if [ "$h_id" = "$me" ]; then
        if [ "$h_phase" = swapping ]; then echo granted > "$DECREE_EVENT_FILE"; return 0; fi
        if (( now >= h_exp )); then
          rm -f "$dir/holder"; set_free "$now"
          say "expire $(ids "$h_mode" "$me") at=$(at "$h_exp")"
          echo expired > "$DECREE_EVENT_FILE"
        else
          echo held > "$DECREE_EVENT_FILE"
        fi
        return 0
      fi
      if granted_before; then
        say "expire $(ids "$mode" "$me") reason=\"no longer the holder\""
        echo expired > "$DECREE_EVENT_FILE"
        return 0
      fi
      grep -q " $me " "$dir/queue" || echo "$asked $me $mode" >> "$dir/queue"
      if [ -n "$h_id" ]; then
        (( now >= h_exp )) || { echo wait > "$DECREE_EVENT_FILE"; return 0; }
        local st; st="$(run_state "$h_id")"
        case "$st" in
          finished*|interrupted*) ;;
          *) echo wait > "$DECREE_EVENT_FILE"; return 0 ;;   # its run is alive: its own look ends it
        esac
        rm -f "$dir/holder"; set_free "$h_exp"
        [ "$h_phase" = held ] || set_mode ""
        say "expire $(ids "$h_mode" "$h_id") at=$(at "$h_exp") reason=\"its run is ${st% *} in ${st#* }\" by=$me"
      fi
      local next
      while :; do
        next="$(pick "$now")"
        [ -n "$next" ] && [ "$next" != "$me" ] && ! alive "$next" || break
        unqueue "$next"
      done
      if [ "$next" != "$me" ]; then echo wait > "$DECREE_EVENT_FILE"; return 0; fi
      unqueue "$me"
      write_holder "$me" "$mode" $(( now + dur_ms_ )) swapping
      touch "$DECREE_RUN_DIR/lease.granted"
      local cur; cur="$(current_mode)"
      say "grant $(ids "$mode" "$me") holder=\"$holder_name\" waited_ms=$(( now - asked )) from=${cur:-unknown}"
      echo granted > "$DECREE_EVENT_FILE"
    }
    locked decide
---
# lease

A "talking stick" for a resource several programs want at once: a GPU, one LLM server, a paid API with a budget. Each program asks for a **lease**; one lease holds the resource at a time, for a limited time, and gives it back. The next holder is picked to keep swaps rare: setting the resource up for another mode (unloading one model, loading another) is slow, so every lease for the mode already loaded goes first, as long as nobody in another mode waits too long.

A lease is a decree run of this machine, and its run id is the lease's id. Nothing runs inside decree itself: the line of waiting leases is in the machine's state folder, `.decree/machines/lease/state/`.

## What it needs

- `bash`, `flock` (util-linux) and `jq`, listed in `machines/lease/scripts/requirements`, so `infra`'s `run.sh check` checks them.
- `decree daemon` running: a waiting lease looks every second whether it is its turn, and a held one whether it has expired, and `decree daemon --once` keeps looking too, until no lease is left waiting or held. Those waits are timers, not questions for a person (`attention: false`), so a lease never puts a file in `.decree/attention/`.

## Adding it

```bash
decree plugin add lease
```

## Setting the rules

The resource's owner writes the rules in `.decree/machines/lease/scripts/resources`, one resource per line, `#` for comments:

```text
# name  max_lease  linger  max_wait
gpu     15m        5s      30m
```

- `max_lease`: the longest a lease holds the resource.
- `linger` (default `5s`): after a lease gives the resource back and none waits in its mode, the resource stays in that mode this long, so a lease for it asked for meanwhile goes without a swap.
- `max_wait` (default `30m`): a time slice per mode, counted from when the mode was swapped in. Once the resource has been in its mode longer than this and a lease for another mode waits, no more leases in the current mode are granted and the resource does not linger: when it is next free, the oldest lease for another mode goes and its mode is swapped in. While no lease for another mode waits, the current mode goes on however long it has run. The lease holding the resource when the slice runs out keeps it until it is released or expires (a renewal is refused while another lease waits), so a lease for another mode waits at most `max_wait` + `max_lease` + one swap after its mode was swapped out.

Durations are decree's: `15m`, `5s`, `2h`.

To put the resource into a mode, write `.decree/machines/lease/scripts/swap/<name>.sh`. It gets the current mode (empty when unknown) and the mode wanted: `swap/gpu.sh <from> <to>`. With no script, there is nothing to do. The shipped `swap/gpu.sh` only says what it would do; [docs/services.md](https://github.com/jtmckay/decree/blob/main/docs/services.md) shows how to unload ollama's and ComfyUI's models. If a swap fails, its lease fails, the mode becomes unknown, and the next lease swaps again.

Keep your edits to the rules and swap scripts with `decree plugin export lease`.

## Asking for a lease

Send a message for machine `lease` with the params `resource` (required), `mode` (what the resource must be set up for, such as a model name), and optionally `duration` (capped at the resource's `max_lease`, and that is the default) and `holder` (free text for the logs):

```bash
id=$(echo "Render the batch" | decree emit --machine lease --param resource=gpu --param mode=comfy --param holder=render-box)
```

Through decree-go-rest it is one POST, which returns the same id. An unknown resource or a missing mode fails the run at once, naming the problem and the resources there are.

## Knowing it is held

A lease holds its resource exactly while its run's state is `held`:

```bash
decree status "$id" --format json | jq -r .state     # held
```

Before that it is in `turn`, `waiting` or `swap`; after, in `done` or `failed`.

## Giving it back, or keeping it

```bash
decree event "$id" release    # give it back, or leave the line if still waiting
decree event "$id" renew      # hold it for its duration again, from now
```

A renewal is refused while another lease waits for the resource: the expiry stays, and the run's log says why. A lease nobody releases ends at its expiry, and the next one goes. A holder past its expiry holds nothing, even if its run was interrupted, and two leases never hold one resource, even with several `decree` processes sharing the project.

## Who goes next

Per resource, one holder at a time:

1. When the resource is free, the oldest lease waiting in its current mode goes.
2. If none waits, the resource lingers in its mode for `linger`; a lease for that mode asked for meanwhile goes with no swap.
3. After the linger, or with no mode set yet, the oldest waiting lease goes, and its mode is swapped in.
4. Once the resource has been in its current mode longer than `max_wait` (counted from the swap that put it in) and a lease for another mode waits, that mode gets no more grants and does not linger: the oldest lease for another mode goes next. This is the time slice (quantum) of round-robin scheduling: a mode that was just swapped in serves its own leases until its slice is used up.

## What it logs

Each grant, swap, hold, release, expiry, cap and refused renewal is a line in the run's log, such as `lease: grant resource=gpu mode=comfy lease=<id> holder="render-box" waited_ms=812 from=llm`, so the [observability](https://github.com/jtmckay/decree/blob/main/docs/reference/observability.md) pipeline shows who held what, and when. Each look adds a few events to the run; prune old runs with `decree prune --older-than <age>`.
