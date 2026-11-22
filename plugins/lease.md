---
name: lease
machine:
  description: One mode per resource at a time, held by one lease (or up to the resource's holders, all in that mode) for a limited time; the next holder is picked to keep mode swaps rare. Ask with params resource and mode (and optionally callback, an address to POST the lease's status to once it is held); the lease is held while its run is in held, until the expires of state/status/leases/<id>.json; give it back with decree reply <id> release.
  concurrency: 16
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
    callback:
      type: string
      default: ''
  state_files:
    resources.git.txt: 'The rules the project''s owner writes: <name> <max_lease> <linger> <max_wait> [modes] [holders], one resource per line. Every script reads it. Committed.'
    swap: 'swap/<resource>.git.sh: how to put each resource into a mode, written by the project''s owner; swap runs it. Committed.'
    leases: 'One folder per resource: queue (the leases waiting, with when each asked), holder (one line per lease that holds it, with its mode and expiry), mode (what the resource is set up for), mode_since (when that mode was swapped in, for max_wait), free_since (when it was last given back) and lock. turn, swap, renew and leave read and write it, each under flock.'
    status: 'What anyone may read to check on a lease or a resource, rewritten under the lock after every change: leases/<lease id>.json ({run, resource, mode, holder, asked, state, position, expires, ended}; state waiting, swapping, granted (no swap needed), held, released, expired, failed or cancelled; kept a day after it ends) and resources/<resource>.json ({resource, mode, mode_since, free_since, holders, queue, updated}). Not committed.'
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
      description: In line, looking at nothing. A lease that leaves the line or the holders, or is granted, wakes the others with again; the deadline (lease.deadline, written by turn) is the next instant the pick can change by time alone (a holder's expiry, the linger's end, the mode's slice), at most a minute away, so a lease whose run died never holds the line for long.
      invoke:
        reply:
          question: This lease waits in line for its resource. Give up its place, or ask to keep it longer once held?
          notify: tell
          timeout_at: lease.deadline
          attention: false
      transitions:
        again:
          target: turn
          description: The line or the holders changed; look again.
        release:
          target: done
          description: Give up this lease's place in line; it is never held.
        renew:
          target: renew
          description: Refused while the lease is not held yet; the log says so.
        error: turn
    swap:
      description: Puts the resource into this lease's mode with state/swap/<resource>.git.sh, a step with a timeout because unloading a model can take minutes; its failure fails the lease and leaves the mode unknown. Then the hold starts.
      invoke:
        script:
          name: swap
          timeout: 30m
      transitions:
        done: callback
    callback:
      description: The hold has started. When the lease was asked for with a callback, POSTs its status (state/status/leases/<id>.json) there (a few tries, a few seconds each), so the asker can start at once; with none, there is nothing to do. If no try is answered with a 2xx, the lease fails and gives the resource back, since nobody knows they hold it.
      invoke:
        script:
          name: callback
          timeout: 1m
      transitions:
        done: held
    held:
      description: The lease holds the resource until its expires (lease.deadline, and the expires of its status), the wait's deadline, so a lease nobody releases ends at its expiry with no looking in between. A renewal moves the deadline; a refused one leaves it.
      invoke:
        reply:
          question: This lease holds its resource. Give it back, or keep it for its duration again?
          notify: tell
          timeout_at: lease.deadline
          attention: false
      transitions:
        again:
          target: turn
          description: A wake meant for the line reached this lease after it was granted; turn finds it held.
        release:
          target: done
          description: Give the resource back now.
        renew:
          target: renew
          description: Keep the resource for this lease's duration again, from now; refused while another lease waits for it.
        error:
          target: turn
          description: The deadline passed; turn ends the lease as expired.
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
  callback.sh: |
    #!/usr/bin/env bash
    # The hold has started: if the lease was asked for with the param callback, POST its
    # status, state/status/leases/<id>.json (Content-Type application/json), so the asker starts as soon as it holds
    # the resource, without looking. Any 2xx answer is delivered. curl tries 3 times, 5s
    # each, 1s apart; with no 2xx the lease fails (leave gives the resource
    # back). The log names the address without its path or query, which may carry a token.
    set -euo pipefail
    . .decree/machines/lease/scripts/lease.sh
    load
    [ -n "$callback" ] || exit 0
    [[ "$callback" =~ ^(https?://[^/?#]+) ]] && where="${BASH_REMATCH[1]}"
    rc=0
    code="$(curl -sS -o /dev/null -w '%{http_code}' --fail --retry 2 --retry-all-errors --retry-delay 1 --max-time 5 \
      -X POST -H 'Content-Type: application/json' --data-binary @"$(lease_file "$me")" "$callback")" || rc=$?
    if (( rc == 0 )); then
      say "callback $(ids "$mode" "$me") to=$where http=$code"
    else
      say "callback failed $(ids "$mode" "$me") to=$where http=${code:-none} curl_exit=$rc reason=\"the holder was never told: the lease is given back\"" >&2
      exit 1
    fi
  lease.sh: |
    # lease's shared code, sourced by its scripts (never run on its own). The rules are the
    # project's $DECREE_STATE_DIR/resources.git.txt. What the machine keeps between runs is in
    # $DECREE_STATE_DIR/leases/<resource>/, read and written only under flock on its lock
    # file, so several decree processes sharing the project never see half a change:
    #   queue       one line per waiting lease: <asked, ms since the epoch> <lease id> <mode>
    #   holder      one line per lease holding it, all in one mode (at most the resource's
    #               holders): <lease id> <mode> <expiry, ms> <swapping|granted|held>;
    #               empty or absent when nobody holds it
    #   mode        what the resource is set up for; empty when unknown
    #   free_since  when the resource was last given back, ms
    #   mode_since  when the current mode was swapped in, ms (max_wait's slice starts there)
    # What anyone may read, in $DECREE_STATE_DIR/status/, rewritten under the lock after
    # every change (publish), each file through a rename:
    #   leases/<lease id>.json  {run, resource, mode, holder, asked, state, position (while
    #                waiting, 1 first), expires (once granted), ended (once over)}; state is
    #                waiting, swapping, granted (in the mode already set), held,
    #                released, expired, failed or cancelled (released before it was held).
    #                The lease holds the resource while state is held and the time is
    #                before expires. Kept a day after it ends.
    #   resources/<resource>.json  {resource, mode, mode_since, free_since, holders: [{lease,
    #                mode, state, expires}], queue: [{lease, mode, asked, position}], updated}.
    # A run's own files, in $DECREE_RUN_DIR: lease.asked (when it first asked), lease.granted,
    # and lease.deadline, the instant its waiting or held wait ends (timeout_at).
    # A holder past its expiry holds nothing, whatever state its run is in. Every grant,
    # release, expiry, refused renewal and swap is one line in the run's log, `lease: <what>`
    # then key=value pairs naming the resource, the mode and the lease.
    set -euo pipefail
    lease_rules="${DECREE_STATE_DIR}/resources.git.txt"
    me="${DECREE_MESSAGE_ID}"
    decree_bin="${DECREE:-decree}"
    status_dir="${DECREE_STATE_DIR}/status"

    # A path as the log shows it: relative to the project root, the scripts' working directory.
    rel() { echo "${1#"$PWD"/}"; }

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

    # When the reply this run last received reached the queue, in ms, from its id: the time
    # that counts for a reply, as for decree's own timeout. Empty when there is none.
    reply_ms() {
      local f="${DECREE_RECEIVED:-}"
      [ -n "$f" ] && [ -f "$f" ] || return 0
      id_ms "$(sed -n 's/^id:[[:space:]]*//p' "$f" | head -n 1)"
    }

    # When message $1 reached the queue, in ms, from its id (YYYYMMDDTHHMMSS.ffffffZ-xxxxxx,
    # UTC). Empty for an id of another shape.
    id_ms() {
      local id="$1"
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
    # Every resource there is: "<name> <max_lease> <linger> <max_wait> <modes>", the defaults
    # filled in, modes being a,b or * (any).
    table() { rules | awk '{ if (NF < 3) $3 = "5s"; if (NF < 4) $4 = "30m"; if (NF < 5) $5 = "*"; if (NF < 6) $6 = 1; print }'; }
    resource_names() { local n; n="$(table | awk '{ print $1 }' | paste -sd ' ' -)"; echo "${n:-none}"; }

    # Makes $1 the resource this script works on, or fails naming the problem. Sets res,
    # modes_, max_holders, max_lease_ms, linger_ms, max_wait_ms, dur_ms_ (capped), capped and dir.
    use_resource() {
      res="$1"
      local line name max linger wait extra
      line="$(table | awk -v r="$res" '$1 == r { print; exit }')"
      [ -n "$line" ] || die "unknown resource \`$res\`. The resources are: $(resource_names) (in $(rel "$lease_rules"))."
      read -r name max linger wait modes_ max_holders extra <<<"$line"
      [ -z "$extra" ] || die "$(rel "$lease_rules"): the line for \`$res\` has more than <name> <max_lease> <linger> <max_wait> [modes] [holders]"
      [[ "$max_holders" =~ ^[1-9][0-9]{0,5}$ ]] || die "$(rel "$lease_rules"): holders \`$max_holders\` for \`$res\` is not a whole number from 1"
      max_lease_ms="$(dur_ms "$max")" || die "$(rel "$lease_rules"): max_lease \`$max\` for \`$res\` is not a duration such as 15m"
      linger_ms="$(dur_ms "$linger")" || die "$(rel "$lease_rules"): linger \`$linger\` for \`$res\` is not a duration such as 5s"
      max_wait_ms="$(dur_ms "$wait")" || die "$(rel "$lease_rules"): max_wait \`$wait\` for \`$res\` is not a duration such as 30m"
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
    # Whether the resource in use takes this lease's mode.
    takes_mode() { [ "$modes_" = "*" ] || [[ ",$modes_," == *",$mode,"* ]]; }

    # Reads the run's params and its resource's rules, or fails naming the problem. Sets
    # mode, holder_name and callback, and the resource (use_resource).
    load() {
      mode="${DECREE_DATA_MODE:-}"
      holder_name="${DECREE_DATA_HOLDER:-}"
      callback="${DECREE_DATA_CALLBACK:-}"
      [ -z "$callback" ] || [[ "$callback" =~ ^https?://[^[:space:]]+$ ]] || die "callback \`$callback\` is not an http:// or https:// address"
      local named="${DECREE_DATA_RESOURCE:-}"
      [ -n "$named" ] || die "no resource: ask with the params resource and mode. The resources are: $(resource_names) (in $(rel "$lease_rules"))."
      [ -n "$mode" ] || die "no mode for resource \`$named\`: ask with the param mode, what the resource must be set up for (such as a model name)."
      [[ "$mode" =~ ^[A-Za-z0-9][A-Za-z0-9._:/@+-]*$ ]] || die "mode \`$mode\` for resource \`$named\` is not one word of letters, digits and ._:/@+-"
      use_resource "$named"
      takes_mode || die "resource \`$res\` has no mode \`$mode\`: its modes are $modes_ (in $(rel "$lease_rules"))."
    }

    # Runs "$@" holding the resource's lock. Then, still holding it: when the holders, the
    # mode or its times changed, or a lease left the line, wakes every other lease in line
    # (wake), since its turn may now come; and rewrites the status (publish).
    locked() {
      local fd rc=0 before line_before
      exec {fd}>"$dir/lock"; flock "$fd"
      before="$(snapshot)"; line_before="$(awk '{ print $2 }' "$dir/queue")"
      "$@" || rc=$?
      if [ "$(snapshot)" != "$before" ] || left_line "$line_before"; then wake; fi
      publish
      flock -u "$fd"; exec {fd}>&-
      return "$rc"
    }
    # What a waiting lease's turn depends on, but expiries: a renewal or a swap's heartbeat
    # moves one without changing who may go next.
    snapshot() {
      awk '{ print $1, $2, $4 }' "$dir/holder" 2>/dev/null || true
      cat "$dir/mode" "$dir/free_since" "$dir/mode_since" 2>/dev/null || true
    }
    # Whether a lease of the line $1 (ids, one per line) is no longer in it.
    left_line() {
      local id
      while read -r id; do
        [ -n "$id" ] || continue
        grep -q " $id " "$dir/queue" || return 0
      done <<<"$1"
      return 1
    }

    # Queues the reply `again` for each other lease in line whose run is live and has no
    # reply queued yet, so its waiting state ends and its turn looks again. Written into
    # inbox/ directly, not with decree reply, which refuses a run not waiting at that moment:
    # a reply to a run executing now is delivered once it waits.
    wake() {
      local a id m f
      while read -r a id m; do
        [ -n "$id" ] && [ "$id" != "$me" ] || continue
        grep -qsl "^to: $id\$" .decree/inbox/*.md && continue
        alive "$id" || continue
        f=".decree/inbox/lease-again-$id-$(now_ms)"
        printf -- '---\nto: %s\nevent: again\n---\nThe line for %s changed.\n' "$id" "$res" > "$f.tmp"
        mv "$f.tmp" "$f.md"
      done < <(cat "$dir/queue")
    }

    # An instant in ms, as RFC 3339, in jq.
    jq_at='def at: if . == null then null else ((. / 1000 | floor | strftime("%Y-%m-%dT%H:%M:%S")) + "." + ("00" + (. % 1000 | tostring))[-3:] + "Z") end;'

    lease_file() { echo "$status_dir/leases/$1.json"; }
    # Writes $2 to file $1 through a rename.
    put() { printf '%s\n' "$2" > "$1.new"; mv "$1.new" "$1"; }

    # Rewrites status/resources/<res>.json from the state, and the state, position and
    # expires of each lease in line or holding it whose status file exists.
    publish() {
      mkdir -p "$status_dir/resources" "$status_dir/leases"
      local doc
      doc="$(jq -n "$jq_at"' ($h | split("\n") | map(select(length > 0) | split(" "))) as $hs
        | ($q | split("\n") | map(select(length > 0) | split(" ")) | sort_by(.[0] | tonumber)) as $qs
        | { resource: $res,
            mode: (if $mode == "" then null else $mode end),
            mode_since: (if $since == "" then null else ($since | tonumber | at) end),
            free_since: (if ($hs | length) > 0 or $free == "" then null else ($free | tonumber | at) end),
            holders: [$hs[] | {lease: .[0], mode: .[1], state: .[3], expires: (.[2] | tonumber | at)}],
            queue: [$qs | to_entries[] | {lease: .value[1], mode: .value[2], asked: (.value[0] | tonumber | at), position: (.key + 1)}],
            updated: ($now | tonumber | at) }' \
        --arg res "$res" --arg mode "$(current_mode)" --arg now "$(now_ms)" \
        --arg since "$(cat "$dir/mode_since" 2>/dev/null || true)" --arg free "$(cat "$dir/free_since" 2>/dev/null || true)" \
        --rawfile h <(cat "$dir/holder" 2>/dev/null || true) --rawfile q "$dir/queue")"
      put "$status_dir/resources/$res.json" "$doc"
      local f entry
      while read -r entry; do
        f="$(lease_file "$(jq -r .lease <<<"$entry")")"
        [ -f "$f" ] || continue
        put "$f" "$(jq --argjson e "$entry" '.state = ($e.state // "waiting") | .position = $e.position | .expires = $e.expires | del(.ended) | del(..|nulls)' "$f")"
      done < <(jq -c '(.queue[] | {lease, position}), (.holders[] | {lease, state, expires})' <<<"$doc")
    }

    # Writes this lease's status file when it first joins the line.
    open_status() {
      local f; f="$(lease_file "$me")"
      [ -f "$f" ] && return 0
      mkdir -p "$status_dir/leases"
      put "$f" "$(jq -n "$jq_at"' {run: $run, resource: $res, mode: $mode, holder: (if $holder == "" then null else $holder end), asked: ($asked | tonumber | at), state: "waiting"} | del(..|nulls)' \
        --arg run "$me" --arg res "$res" --arg mode "$mode" --arg holder "$holder_name" --arg asked "$1")"
    }

    # Writes the instant this run's waiting or held wait ends, ms $1, to lease.deadline.
    write_deadline() { at "$1" > "$DECREE_RUN_DIR/lease.deadline"; }

    ids() { echo "resource=$res mode=$1 lease=$2"; }

    # This lease's line of the holder file -> h_id h_mode h_exp h_phase (all empty when it
    # holds nothing; h_id is then empty, never another lease's id).
    read_holder() {
      h_id=""; h_mode=""; h_exp=0; h_phase=""
      local line; line="$(awk -v me="$me" '$1 == me { print; exit }' "$dir/holder" 2>/dev/null || true)"
      [ -z "$line" ] || read -r h_id h_mode h_exp h_phase <<<"$line"
    }
    # Sets lease $1's line to <mode> <expiry> <phase>, adding it when absent.
    write_holder() {
      { grep -v "^$1 " "$dir/holder" 2>/dev/null || true; printf '%s %s %s %s\n' "$1" "$2" "$3" "$4"; } > "$dir/holder.new"
      mv "$dir/holder.new" "$dir/holder"
    }
    # Removes lease $1's line; when no line is left, the resource is free from $2 (ms).
    drop_holder() {
      { grep -v "^$1 " "$dir/holder" 2>/dev/null || true; } > "$dir/holder.new"
      mv "$dir/holder.new" "$dir/holder"
      [ -s "$dir/holder" ] || set_free "$2"
    }
    current_mode() { [ -s "$dir/mode" ] && cat "$dir/mode" || true; }
    set_mode() { printf '%s\n' "$1" > "$dir/mode.new"; mv "$dir/mode.new" "$dir/mode"; }
    set_free() { echo "$1" > "$dir/free_since"; }
    unqueue() { grep -v " $1 " "$dir/queue" > "$dir/queue.new" || true; mv "$dir/queue.new" "$dir/queue"; }

    # "<status> <state>" of a run, as decree status reports it; "finished" for an unknown run.
    run_state() { "$decree_bin" status "$1" --format json 2>/dev/null | jq -r '"\(.status) \(.state // "")"' || echo "finished"; }
    # Whether a run can still act: not finished, not interrupted.
    alive() { case "$(run_state "$1")" in finished*|interrupted*) return 1 ;; *) return 0 ;; esac; }

    # This run's own facts: when it asked, the time its message reached the queue (its id),
    # so leases asked at once by runs executing at once keep the order they were asked in
    # (and it keeps its age across a retry); and whether it was ever granted (a granted lease
    # never goes back in line).
    asked_ms() {
      local f="$DECREE_RUN_DIR/lease.asked"
      [ -s "$f" ] || { local t; t="$(id_ms "$me")"; echo "${t:-$(now_ms)}" > "$f"; }
      cat "$f"
    }
    granted_before() { [ -f "$DECREE_RUN_DIR/lease.granted" ]; }

    # Lease $2 (this one by default) is over: its status's state becomes $1 (released,
    # expired, failed or cancelled), with when it ended; its expires is kept. Nothing to do
    # for a lease with no status, and nothing for one already over.
    end_info() {
      local f; f="$(lease_file "${2:-$me}")"
      [ -f "$f" ] || return 0
      put "$f" "$(jq "$jq_at"' if (.state | IN("released", "expired", "failed", "cancelled")) then . else .state = $state | .ended = ($now | tonumber | at) | del(.position) end' \
        --arg state "$1" --arg now "$(now_ms)" "$f")"
    }
  leave.sh: |
    #!/usr/bin/env bash
    # The lease is over (done or failed): it leaves the line, and if it holds the resource it
    # gives it back, which starts the linger. A lease that failed while swapping
    # leaves the mode unknown, so the next lease swaps again. Safe to re-run.
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
        drop_holder "$me" "$now"
        if [ "${DECREE_STATE:-}" = failed ]; then end_info failed; else end_info released; fi
        if [ "$h_phase" = swapping ]; then
          set_mode ""
          say "release $(ids "$h_mode" "$me") at=$(at "$now") reason=\"failed while swapping: mode unknown\""
        else
          say "release $(ids "$h_mode" "$me") at=$(at "$now")"
        fi
      elif [ -n "$was_queued" ]; then
        if [ "${DECREE_STATE:-}" = failed ]; then end_info failed; else end_info cancelled; fi
        say "release $(ids "$mode" "$me") reason=\"left the line, never held\""
      else
        [ "${DECREE_STATE:-}" != failed ] || end_info failed
      fi
      # A status kept a day after its lease ended is removed.
      local f
      while read -r f; do
        jq -e '.ended' "$f" >/dev/null 2>&1 && rm -f "$f"
      done < <(find "$status_dir/leases" -name '*.json' -mmin +1440 2>/dev/null)
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
          end_info expired
          say "expire $(ids "$mode" "$me") reason=\"no longer the holder\""
          echo expired > "$DECREE_EVENT_FILE"
        else
          say "renew refused $(ids "$mode" "$me") reason=\"not held yet\""
          echo waiting > "$DECREE_EVENT_FILE"
        fi
        return 0
      fi
      if (( sent >= h_exp )); then
        drop_holder "$me" "$now"
        end_info expired
        say "expire $(ids "$h_mode" "$me") at=$(at "$h_exp") reason=\"renew sent at $(at "$sent"), after the expiry\""
        echo expired > "$DECREE_EVENT_FILE"
        return 0
      fi
      local others=0 asked id m
      while read -r asked id m; do
        [ -n "$id" ] && [ "$id" != "$me" ] || continue
        if alive "$id"; then others=$((others + 1)); else unqueue "$id"; end_info cancelled "$id"; fi
      done < <(cat "$dir/queue")
      if (( others > 0 )); then
        say "renew refused $(ids "$h_mode" "$me") expires=$(at "$h_exp") reason=\"$others other lease(s) wait for $res\""
      else
        local exp=$(( now + dur_ms_ ))
        write_holder "$me" "$h_mode" "$exp" held
        write_deadline "$exp"
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
    command   curl
  swap.sh: |
    #!/usr/bin/env bash
    # The lease was granted: put the resource into its mode, then start the hold. A swap runs
    # $DECREE_STATE_DIR/swap/<resource>.git.sh <from> <to> with bash (<from> empty when
    # unknown), with LEASE_RESOURCE set. No script means there is nothing to do, and a lease
    # for the mode already set needs none.
    # While the swap runs, the lease's expiry is kept a duration ahead every second, so if
    # this run is interrupted the resource frees one duration later. If the swap fails, the
    # lease fails and the mode is unknown (leave clears the holder).
    set -euo pipefail
    . .decree/machines/lease/scripts/lease.sh
    load
    from=""
    start() {
      read_holder
      [ "$h_id" = "$me" ] || { say "lost $(ids "$mode" "$me") reason=\"this lease expired while it was interrupted\"" >&2; return 1; }
      from="$(current_mode)"
      [ "$from" = "$mode" ] || set_mode ""
    }
    locked start
    script="${DECREE_STATE_DIR}/swap/${res}.git.sh"
    # No script means there is nothing to do, and nothing to say.
    if [ "$from" != "$mode" ] && [ -f "$script" ]; then
      say "swap $(ids "$mode" "$me") from=${from:-unknown} to=$mode script=$(rel "$script")"
      beat() { read_holder; [ "$h_id" != "$me" ] || write_holder "$me" "$mode" $(( $(now_ms) + dur_ms_ )) swapping; }
      ( while sleep 1; do locked beat; done ) </dev/null >/dev/null 2>&1 &
      heart=$!
      rc=0
      LEASE_RESOURCE="$res" bash "$script" "$from" "$mode" || rc=$?
      kill "$heart" 2>/dev/null || true
      wait "$heart" 2>/dev/null || true
      if (( rc != 0 )); then
        say "swap failed $(ids "$mode" "$me") from=${from:-unknown} to=$mode exit=$rc" >&2
        exit 1
      fi
    fi
    hold() {
      read_holder
      [ "$h_id" = "$me" ] || { say "lost $(ids "$mode" "$me") reason=\"this lease no longer holds $res\"" >&2; return 1; }
      set_mode "$mode"
      [ "$from" = "$mode" ] && [ -s "$dir/mode_since" ] || now_ms > "$dir/mode_since"
      local exp=$(( $(now_ms) + dur_ms_ ))
      write_holder "$me" "$mode" "$exp" held
      write_deadline "$exp"
      say "held $(ids "$mode" "$me") holder=\"$holder_name\" expires=$(at "$exp") status=$(rel "$(lease_file "$me")") answer=\"decree reply $me release|renew\""
    }
    locked hold
  tell.sh: |
    #!/usr/bin/env bash
    # The ask script of waiting and held: on the first wait in line, says where the lease is
    # and how to answer; it says nothing when the lease waits again after a wake or its
    # deadline. held says the same when the hold starts (swap), so here it says nothing.
    set -euo pipefail
    [ "${DECREE_VISITS:-1}" = 1 ] && [ "${DECREE_STATE}" = waiting ] || exit 0
    . .decree/machines/lease/scripts/lease.sh
    load
    position="$(jq -r '.position // 1' "$(lease_file "$me")" 2>/dev/null || echo 1)"
    say "wait $(ids "$mode" "$me") ahead=$(( position - 1 )) status=$(rel "$(lease_file "$me")") answer=\"decree reply $me release\""
  turn.sh: |
    #!/usr/bin/env bash
    # Who goes next, per resource, one mode at a time, held by at most the resource's holders
    # leases at once (1 unless its rules say more):
    #   1. a free resource goes to the oldest lease waiting in its current mode;
    #   2. if none waits, the resource lingers in its mode for linger: a lease for that mode
    #      asked for meanwhile goes with no swap;
    #   3. after the linger, or with no mode set, the oldest lease goes, and its mode is swapped in;
    #   4. max_wait is a time slice per mode, from when it was swapped in (round-robin's
    #      quantum): once the mode has run longer than that and a lease in another mode waits,
    #      the oldest such lease goes next, with no linger. While no other mode waits, the
    #      current mode goes on however long it has run.
    #   5. while the resource is held, the oldest lease waiting in the held mode joins the
    #      holders, no swap, as long as there are fewer than holders of them, nobody is
    #      swapping, and 4 does not pick another mode; any other lease waits for all of them.
    # A holder past its expiry holds nothing: its lease is ended here when its run is finished or
    # interrupted (a live run's deadline ends it). Names granted, wait, held or expired;
    # fails at once on a bad request, naming the problem and the resources there are. On
    # wait it writes lease.deadline, when to look again at the latest (wait_until).
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

    # Waits: writes lease.deadline, the next instant the pick can change by time alone (a
    # holder's expiry, the end of the linger, the end of the mode's slice), at least a second
    # and at most a minute away, so a lease whose run died, which wakes nobody, never holds
    # the line for long. A lease leaving the line or the holders, or being granted, wakes
    # this one sooner (wake).
    wait_until() {
      local now="$1" next=$(( $1 + 60000 )) id m exp phase cur free since
      later() { if (( $1 > now && $1 < next )); then next=$1; fi; }
      while read -r id m exp phase; do
        [ -z "$id" ] || later "$exp"
      done < <(cat "$dir/holder" 2>/dev/null || true)
      cur="$(current_mode)"
      if [ -n "$cur" ]; then
        free="$(cat "$dir/free_since" 2>/dev/null || echo 0)"
        since="$(cat "$dir/mode_since" 2>/dev/null || echo "$now")"
        later $(( free + linger_ms ))
        later $(( since + max_wait_ms ))
      fi
      (( next >= now + 1000 )) || next=$(( now + 1000 ))
      write_deadline "$next"
      echo wait > "$DECREE_EVENT_FILE"
    }

    decide() {
      local now; now="$(now_ms)"
      read_holder
      if [ "$h_id" = "$me" ]; then
        if [ "$h_phase" != held ]; then echo granted > "$DECREE_EVENT_FILE"; return 0; fi
        if (( now >= h_exp )); then
          drop_holder "$me" "$now"
          end_info expired
          say "expire $(ids "$h_mode" "$me") at=$(at "$h_exp")"
          echo expired > "$DECREE_EVENT_FILE"
        else
          echo held > "$DECREE_EVENT_FILE"
        fi
        return 0
      fi
      if granted_before; then
        end_info expired
        say "expire $(ids "$mode" "$me") reason=\"no longer the holder\""
        echo expired > "$DECREE_EVENT_FILE"
        return 0
      fi
      grep -q " $me " "$dir/queue" || echo "$asked $me $mode" >> "$dir/queue"
      open_status "$asked"
      # Every other holder: one past its expiry whose run is over holds nothing and is
      # dropped here; a live one, even past its expiry, still counts until its held wait's
      # deadline ends it, since a renewal it queued before the expiry still wins.
      local id m exp phase st n=0 held_mode="" swapping=""
      while read -r id m exp phase; do
        [ -n "$id" ] || continue
        if (( now >= exp )); then
          st="$(run_state "$id")"
          case "$st" in
            finished*|interrupted*)
              drop_holder "$id" "$exp"
              [ "$phase" = held ] || set_mode ""
              end_info expired "$id"
              say "expire $(ids "$m" "$id") at=$(at "$exp") reason=\"its run is ${st% *} in ${st#* }\" by=$me"
              continue ;;
          esac
        fi
        n=$((n + 1)); held_mode="$m"
        [ "$phase" != swapping ] || swapping=1
      done < <(cat "$dir/holder" 2>/dev/null || true)
      if (( n > 0 )); then
        if [ -n "$swapping" ] || [ "$mode" != "$held_mode" ] || (( n >= max_holders )); then
          wait_until "$now"; return 0
        fi
      fi
      local next
      while :; do
        next="$(pick "$now")"
        [ -n "$next" ] && [ "$next" != "$me" ] && ! alive "$next" || break
        unqueue "$next"
        end_info cancelled "$next"
      done
      if [ "$next" != "$me" ]; then wait_until "$now"; return 0; fi
      unqueue "$me"
      local cur; cur="$(current_mode)"
      if [ "$cur" = "$mode" ]; then phase=granted; else phase=swapping; fi
      write_holder "$me" "$mode" $(( now + dur_ms_ )) "$phase"
      touch "$DECREE_RUN_DIR/lease.granted"
      say "grant $(ids "$mode" "$me") holder=\"$holder_name\" waited_ms=$(( now - asked )) from=${cur:-unknown}$( (( max_holders == 1 )) || echo " holders=$((n + 1))/$max_holders")"
      echo granted > "$DECREE_EVENT_FILE"
    }
    locked decide
state:
  resources.git.txt: |
    # The resources leases are for, one per line:
    #   <name> <max_lease> <linger> <max_wait> [modes] [holders]
    #   name       one word: what a lease asks for with the param resource.
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
    #   modes      the modes a lease may ask for, comma-separated; a lease for any other
    #              fails at once. Leave it out, or write *, to take any mode.
    #   holders    how many leases may hold it at once, all in its current mode (default 1):
    #              while it is held, a lease in that mode joins without a swap until there
    #              are this many; a lease in another mode waits until all of them are done.
    # Durations are decree's: a whole number and s, m, h or d. To put a resource into a mode,
    # write swap/<name>.git.sh beside this file: lease runs it with bash as
    # `swap/<name>.git.sh <from> <to>`. Both files are the project's: committed, and never
    # replaced by an install. For example:
    #
    # box1  15m  5s  30m  draft,final
    # box2  15m  5s  30m  *            4
---
# lease

A "talking stick" for anything several programs want at once and only one (or a few, in the same mode) can use at a time: a machine, a model server, a paid API with a budget, a test device. Each program asks for a **lease**; one lease holds the resource at a time, for a limited time, and gives it back. A resource can be set up for different **modes** (a different model loaded, a different configuration), and switching is slow, so the next holder is picked to keep swaps rare: every lease for the mode already set goes first, as long as nobody in another mode waits too long.

A lease is a decree run of this machine, and its run id is the lease's id. Nothing runs inside decree itself: the line of waiting leases is in the machine's state folder, `.decree/machines/lease/state/`. lease knows nothing about what its resources are; the project's owner names them, and says how to swap each one's mode.

## What it needs

- `bash`, `flock` (util-linux) and `jq`, listed in `machines/lease/scripts/requirements`, so `infra`'s `run.sh check` checks them.
- `decree daemon` running: a waiting lease looks again only when the line changes (a lease leaves it or the holders, or is granted, and wakes the others) or when time alone could change the pick (a holder's expiry, the linger's end, the mode's slice), and at least once a minute; a held one waits, without looking, until its expiry. Both are the deadlines of their waits (`timeout_at`). `decree daemon --once` keeps going through both, until no lease is left waiting or held. Those waits are timers, not questions for a person (`attention: false`), so a lease never puts a file in `.decree/attention/`.

## Adding it

```bash
decree install lease
```

## Setting what can be leased

The project's owner writes the resources in `.decree/machines/lease/state/resources.git.txt`, one per line, `#` for comments. The install writes it once, with no resources and the format in its comments; from then on it is the project's: git commits it, and no install of the plugin replaces it.

```text
# name  max_lease  linger  max_wait  modes        holders
box1    15m        5s      30m       draft,final
box2    15m        5s      30m       llm,image    4
```

- `name`: one word, what a lease asks for.
- `max_lease`: the longest a lease holds the resource.
- `linger` (default `5s`): after a lease gives the resource back and none waits in its mode, the resource stays in that mode this long, so a lease for it asked for meanwhile goes without a swap.
- `max_wait` (default `30m`): a time slice per mode, counted from when the mode was swapped in. Once the resource has been in its mode longer than this and a lease for another mode waits, no more leases in the current mode are granted and the resource does not linger: when it is next free, the oldest lease for another mode goes and its mode is swapped in. While no lease for another mode waits, the current mode goes on however long it has run. The lease holding the resource when the slice runs out keeps it until it is released or expires (a renewal is refused while another lease waits), so a lease for another mode waits at most `max_wait` + `max_lease` + one swap after its mode was swapped out.
- `modes` (optional): the modes a lease may ask for, comma-separated. A lease for any other fails at once, naming the modes there are. Without it, or with `*`, any mode goes.
- `holders` (optional, default `1`): how many leases may hold the resource at once, all in the mode it is in. With `box2` above, up to four `llm` leases hold it together; a fifth `llm` lease waits for one of them, and an `image` lease waits until every `llm` lease is done, then swaps. A lease joining the holders never swaps, and none joins while the first is still swapping. It is a shared lock per mode, an exclusive one across modes.

Durations are decree's: `15m`, `5s`, `2h`.

To put a resource into a mode, write `.decree/machines/lease/state/swap/<name>.git.sh`, committed like the rules. lease runs it with bash, with the current mode (empty when unknown) and the mode wanted, `swap/box1.git.sh <from> <to>`, and `LEASE_RESOURCE` set, as a step with a 30-minute timeout. With no script, there is nothing to do. If a swap fails, its lease fails, the mode becomes unknown, and the next lease swaps again. [docs/services.md](https://github.com/jtmckay/decree/blob/main/docs/services.md) shows swaps that unload one model server's models before another's load.

## Asking for a lease

Send a message for machine `lease` with the params `resource` and `mode` (what the resource must be set up for), and optionally `duration` (capped at the resource's `max_lease`, and that is the default), `holder` (free text for the logs) and `callback` (below):

```bash
id=$(echo "Render the batch" | decree emit --machine lease --param resource=box1 --param mode=final --param holder=render-job)
```

Through decree-go-rest it is one POST, which returns the same id. An unknown resource, a mode the resource does not take, a missing resource or mode, or a `callback` that is not an `http://` or `https://` address fails the run at once, naming the problem and the resources there are. To find which resource to ask for, and where, see the `registry` plugin.

## Checking on a lease

The id `decree emit` printed (or decree-go-rest's POST returned) is the lease's id, its run. Its status is one file in the machine's state folder, so anything that can read the project, decree-go-rest included, checks on a lease by reading it, with no command:

```bash
cat ".decree/machines/lease/state/status/leases/$id.json"
# {"run": "<id>", "resource": "box1", "mode": "final", "holder": "render-job",
#  "asked": "2026-11-22T10:00:00.000Z", "state": "held", "expires": "2026-11-22T10:15:00.000Z"}
```

`state` is:

| `state` | Meaning |
|---|---|
| `waiting` | In line; `position` is its place (1 goes next). |
| `swapping` | Granted; the resource is being put into its mode. |
| `granted` | Granted in the mode already set; the hold starts at once. |
| `held` | It holds the resource until `expires`. |
| `released` | Given back (`ended` says when). |
| `expired` | Its time ran out. |
| `failed` | Its swap or its callback failed; the resource was given back. |
| `cancelled` | It left the line before it was held, or its run stopped while it waited. |

The lease holds the resource while `state` is `held` and the time is before `expires`, so a client reads it once when the hold starts and knows it has the resource until then, or until it releases it; it does not have to look again. A renewal that is granted moves `expires`; a refused one leaves it. The file is written as the lease first joins the line, rewritten after every change to its resource, and kept a day after the lease ends. A request that fails at once (an unknown resource, a bad mode) never joins the line and has no file: its run is `failed` (`decree status "$id" --format json` says why).

Each resource has one too, `.decree/machines/lease/state/status/resources/<resource>.json`: its `mode` and since when (`mode_since`), when it was last given back (`free_since`, while nobody holds it), the `holders` (`lease`, `mode`, `state`, `expires`), the `queue` in order (`lease`, `mode`, `asked`, `position`) and when it was `updated`. Every file is written whole, through a rename, so a reader never sees half of one.

The `held` state waits on `expires` (its `timeout_at`), so the run ends at the expiry, within one daemon pass, with no looking in between. A `renew` queued before the expiry wins even when decree gets to it after.

## Being told when it is held

With `--param callback=<address>`, the lease POSTs its status to that address as soon as the hold starts, so whatever asked can start then, without looking:

```bash
id=$(echo "Render" | decree emit --machine lease --param resource=box1 --param mode=final \
  --param callback=https://render.example/leases/ready)
# later, to that address: POST, Content-Type: application/json
# {"run": "<id>", "resource": "box1", "mode": "final", "state": "held", "expires": "...", ...}
```

It is called once per lease, not on a renewal. Any `2xx` answer counts; curl tries three times, 5 seconds each. If none answers, the lease fails (`failed`) and gives the resource back, since nobody knows they hold it. While it calls, the run is in `callback` and the status already says `held`. Put a token the receiver checks in the address (its path or query); the log names only the scheme and host. It needs `curl`.

## Giving it back, or keeping it

```bash
decree reply "$id" release    # give it back, or leave the line if still waiting
decree reply "$id" renew      # hold it for its duration again, from now
```

A renewal is refused while another lease waits for the resource: the expiry stays, and the run's log says why. A lease nobody releases ends at its expiry, and the next one goes. A holder past its expiry holds nothing, even if its run was interrupted, and no more leases than `holders` ever hold one resource, all in one mode, even with several `decree` processes sharing the project.

## Who goes next

Per resource, one mode at a time, and at most `holders` leases holding it in that mode:

1. When the resource is free, the oldest lease waiting in its current mode goes.
2. If none waits, the resource lingers in its mode for `linger`; a lease for that mode asked for meanwhile goes with no swap.
3. After the linger, or with no mode set yet, the oldest waiting lease goes, and its mode is swapped in.
4. Once the resource has been in its current mode longer than `max_wait` (counted from the swap that put it in) and a lease for another mode waits, that mode gets no more grants and does not linger: the oldest lease for another mode goes next. This is the time slice (quantum) of round-robin scheduling: a mode that was just swapped in serves its own leases until its slice is used up.
5. While the resource is held and has room (fewer than `holders` hold it), the oldest lease waiting in the held mode joins at once, without a swap, unless 4 has picked another mode; then the holders drain, and the other mode goes.

## What it logs

Each grant, swap, hold, release, expiry, cap and refused renewal is a line in the run's log, such as `lease: grant resource=box1 mode=final lease=<id> holder="render-job" waited_ms=812 from=draft`, so the [observability](https://github.com/jtmckay/decree/blob/main/docs/reference/observability.md) pipeline shows who held what, and when. A lease adds events to its run only when something happens to it: a wake when its line changes, or its deadline, at most once a minute while it waits (a held lease adds none until it ends); the `prune` plugin deletes old finished runs. Paths in the log are relative to the project.
