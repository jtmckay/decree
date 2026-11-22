---
name: registry
machine:
  description: Where each service is, and which lease to get before using it. The table is state/services.git.txt; every five minutes (and at each daemon start) check probes each address and records which answer, so registry.sh hands out live addresses first. It only says where to go; leasing is the lease plugin's.
  state_files:
    services.git.txt: 'The table the project''s owner writes: <server> <service> <url> [lease], one line per service. check reads it; registry.sh reads it for every lookup. Committed.'
    health: 'One line per service from the last check: <server> <service> <up|down> <checked, ms since the epoch>. check writes it; registry.sh reads it to put live addresses first. Not committed: it is this computer''s view.'
  initial: check
  states:
    check:
      description: Probe each address in services.git.txt (any HTTP answer counts as up) and write state/health; fails on a malformed line, naming it.
      invoke: check
      transitions:
        done: done
    done:
      final: true
    failed:
      final: true
scripts:
  check.sh: |
    #!/usr/bin/env bash
    # registry's check: every address in state/services.git.txt gets one GET with a 5s limit
    # (REGISTRY_TIMEOUT, in seconds); any HTTP answer, even an error status, counts as up,
    # since the server is there. Writes state/health whole, then logs one line per service.
    set -euo pipefail
    bash .decree/machines/registry/scripts/registry.sh check
    out="$DECREE_STATE_DIR/health.new"
    : > "$out"
    now() { local t="${EPOCHREALTIME//[!0-9]/}"; echo $(( 10#$t / 1000 )); }
    sed -e 's/#.*//' -e '/^[[:space:]]*$/d' "$DECREE_STATE_DIR/services.git.txt" 2>/dev/null |
      while read -r server service url _; do
        code="$(curl -s -o /dev/null -m "${REGISTRY_TIMEOUT:-5}" -w '%{http_code}' "$url" || true)"
        if [ -n "$code" ] && [ "$code" != 000 ]; then st=up; else st=down; fi
        echo "$server $service $st $(now)" >> "$out"
        echo "registry: $st server=$server service=$service url=$url${code:+ http=$code}"
      done
    mv "$out" "$DECREE_STATE_DIR/health"
  registry.sh: |
    #!/usr/bin/env bash
    # The registry's lookups, for any script of any machine (list `needs: [registry]`). It
    # only says where to go: it never takes a lease.
    #   registry.sh resolve <service> [<server>]
    #       one line per place the service is: <server> <url> <lease>, the ones the last
    #       check found up first, then the unchecked, then the down; exit 1 when none.
    #       <lease> is the lease to get before using it: <resource> (this project's lease
    #       machine), <resource>@<address> (the lease machine of the decree at that address),
    #       or - when it needs none.
    #   registry.sh url <service> [<server>]   the addresses alone, in the same order.
    #   registry.sh env                        <SERVICE>_URL=<address> for each service, its
    #                                          first address as `url` orders them (COMFY_URL,
    #                                          OLLAMA_URL), for `eval`; a variable already set
    #                                          is left out, so the environment wins.
    #   registry.sh list                       the table, with each line's health.
    #   registry.sh check                      fails naming each malformed line of the table.
    # The table is .decree/machines/registry/state/services.git.txt.
    set -euo pipefail
    here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
    table="$here/state/services.git.txt"
    health="$here/state/health"

    # The table's lines, comments and blank lines removed.
    rows() { [ -f "$table" ] || return 0; sed -e 's/#.*//' -e '/^[[:space:]]*$/d' "$table"; }

    # The last check's word for "<server> <service>": up, down, or nothing.
    state_of() { awk -v s="$1" -v v="$2" '$1 == s && $2 == v { w = $3 } END { print w }' "$health" 2>/dev/null || true; }

    # "<server> <service>" -> 0 up, 1 unchecked, 2 down.
    rank() {
      case "$(state_of "$1" "$2")" in up) echo 0 ;; down) echo 2 ;; *) echo 1 ;; esac
    }

    # "<server> <url> <lease>" for each place $1 is (on server $2 only, when given), live first.
    resolve() {
      local found
      found="$(rows | while read -r server service url lease _; do
        [ "$service" = "$1" ] || continue
        [ -z "${2:-}" ] || [ "$server" = "$2" ] || continue
        echo "$(rank "$server" "$service") $server $url ${lease:--}"
      done | sort -s -n -k1,1 | cut -d' ' -f2-)"
      [ -n "$found" ] || { echo "registry: no service \`$1\`${2:+ on server \`$2\`} in $table" >&2; return 1; }
      echo "$found"
    }

    check_rows() {
      local n=0 bad=0 server service url lease extra
      while IFS= read -r line; do
        n=$((n + 1))
        line="${line%%#*}"
        [[ "$line" =~ [^[:space:]] ]] || continue
        read -r server service url lease extra <<<"$line"
        if [ -n "$extra" ] || [ -z "$url" ]; then
          echo "registry: $table:$n: want <server> <service> <url> [lease], got: $line" >&2; bad=1; continue
        fi
        for w in "$server" "$service"; do
          [[ "$w" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]] || { echo "registry: $table:$n: \`$w\` is not one word of letters, digits and ._-" >&2; bad=1; }
        done
        [[ "$url" =~ ^https?:// ]] || { echo "registry: $table:$n: \`$url\` is not an http:// or https:// address" >&2; bad=1; }
        if [ -n "$lease" ] && ! [[ "$lease" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*(@https?://[^[:space:]]+)?$ ]]; then
          echo "registry: $table:$n: lease \`$lease\` is not <resource> or <resource>@<http address>" >&2; bad=1
        fi
      done < <( [ -f "$table" ] && cat "$table" || true )
      return "$bad"
    }

    case "${1:-}" in
      resolve)
        [ -n "${2:-}" ] || { echo "registry: usage: registry.sh resolve <service> [<server>]" >&2; exit 2; }
        resolve "$2" "${3:-}" || exit 1
        ;;
      url)
        [ -n "${2:-}" ] || { echo "registry: usage: registry.sh url <service> [<server>]" >&2; exit 2; }
        found="$(resolve "$2" "${3:-}")" || exit 1
        cut -d' ' -f2 <<<"$found"
        ;;
      env)
        rows | awk '{ print $2 }' | awk '!seen[$0]++' | while read -r service; do
          var="$(printf '%s_URL' "$service" | tr '[:lower:]' '[:upper:]' | tr -c 'A-Z0-9_\n' '_')"
          [ -z "${!var:-}" ] || continue
          printf '%s=%q\n' "$var" "$(resolve "$service" | head -n 1 | cut -d' ' -f2)"
        done
        ;;
      list)
        rows | while read -r server service url lease _; do
          st="$(state_of "$server" "$service")"
          printf '%-12s %-12s %-36s %-16s %s\n' "$server" "$service" "$url" "${lease:--}" "${st:-unchecked}"
        done
        ;;
      check) check_rows ;;
      *) echo "registry: usage: registry.sh resolve <service> [<server>] | url <service> [<server>] | env | list | check" >&2; exit 2 ;;
    esac
  requirements: |
    # What registry's scripts need beyond decree, checked by infra's `run.sh check`.
    # kind    name    version   source
    command   bash
    command   curl
