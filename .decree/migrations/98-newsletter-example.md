---
machine: develop
---
# 98: A newsletter example: feeds in, one markdown issue out

## Overview

The user wants a personal newsletter: specific RSS/Atom sources, gathered on a set cadence, filtered and prioritised to their taste, written as a markdown issue, with an optional ntfy ping. This example is the basic version. The full one will live in existential.

It is the skill's simplest machine on purpose: three scripts in a straight line, run by cron. The README shows what to add later, and when, without adding it.

## Requirements

1. **`examples/newsletter/`**, a decree project (`decree check` passes, `decree graph` written):

   ```yaml
   # .decree/machines/newsletter.yml
   name: newsletter
   description: Gather new items from my feeds, pick and summarise the ones that fit my taste, and write the issue.
   initial: gather
   states:
     gather:     # new items from lib/newsletter/feeds.txt since the last issue, by link -> items.jsonl
       invoke: gather
       transitions: { done: write }
     write:      # a local model reads lib/newsletter/taste.md and items.jsonl, writes issue.md
       invoke: write
       transitions: { done: deliver }
     deliver:    # issue.md to newsletter/<date>.md; an ntfy ping if NTFY_URL is set
       invoke: deliver
       transitions: { done: done }
     done:   { final: true }
     failed: { final: true }
   ```

   - **`cron/newsletter.md`:** `cron: "0 7 * * 1"`, `machine: newsletter`.
   - **`.decree/env`:** `OLLAMA_URL=http://127.0.0.1:11434`, `OLLAMA_MODEL=gemma4:e4b`, `NEWSLETTER_DIR=newsletter`, `NEWSLETTER_MAX_ITEMS=60`, and `NTFY_URL`/`NTFY_TOPIC` commented out.
   - **`lib/newsletter/feeds.txt`:** one feed URL per line, `#` comments, two or three real feeds as examples.
   - **`lib/newsletter/taste.md`:** prose, as the user would write it: what I want, what I skip, the format (at most 7 items, one line each, what it is and why it matters, every item linked).
2. **Scripts**, each doing one job, safe to re-run:
   - **`gather`** (Python 3, standard library only, to show a script can be any executable): reads `feeds.txt`, fetches each feed (RSS 2.0 and Atom; a feed that fails is logged and skipped, but every feed failing is an error), and writes `$DECREE_RUN_DIR/items.jsonl` (`title`, `link`, `source`, `published`, `summary` trimmed to 500 characters) for items whose link is not in `$NEWSLETTER_DIR/seen.tsv`, newest first, at most `NEWSLETTER_MAX_ITEMS`. It does not mark them seen: `deliver` does, so a failed run sends them again next time.
   - **`write`** (bash, `curl` and `jq`): sends `taste.md` and `items.jsonl` to Ollama's `/api/chat` (`stream: false`) and writes `$DECREE_RUN_DIR/issue.md`: a title with the date, then the picked items as markdown links. If `items.jsonl` is empty, it writes an issue saying there was nothing new, without calling the model.
   - **`deliver`** (bash): copies `issue.md` to `$NEWSLETTER_DIR/<YYYY-MM-DD>.md` (not overwriting an existing file: add `-2`, `-3`), appends every gathered link to `seen.tsv` with the date, and if `NTFY_URL` is set posts the issue's first three lines to `$NTFY_URL/$NTFY_TOPIC` with the file name as the title. With `NTFY_URL` unset it says so in its log and succeeds.
3. **README**, short, in the style of the other examples:
   - what it does, the machine and its graph, the files;
   - how to run it once (`echo "This week" | decree emit --machine newsletter && decree process`) and on a schedule (`decree daemon` with the cron file);
   - **"Growing it"**, a table of what to add only once a real issue shows the need, each one line:
     - an empty week: `gather` names `nothing_new`, which goes to `done`;
     - a poor issue from the local model: `write` checks its output (links, item count) and exits non-zero, then `attempts: [local, claude]`;
     - too many items for the context: score each item first (a schema-constrained model, reason before event), keep the top 20, then write; still one script;
     - summary-only feeds: fetch the full text in `gather`;
     - several editions: an `edition` param with `enum`, choosing `lib/newsletter/<edition>/`, and one cron file each;
     - learning from you: rating links in the issue, served by an HTTP front door (decree-go-rest) appending to `ratings.tsv`, which `write` reads as examples;
     - misbehaving feeds: a feed reader (Miniflux, FreshRSS) does the fetching, and `gather` reads its API.
   - One sentence on why not GLiNER here: taste is a judgment, and classifiers are poor at judgments ([docs/routers.md](../../docs/routers.md)).
4. **Tests** (`tests/newsletter_test.rs`), with no network and no model: feeds as local `file://` fixtures (one RSS, one Atom, one broken), Ollama replaced by a stub (a fake `curl` on `PATH`, or a local HTTP server in the test). Cover: a run writes `newsletter/<date>.md` with the stub's picks and records the links in `seen.tsv`; a second run with no new items writes the "nothing new" issue without calling the model; a broken feed is skipped, all feeds broken fails `gather`; `deliver` with `NTFY_URL` set posts once (stubbed), and unset does not.
5. The examples list in the main README and `docs/` (wherever examples are listed), and a CHANGELOG "Added" line.

- Only this migration's scope. No change to decree's behaviour.
- Never edit `.decree/migrations/` or `.decree/runs/`, not even by a search and replace across the repository: exclude both from every bulk edit.
- If anything here contradicts the reference docs in a way you cannot settle, write the question to a file named `STOP` in the run directory (the directory that holds the message file you were given) and end without further changes.
- No test calls a real model or the network. No new crate dependencies; Python 3 standard library only for `gather`.
- Print the evidence for each acceptance criterion at the end of your reply.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass, and `decree check` passes in every example directory.

## Acceptance Criteria

- **Given** `examples/newsletter`
  **When** `decree check` and `decree graph` run there
  **Then** both exit 0, `decree graph` changes nothing, and the machine has the states `gather`, `write`, `deliver`, `done` and `failed`, and no others

- **Given** two fixture feeds with five items and a stubbed model
  **When** a `newsletter` message is processed
  **Then** `newsletter/<today>.md` holds the stub's issue, and `seen.tsv` lists the five links

- **Given** the same feeds again
  **When** a second message is processed
  **Then** the issue says there was nothing new, and the stub model was not called

- **Given** `NTFY_URL` unset
  **When** `deliver` runs
  **Then** it succeeds, and its log says the ping was skipped
