---
name: decree_newsletter
machine:
  description: Gather new items from my feeds, pick and summarise the ones that fit my taste, and write the issue.
  state_files:
    seen.tsv: Links already sent, so an item is never in two issues. gather reads it; deliver appends.
  initial: gather
  states:
    gather:
      invoke: gather
      transitions:
        done: write
    write:
      invoke: write
      transitions:
        done: deliver
    deliver:
      invoke: deliver
      transitions:
        done: done
    done:
      final: true
    failed:
      final: true
scripts:
  deliver.sh: |
    #!/usr/bin/env bash
    # decree_newsletter's deliver: copy the run's issue.md to $NEWSLETTER_DIR/<YYYY-MM-DD>.md (-2, -3
    # when that day already has an issue), record every gathered link in $DECREE_STATE_DIR/seen.tsv
    # with the date, and, if NTFY_URL is set, post the issue's first three lines to
    # $NTFY_URL/$NTFY_TOPIC. Safe to re-run: an identical issue is not copied twice, and a link
    # already in seen.tsv is not added again.
    set -euo pipefail
    dir="${NEWSLETTER_DIR:-newsletter}"
    issue="${DECREE_RUN_DIR}/issue.md"
    items="${DECREE_RUN_DIR}/items.jsonl"
    today="$(date +%F)"
    mkdir -p "${dir}"

    # The day's file name: the first free one, or the one already holding this issue.
    target="${dir}/${today}.md"
    n=1
    while [ -e "${target}" ] && ! cmp -s "${issue}" "${target}"; do
      n=$((n + 1))
      target="${dir}/${today}-${n}.md"
    done
    cp "${issue}" "${target}"
    echo "deliver: ${target}"

    seen="${DECREE_STATE_DIR}/seen.tsv"
    touch "${seen}"
    added=0
    if [ -s "${items}" ]; then
      while IFS= read -r link; do
        if ! cut -f1 "${seen}" | grep -qxF -- "${link}"; then
          printf '%s\t%s\n' "${link}" "${today}" >> "${seen}"
          added=$((added + 1))
        fi
      done < <(jq -r '.link' "${items}")
    fi
    echo "deliver: ${added} links added to ${seen}"

    if [ -z "${NTFY_URL:-}" ]; then
      echo "deliver: NTFY_URL is not set; skipping the ntfy ping"
      exit 0
    fi
    if [ -z "${NTFY_TOPIC:-}" ]; then
      echo "deliver: NTFY_URL is set but NTFY_TOPIC is not" >&2
      exit 1
    fi
    head -n 3 "${target}" | curl -sS --fail --max-time 30 -H "Title: $(basename "${target}")" \
      --data-binary @- "${NTFY_URL%/}/${NTFY_TOPIC}" > /dev/null
    echo "deliver: pinged ${NTFY_URL%/}/${NTFY_TOPIC}"
  feeds.txt: |
    # One RSS or Atom feed URL per line. Lines starting with # are comments.
    https://blog.rust-lang.org/feed.xml
    https://lwn.net/headlines/rss
    https://simonwillison.net/atom/everything/
  gather.py: |
    #!/usr/bin/env python3
    """newsletter's gather: the new items of every feed in feeds.txt, beside this script.

    Reads RSS 2.0 and Atom, and writes $DECREE_RUN_DIR/items.jsonl, one JSON object per line
    (title, link, source, published, summary), newest first, at most $NEWSLETTER_MAX_ITEMS.
    An item is new when its link is not in $DECREE_STATE_DIR/seen.tsv. It does not mark items
    seen: deliver does, so a failed run gathers them again next time. A feed that fails is
    logged and skipped; every feed failing is an error. Python 3 standard library only.
    """

    import email.utils
    import html
    import json
    import os
    import re
    import sys
    import urllib.request
    import xml.etree.ElementTree as ET
    from datetime import datetime, timezone

    ATOM = "{http://www.w3.org/2005/Atom}"
    SUMMARY_CHARS = 500
    TIMEOUT_SECONDS = 30


    def feed_urls(path):
        with open(path, encoding="utf-8") as f:
            lines = (line.strip() for line in f)
            return [line for line in lines if line and not line.startswith("#")]


    def seen_links(path):
        try:
            with open(path, encoding="utf-8") as f:
                return {line.split("\t", 1)[0].strip() for line in f if line.strip()}
        except FileNotFoundError:
            return set()


    def text(element):
        return "" if element is None else "".join(element.itertext()).strip()


    def plain(markup):
        """The text of an HTML summary, on one line, at most SUMMARY_CHARS characters."""
        words = html.unescape(re.sub(r"<[^>]*>", " ", markup)).split()
        return " ".join(words)[:SUMMARY_CHARS]


    def iso_date(value):
        """An RSS (RFC 822) or Atom (RFC 3339) date as UTC ISO 8601, or "" if unreadable."""
        value = value.strip()
        if not value:
            return ""
        try:
            when = email.utils.parsedate_to_datetime(value)
        except (TypeError, ValueError):
            try:
                when = datetime.fromisoformat(value.replace("Z", "+00:00"))
            except ValueError:
                return ""
        if when.tzinfo is None:
            when = when.replace(tzinfo=timezone.utc)
        return when.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


    def rss_items(channel, url):
        source = text(channel.find("title")) or url
        for item in channel.findall("item"):
            yield {
                "title": text(item.find("title")),
                "link": text(item.find("link")) or text(item.find("guid")),
                "source": source,
                "published": iso_date(text(item.find("pubDate"))),
                "summary": plain(text(item.find("description"))),
            }


    def atom_link(entry):
        for link in entry.findall(ATOM + "link"):
            if link.get("rel", "alternate") == "alternate" and link.get("href"):
                return link.get("href").strip()
        return ""


    def atom_items(feed, url):
        source = text(feed.find(ATOM + "title")) or url
        for entry in feed.findall(ATOM + "entry"):
            summary = entry.find(ATOM + "summary")
            if summary is None:
                summary = entry.find(ATOM + "content")
            yield {
                "title": text(entry.find(ATOM + "title")),
                "link": atom_link(entry),
                "source": source,
                "published": iso_date(
                    text(entry.find(ATOM + "published")) or text(entry.find(ATOM + "updated"))
                ),
                "summary": plain(text(summary)),
            }


    def fetch(url):
        request = urllib.request.Request(url, headers={"User-Agent": "decree-newsletter"})
        with urllib.request.urlopen(request, timeout=TIMEOUT_SECONDS) as response:
            root = ET.fromstring(response.read())
        if root.tag == "rss" and root.find("channel") is not None:
            return list(rss_items(root.find("channel"), url))
        if root.tag == ATOM + "feed":
            return list(atom_items(root, url))
        raise ValueError(f"neither RSS 2.0 nor Atom (root element {root.tag})")


    def main():
        run_dir = os.environ["DECREE_RUN_DIR"]
        max_items = int(os.environ.get("NEWSLETTER_MAX_ITEMS", "60"))

        urls = feed_urls(os.path.join(os.path.dirname(os.path.abspath(__file__)), "feeds.txt"))
        if not urls:
            print("gather: feeds.txt lists no feeds", file=sys.stderr)
            return 1
        seen = seen_links(os.path.join(os.environ["DECREE_STATE_DIR"], "seen.tsv"))

        items, failed = {}, 0
        for url in urls:
            try:
                found = fetch(url)
            except Exception as error:  # any failure skips this feed only
                print(f"gather: skipping {url}: {error}", file=sys.stderr)
                failed += 1
                continue
            new = [i for i in found if i["link"] and i["link"] not in seen]
            for item in new:
                items.setdefault(item["link"], item)
            print(f"gather: {url}: {len(found)} items, {len(new)} new")
        if failed == len(urls):
            print(f"gather: all {failed} feeds failed", file=sys.stderr)
            return 1

        # Newest first; ISO 8601 UTC strings sort as dates, and undated items go last.
        newest = sorted(items.values(), key=lambda i: i["published"], reverse=True)[:max_items]
        path = os.path.join(run_dir, "items.jsonl")
        with open(path + ".tmp", "w", encoding="utf-8") as f:
            for item in newest:
                f.write(json.dumps(item, ensure_ascii=False) + "\n")
        os.replace(path + ".tmp", path)
        print(f"gather: {len(newest)} new items in items.jsonl")
        return 0


    if __name__ == "__main__":
        sys.exit(main())
  taste.md: |
    # What I want in my newsletter

    I write Rust and run my own small servers, so I want the things that change how I work: language
    and compiler releases, new tools I could use this week, security fixes for software I am likely
    to run, and the occasional long piece that explains why something works the way it does.
    Practical beats clever.

    I skip announcements of events, job posts, funding news, opinion pieces about which language is
    best, and anything that is mostly a link to somewhere else. If two items are about the same
    thing, keep the better one.

    The format: at most 7 items, the best first, one line each. Each line says what it is and why it
    matters to me, in plain words, and links to it. If nothing is worth my time this week, say so in
    one line rather than padding the list.
  write.sh: |
    #!/usr/bin/env bash
    # decree_newsletter's write: a local model reads taste.md, beside this script, and the run's
    # items.jsonl, and picks and summarises the items worth reading. Writes
    # $DECREE_RUN_DIR/issue.md: a title with the date, then the model's picks as markdown links.
    # With no new items it says so, without calling the model.
    set -euo pipefail
    items="${DECREE_RUN_DIR}/items.jsonl"
    taste=.decree/machines/${DECREE_MACHINE}/scripts/taste.md
    issue="${DECREE_RUN_DIR}/issue.md"
    today="$(date +%F)"

    if [ ! -s "${items}" ]; then
      printf '# Newsletter, %s\n\nNothing new in your feeds since the last issue.\n' "${today}" > "${issue}"
      echo "write: no new items; wrote the nothing-new issue without calling the model"
      exit 0
    fi

    instructions="You write a personal newsletter. The reader describes their taste below. Pick the items that fit it, and follow the format it asks for. Reply with the markdown list only: one line per item, '- [title](link): what it is and why it matters'. Use only links from the items given.

    $(cat "${taste}")"

    request="$(jq -n \
      --arg model "${OLLAMA_MODEL}" \
      --arg system "${instructions}" \
      --rawfile items "${items}" \
      '{model: $model, stream: false, messages: [
         {role: "system", content: $system},
         {role: "user", content: ("The new items, one JSON object per line:\n\n" + $items)}
       ]}')"

    echo "write: asking ${OLLAMA_MODEL} at ${OLLAMA_URL} about $(wc -l < "${items}") items"
    reply="$(curl -sS --fail --max-time 600 -H 'Content-Type: application/json' \
      -d "${request}" "${OLLAMA_URL}/api/chat")"
    picks="$(jq -r '.message.content // empty' <<< "${reply}")"
    if [ -z "${picks//[[:space:]]/}" ]; then
      echo "write: the model's reply has no content: ${reply}" >&2
      exit 1
    fi

    printf '# Newsletter, %s\n\n%s\n' "${today}" "${picks}" > "${issue}.tmp"
    mv "${issue}.tmp" "${issue}"
    echo "write: wrote issue.md"
