#!/usr/bin/env python3
"""newsletter's gather: the new items of every feed in $DECREE_LIB/newsletter/feeds.txt.

Reads RSS 2.0 and Atom, and writes $DECREE_RUN_DIR/items.jsonl, one JSON object per line
(title, link, source, published, summary), newest first, at most $NEWSLETTER_MAX_ITEMS.
An item is new when its link is not in $NEWSLETTER_DIR/seen.tsv. It does not mark items
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
    lib = os.environ["DECREE_LIB"]
    run_dir = os.environ["DECREE_RUN_DIR"]
    newsletter_dir = os.environ.get("NEWSLETTER_DIR", "newsletter")
    max_items = int(os.environ.get("NEWSLETTER_MAX_ITEMS", "60"))

    urls = feed_urls(os.path.join(lib, "newsletter", "feeds.txt"))
    if not urls:
        print("gather: feeds.txt lists no feeds", file=sys.stderr)
        return 1
    seen = seen_links(os.path.join(newsletter_dir, "seen.tsv"))

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
