---
machine: develop
---
# A12: The official `lease` plugin: one holder per resource, batched by mode, with linger and max_wait

## Overview

Several programs may want one constrained resource at the same time: a GPU, one LLM server, a paid API with a budget. The requests may come through decree-go-rest, `decree emit`, or other machines. Swapping what the resource is set up for (unloading one model and loading another) is slow. So it pays to finish every request for the mode already loaded before switching, as long as nobody waits too long. The user wants a generic "talking stick" for this:
- the owner sets each resource's rules;
- the holder gets the resource for a limited time, and is expected to give it back;
- the next holder is chosen to keep switches rare.

existential's `gpu_lease` plugin is one instance: one GPU, two hard-coded modes, strict arrival order, a 30 minute hold, swap by unloading ComfyUI or ollama. This migration makes the general version as a plugin. Nothing changes in decree's core: leases were rejected for core on 2026-10-15. A lease is a run, and the queue is the machine's store.

Prior art, for the names and behaviour:
- etcd and Kubernetes leases: grant with a time to live, renew (keepalive), release (revoke); a lease past its time holds nothing.
- Kafka's `linger.ms`: wait briefly for more work before switching.
- Batch scheduling with setup times: serve the current setup first, with an age bound so nothing starves.
- existential's `gpu_lease`: the run as the lease, the wake event, the swap step.

## Outcome

### Where it lives

- `lease` is an official plugin: `plugins/lease.md` in this repository, built into decree like the other official plugins (A10). Its body explains, for a person who has never seen decree:
  - how to add it to a project (`decree plugin add lease`);
  - how to set the rules;
  - how to ask for a lease, how to know it is held, and how to release or renew it.

### Asking for and giving back a lease

- **Ask:** a message for machine `lease` with params `resource` (required), `mode` (what the resource must be set up for, such as a model name), optional `duration` and optional `holder` (free text, for logs). The run's id is the lease's id. Through decree-go-rest it is one POST that returns that id.
- **Granted:** a lease is held exactly while its run's state is `held`. A client learns it holds the resource from `decree status <id> --format json`. The other state names are the implementer's choice.
- **Give back:** `decree event <id> release` ends the lease, whether it is held or still waiting in line. `decree event <id> renew` extends a held lease by its duration again, but only while no other lease waits for that resource. Otherwise the renewal is refused, the expiry stays as it was, and the run's log says why.
- **Duration:** a lease lasts its `duration`, capped at the resource's `max_lease` (and the cap is logged). With no `duration`, it lasts `max_lease`. A lease nobody releases ends at its expiry, and the next one goes.

### The rules

The resource's owner writes them in `scripts/lease/resources`, one resource per line, with `#` comments: `<name> <max_lease> <linger> <max_wait>`. Durations use decree's format (`15m`, `5s`). `max_wait` is the longest a resource stays in one mode while a lease for another mode waits; the clock starts when the mode is swapped in (Who goes next, rule 4). The plugin ships one line, `gpu 15m 5s 30m`.

### Swapping modes

`scripts/lease/swap/<resource>.sh <from> <to>` puts the resource into mode `<to>`. `<from>` is empty when the current mode is unknown. A missing script means there is nothing to do. The shipped script only says what it would do, and points to `docs/services.md` for unloading ollama and ComfyUI. A swap is a step with a timeout, never an `onentry`. If a swap fails, its lease fails, the mode becomes unknown, and the next lease swaps again.

### Who goes next

Per resource, with at most one holder at a time:
1. When the resource is free, a waiting lease in the current mode goes first, oldest first.
2. If none waits, the resource lingers in its mode for `linger` (default 5s): a lease for that mode asked for during the linger is granted with no swap.
3. After the linger, or when no mode is set yet, the oldest waiting lease's mode is swapped in, and the leases in that mode go oldest first.
4. A mode's `max_wait` clock starts when it is swapped in. Once the resource has been in its current mode for longer than `max_wait` and a lease for another mode waits, no more leases in the current mode are granted and the resource does not linger: when it is next free, the oldest waiting lease for another mode is granted, its mode is swapped in, and that mode's clock starts. This is a time slice (the round-robin quantum of OS scheduling): a mode that has already run long gives way as soon as another mode asks, and one that was just swapped in keeps serving its own leases until its slice is used up.
   - The lease that holds the resource when the slice is used up keeps it until it is released or expires (renewal is already refused while another lease waits). So a lease for another mode waits at most `max_wait` + `max_lease` + one swap after its mode was swapped out, and the docs say so.
   - While no lease for another mode waits, the current mode goes on, however long it has run.

### Safety and observability