---
# decree_newsletter

A personal newsletter: the RSS and Atom feeds you choose, gathered on a schedule, picked and summarised to your taste by a local model, and written as one markdown issue, with an optional [ntfy](https://ntfy.sh) ping. It is the simplest machine that does the job, on purpose: three scripts in a straight line, run by cron.

## What it does

The machine remembers one thing between runs, declared under `state_files:`: `seen.tsv`, the links already sent, in `.decree/machines/decree_newsletter/state/` (`$DECREE_STATE_DIR`), one link and the date it was sent per line.

1. `gather` (Python 3, standard library only) fetches every feed in `.decree/machines/decree_newsletter/scripts/feeds.txt`, RSS 2.0 or Atom, and writes the run's `items.jsonl`: `title`, `link`, `source`, `published` and `summary` for each item whose link is not in `seen.tsv`, newest first, at most `NEWSLETTER_MAX_ITEMS`. A feed that fails is logged and skipped; every feed failing fails the run.
2. `write` (bash, `curl` and `jq`) sends `taste.md` and `items.jsonl` to Ollama's `/api/chat`, and writes the run's `issue.md`: a title with the date, then the model's picks as markdown links. With no new items it writes an issue saying so, without calling the model.
3. `deliver` copies `issue.md` to `newsletter/<YYYY-MM-DD>.md`, records every gathered link in `seen.tsv`, and, if `NTFY_URL` is set, posts the issue's first three lines to `$NTFY_URL/$NTFY_TOPIC`.

`gather` does not mark items seen; `deliver` does, so the items of a failed run are gathered again next time.

## What it needs

- Python 3, `curl`, `jq`, and [Ollama](https://ollama.com) with a model: `ollama pull gemma4:e4b`.
- These settings in `.decree/.env`:

  ```sh
  OLLAMA_URL=http://127.0.0.1:11434
  OLLAMA_MODEL=gemma4:e4b
  NEWSLETTER_DIR=newsletter        # where issues go, from the project root
  NEWSLETTER_MAX_ITEMS=60          # the most new items the model reads, newest first
  # NTFY_URL=https://ntfy.sh       # uncomment for a ping on each issue
  # NTFY_TOPIC=my-newsletter
  ```

- A cron file, to run it on a schedule, `.decree/machines/decree_newsletter/cron/newsletter.md`:

  ```markdown
  ---
  cron: "0 7 * * 1"
  machine: newsletter
  ---
  # This week's newsletter
  ```

## How to use it

1. `decree_newsletter` is never edited: write a plugin of your own that extends it (`name: my_newsletter`, `extends: decree_newsletter`) holding `scripts/feeds.txt` (one feed URL per line) and `scripts/taste.md`, the whole prompt about you: write it as you would to a friend picking links for you. It carries `decree_newsletter`'s cron file, which runs once, from it. `decree install <file>` installs both.
2. One issue now: `echo "This week" | decree emit --machine my_newsletter && decree daemon --once`.
3. Every Monday at 07:00, from the cron file: `decree daemon`.

## Growing it

Add each of these only once a real issue shows the need:

| When | Add |
| --- | --- |
| An empty week | `gather` names `nothing_new`, and gets the transition `nothing_new: done`. |
| A poor issue from the local model | `write` checks its output and exits non-zero, and gets `attempts: [local, claude]`. |
| Too many items for the context | `write` scores each item first, keeps the top 20, then writes. |
| Summary-only feeds | `gather` fetches the full text of each new item. |
| Several editions | An `edition` param with `enum`, choosing `machines/decree_newsletter/scripts/<edition>/`, and one cron file each. |