cron:
  boot.md: |
    ---
    cron: '@reboot'
    ---
    Probe every service in the registry.
  health.md: |
    ---
    cron: '*/5 * * * *'
    ---
    Probe every service in the registry.
state:
  services.git.txt: |
    # The services this project can reach, one per line: <server> <service> <url> [lease]
    #   server    a name for the computer that serves it: one word
    #   service   what it serves: one word
    #   url       its base address; check probes it, and any HTTP answer counts as up
    #   lease     optional: the lease to get before using it, from the lease plugin:
    #             <resource> for this project's lease machine, or <resource>@<address> for
    #             the lease machine of the decree at that address (its decree-go-rest).
    # A service on several servers is listed once per server; `registry.sh resolve
    # <service>` gives every place, live ones first, with the lease each needs.
    #
    # For example:
    # box1    render  http://10.0.0.5:8188   box1
    # box2    render  http://10.0.0.9:8188   box2@http://10.0.0.9:8080
    # laptop  search  http://localhost:7700
---
# registry

One table of where things are: each server, the services it runs, their addresses, and which lease to get before using a service. A script asks the registry where to go, and the registry answers; it never takes a lease or calls a service itself. Nothing uses it until you add it:

```bash
decree install registry
```

## The table

Write `.decree/machines/registry/state/services.git.txt`, one service per line, `#` for comments. It is the project's, so it is committed and no install replaces it:

```text
# server  service  url                    lease
box1      render   http://10.0.0.5:8188   box1
box2      render   http://10.0.0.9:8188   box2@http://10.0.0.9:8080
laptop    search   http://localhost:7700
```

- `server` and `service` are one word each. A service on several servers is one line per server.
- `lease` (optional) is what to lease before using the service, from the [`lease`](lease.md) plugin: a `<resource>` of this project's lease machine, or `<resource>@<address>` for the lease machine of another decree, reached at that address through decree-go-rest. Leave it out when the service needs no lease.

## Health

`check` sends each address one GET (5 seconds at most, `REGISTRY_TIMEOUT`) at each daemon start and every five minutes, and records which answered in `state/health`; any HTTP answer, even an error, counts as up. A malformed line fails the run, naming it. Lookups put live addresses first, then unchecked ones, then the down ones.

## Looking things up

From any script (list `needs: [registry]` in the plugin that uses it):

```bash
reg=.decree/machines/registry/scripts/registry.sh
bash "$reg" resolve render         # box1 http://10.0.0.5:8188 box1
                                   # box2 http://10.0.0.9:8188 box2@http://10.0.0.9:8080
bash "$reg" resolve render box2    # the one on box2
bash "$reg" url search             # http://localhost:7700
eval "$(bash "$reg" env)"          # RENDER_URL, SEARCH_URL, ... unless already set
bash "$reg" list                   # the table and each line's health
```

## Then the lease

The registry says where to go; the lease is a separate step. With the line `box1 http://10.0.0.5:8188 box1`, ask this project's lease machine for resource `box1`, wait until it is held, use `http://10.0.0.5:8188`, and release it:

```bash
read -r server url lease < <(bash "$reg" resolve render | head -n 1)
id=$(echo "Render" | decree emit --machine lease --param resource="${lease%%@*}" --param mode=final)
```

For `box2@http://10.0.0.9:8080`, send the same lease message to the decree at that address instead. Which line to take, and whether to try the next one when a lease would wait, is the caller's choice.