- Two holders never hold one resource, even when a lease's run was interrupted, or when several `decree` processes share the project. The store records each grant's expiry, and a holder past its expiry holds nothing, whatever state its run is in.
- An unknown `resource`, or a missing `mode`, fails the run at once with a message naming the problem and the resources that exist.
- Each grant, release, expiry, refused renewal and swap appears as a line in the run's log. The line names the resource, the mode and the lease, so the existing Loki/Grafana pipeline can show who held what and when.

## Constraints

- No change to decree's core: the plugin uses only machines, scripts, `$DECREE_STORE`, `decree event` and `decree status`. If the plugin can't be built correctly without a core change, write the reason to `STOP` and end without making changes.
- The plugin's machine follows the skill's "simplest machine" rule. Each state it has is there for one of the behaviours above, with a comment saying which.
- Any tool its scripts need beyond POSIX shell and decree (such as `flock` or `jq`) is named in the plugin's body under what it needs, and in its `scripts/lease/requirements` (A08), so `infra` checks it.
- Tests use a stub swap script that records each call, and durations of seconds. No test waits longer than it must.
- Only this migration's scope. existential is not changed.
- Never edit `.decree/migrations/`, `.decree/runs/` or `.decree/processed.md`, not even by a search and replace across the repository: exclude them from every bulk edit.
- If the reference docs and this migration disagree in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new dependencies.
- Print the evidence for each acceptance criterion (test names or command output) at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.
- CHANGELOG lists the new official plugin under Added, and `docs/decisions.md` gets a decision citing the prior art above and saying why this is a plugin and not core.

## Acceptance Criteria

In each criterion, the resource is `gpu` with `max_lease 3s`, `linger 1s` and `max_wait 4s`, unless the criterion says otherwise.

- **Given** no lease on `gpu`
  **When** a lease for mode `x` is asked for
  **Then** its run reaches `held`, the swap stub records `"" -> x` once, and `decree event <id> release` ends the run in `done`

- **Given** lease A holds `gpu` in mode `x`
  **When** lease B asks for mode `x`
  **Then** B is not `held` until A is released, and then B is `held` with no swap recorded

- **Given** A holds `gpu` in `x`, then B asks for `y`, then C asks for `x`
  **When** A is released
  **Then** C is held next with no swap; when C is released, B is held after the linger, and the stub has recorded exactly one `x -> y`

- **Given** A holds `gpu` in `x` and nothing waits
  **When** A is released and a lease D for `x` is asked for within the linger
  **Then** D is held with no swap

- **Given** A holds `gpu` in `x` and B waits for `y`
  **When** A is released and no lease for `x` is asked for within the linger
  **Then** B is held once the linger has passed, after one `x -> y` swap

- **Given** `gpu` was swapped to `x` and leases for `x` keep arriving
  **When** B asks for `y` after `gpu` has been in `x` longer than `max_wait`
  **Then** the next grant goes to B, even though leases for `x` are waiting, and there is no linger before the `x -> y` swap

- **Given** `gpu` was just swapped to `x`, leases for `x` keep arriving, and B asks for `y` at once
  **When** `max_wait` has not yet passed since the swap
  **Then** leases for `x` keep being granted; once `max_wait` has passed since the swap, the next grant goes to B

- **Given** `gpu` has been in `x` longer than `max_wait` and only leases for `x` wait
  **When** the holder is released
  **Then** the next lease for `x` is held, with no swap

- **Given** a held lease nobody releases
  **When** `max_lease` passes
  **Then** its run ends and the next waiting lease is held

- **Given** a lease asked for with `duration: 10s`
  **When** it is held
  **Then** it ends after `max_lease` (3s), and its log names the cap

- **Given** a held lease and another lease waiting
  **When** the holder sends `renew`
  **Then** the renewal is refused in the log, and the lease ends at its original expiry

- **Given** a held lease and nothing waiting
  **When** the holder sends `renew`
  **Then** it is still held after its original expiry

- **Given** a lease still waiting in line
  **When** it is sent `release`
  **Then** it ends in `done` and is never held

- **Given** the holder's run is interrupted
  **When** its expiry passes
  **Then** the next waiting lease is held, and no two leases are ever `held` on `gpu` at once

- **Given** a swap stub that fails for mode `z`
  **When** a lease for `z` is granted
  **Then** that lease's run fails, and the next lease (for any mode) swaps from an unknown mode

- **Given** a lease for resource `tpu`, which is not in `scripts/lease/resources`
  **When** it runs
  **Then** it fails at once naming `tpu` and listing `gpu`

- **Given** a fresh `decree init`
  **When** `decree plugin list` runs, then `decree plugin add lease`
  **Then** the list shows `lease` with its description, and after adding it `decree check` passes
