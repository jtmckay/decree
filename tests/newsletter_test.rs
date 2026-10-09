//! `examples/newsletter/`: the `newsletter` machine run through the binary, with no network
//! and no model. The example's `.decree/` is copied to a temp project whose `feeds.txt` lists
//! local `file://` fixtures (`tests/fixtures/newsletter/`: one RSS 2.0, one Atom, one broken),
//! and a stub `curl` first on `PATH` stands in for Ollama's `/api/chat` and for ntfy. Covers:
//! a first run writes `newsletter/<today>.md` with the stub's picks and records the five links
//! in the machine's store, `.decree/store/newsletter/seen.tsv`; a second run writes the "nothing new" issue without calling the model; a
//! broken feed is skipped, and all feeds broken fails `gather`; `deliver` posts to ntfy once
//! with `NTFY_URL` set, and skips the ping, saying so, without it. Skipped without `python3`
//! or `jq`, which the example's scripts need.

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

mod common;
use common::write_script;

/// What the stub model picks; `write` puts it under the issue's title.
const PICKS: &str = "- [Atom entry one](https://atom.example/one): the first entry, and why it matters.\n- [Newest RSS item](https://rss.example/newest): the newest item, worth a look.";

/// Logs each call's URL to `calls`, and its request body to `body-<n>`. `/api/chat` answers
/// as Ollama does with `stream: false`, with `stub/picks.md` as the message; anything else
/// (ntfy) answers `{}`.
const STUB_CURL: &str = r#"#!/usr/bin/env bash
dir="$(dirname "$0")"
url=""; body=""; title=""
while [ $# -gt 0 ]; do
  case "$1" in
    -d|--data-binary) body=$2; shift 2 ;;
    -H) case "$2" in Title:*) title=${2#Title: } ;; esac; shift 2 ;;
    --max-time) shift 2 ;;
    http*) url=$1; shift ;;
    *) shift ;;
  esac
done
n=$(( $(wc -l < "$dir/calls" 2>/dev/null || echo 0) + 1 ))
echo "$url $title" >> "$dir/calls"
if [ "$body" = "@-" ]; then cat > "$dir/body-$n"; else printf '%s' "$body" > "$dir/body-$n"; fi
case "$url" in
  */api/chat) jq -n --rawfile picks "$DECREE_PROJECT_ROOT/stub/picks.md" '{message: {role: "assistant", content: $picks}, done: true}' ;;
  *) echo '{}' ;;
esac
"#;

/// The links in the fixtures, newest first.
const LINKS: [&str; 5] = [
    "https://atom.example/one",
    "https://rss.example/newest",
    "https://rss.example/middle",
    "https://rss.example/oldest",
    "https://atom.example/two",
];

fn has_tools() -> bool {
    let found = ["python3", "jq"]
        .iter()
        .all(|tool| Command::new(tool).arg("--version").output().is_ok());
    if !found {
        eprintln!("python3 or jq is not installed: skipping the newsletter example's runs");
    }
    found
}

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/newsletter")
}

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/newsletter");
    format!("file://{}", path.join(name).display())
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap();
        }
    }
}

fn today() -> String {
    let out = Command::new("date").arg("+%F").output().unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

struct Project {
    tmp: TempDir,
}

impl Project {
    /// The example's project, reading the named fixture feeds.
    fn new(feeds: &[&str]) -> Project {
        let p = Project {
            tmp: TempDir::new().unwrap(),
        };
        copy_dir(&example().join(".decree"), &p.root().join(".decree"));
        // The example commits a sample store to show what it holds; a new project starts empty.
        fs::remove_dir_all(p.root().join(".decree/store")).unwrap();
        // As the README says: copy the template to the `.env` decree reads.
        fs::copy(
            p.root().join(".decree/.env.example"),
            p.root().join(".decree/.env"),
        )
        .unwrap();
        let list: String = feeds.iter().map(|f| fixture(f) + "\n").collect();
        fs::write(
            p.root().join(".decree/lib/newsletter/feeds.txt"),
            format!("# fixtures\n{list}"),
        )
        .unwrap();
        fs::create_dir(p.root().join("stub")).unwrap();
        fs::write(p.root().join("stub/picks.md"), PICKS).unwrap();
        fs::create_dir(p.bin()).unwrap();
        write_script(&p.bin().join("curl"), STUB_CURL);
        p
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    fn bin(&self) -> PathBuf {
        self.root().join("bin")
    }

    /// Queues a `newsletter` message, as the README does, and returns its id.
    fn emit(&self) -> String {
        let out = cargo_bin_cmd!("decree")
            .current_dir(self.root())
            .args(["emit", "--machine", "newsletter", "--format", "json"])
            .write_stdin("This week")
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let emitted: Value = serde_json::from_slice(&out.stdout).unwrap();
        emitted["id"].as_str().unwrap().to_string()
    }

    /// Emits a message and runs `decree process` with the stub `curl` first on `PATH` and
    /// `ntfy` as `NTFY_URL` (unset when `None`); returns the run's id.
    fn run(&self, ntfy: Option<&str>) -> String {
        let id = self.emit();
        let path = std::env::join_paths(
            std::iter::once(self.bin())
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        let mut cmd = cargo_bin_cmd!("decree");
        cmd.current_dir(self.root())
            .env("PATH", path)
            .env("NO_COLOR", "1")
            .env_remove("NTFY_URL")
            .env_remove("NTFY_TOPIC")
            .arg("process");
        if let Some(url) = ntfy {
            cmd.env("NTFY_URL", url).env("NTFY_TOPIC", "news");
        }
        cmd.output().unwrap();
        id
    }

    fn run_dir(&self, id: &str) -> PathBuf {
        self.root().join(".decree/runs").join(id)
    }

    fn final_state(&self, id: &str) -> String {
        let message = fs::read_to_string(self.run_dir(id).join("message.md")).unwrap();
        message
            .lines()
            .find_map(|l| l.strip_prefix("state: "))
            .unwrap()
            .to_string()
    }

    /// The `<from> <event> <to>` of each transition after the claim.
    fn path(&self, id: &str) -> Vec<String> {
        fs::read_to_string(self.run_dir(id).join("events.jsonl"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .filter(|e| e["type"] == "transition" && e["source"] != "claim")
            .map(|e| format!("{} {} {}", e["from"], e["event"], e["to"]).replace('"', ""))
            .collect()
    }

    /// The log of the run's `state` script.
    fn log(&self, id: &str, state: &str) -> String {
        let suffix = format!("-{state}-{state}.log");
        let path = fs::read_dir(self.run_dir(id))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.to_string_lossy().ends_with(&suffix))
            .unwrap_or_else(|| panic!("no {state} log in {id}"));
        fs::read_to_string(path).unwrap()
    }

    /// The stub `curl`'s calls: `<url> <title>` each.
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.bin().join("calls"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    fn body(&self, n: usize) -> String {
        fs::read_to_string(self.bin().join(format!("body-{n}"))).unwrap()
    }

    fn newsletter(&self, file: &str) -> String {
        fs::read_to_string(self.root().join("newsletter").join(file)).unwrap()
    }

    /// `seen.tsv`, in the machine's store.
    fn seen_tsv(&self) -> String {
        fs::read_to_string(self.root().join(".decree/store/newsletter/seen.tsv")).unwrap()
    }

    /// The first column of `seen.tsv`.
    fn seen(&self) -> Vec<String> {
        self.seen_tsv()
            .lines()
            .map(|l| l.split('\t').next().unwrap().to_string())
            .collect()
    }

    fn items(&self, id: &str) -> Vec<Value> {
        fs::read_to_string(self.run_dir(id).join("items.jsonl"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

#[test]
fn a_run_writes_the_stub_issue_and_records_the_links() {
    if !has_tools() {
        return;
    }
    let p = Project::new(&["rss.xml", "atom.xml", "broken.xml"]);
    let id = p.run(None);
    assert_eq!(
        p.path(&id),
        [
            "gather done write",
            "write done deliver",
            "deliver done done"
        ]
    );
    assert_eq!(p.final_state(&id), "done");

    let today = today();
    assert_eq!(
        p.newsletter(&format!("{today}.md")),
        format!("# Newsletter, {today}\n\n{PICKS}\n")
    );
    assert_eq!(p.seen(), LINKS);
    assert!(p
        .seen_tsv()
        .lines()
        .all(|l| l.ends_with(&format!("\t{today}"))));

    // gather: newest first, with the fields the message names, HTML stripped and trimmed.
    let items = p.items(&id);
    let links: Vec<&str> = items.iter().map(|i| i["link"].as_str().unwrap()).collect();
    assert_eq!(links, LINKS);
    assert_eq!(items[0]["source"], "Example Atom");
    assert_eq!(items[0]["published"], "2026-10-07T09:00:00Z");
    assert_eq!(items[1]["title"], "Newest RSS item");
    assert_eq!(items[1]["source"], "Example RSS");
    assert_eq!(items[1]["published"], "2026-10-07T08:00:00Z");
    assert_eq!(items[1]["summary"].as_str().unwrap().chars().count(), 500);
    assert_eq!(items[3]["summary"], "An HTML summary.");
    assert_eq!(items[4]["summary"], "The second Atom entry.");

    // The model was asked once, with the taste and the items, not streaming.
    assert_eq!(p.calls(), ["http://127.0.0.1:11434/api/chat "]);
    let request: Value = serde_json::from_str(&p.body(1)).unwrap();
    assert_eq!(request["model"], "gemma4:e4b");
    assert_eq!(request["stream"], false);
    assert!(request["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("What I want in my newsletter"));
    assert!(request["messages"][1]["content"]
        .as_str()
        .unwrap()
        .contains("https://rss.example/oldest"));

    // The broken feed was skipped, and said so.
    let gather = p.log(&id, "gather");
    assert!(
        gather.contains(&format!("skipping {}", fixture("broken.xml"))),
        "{gather}"
    );
}

#[test]
fn a_second_run_with_nothing_new_does_not_call_the_model() {
    if !has_tools() {
        return;
    }
    let p = Project::new(&["rss.xml", "atom.xml"]);
    p.run(None);
    let id = p.run(None);
    assert_eq!(p.final_state(&id), "done");
    assert!(p.items(&id).is_empty());

    let today = today();
    assert_eq!(
        p.newsletter(&format!("{today}-2.md")),
        format!("# Newsletter, {today}\n\nNothing new in your feeds since the last issue.\n")
    );
    assert_eq!(
        p.newsletter(&format!("{today}.md")),
        format!("# Newsletter, {today}\n\n{PICKS}\n")
    );
    assert_eq!(p.calls().len(), 1, "{:?}", p.calls());
    assert_eq!(p.seen(), LINKS);
}

#[test]
fn every_feed_broken_fails_gather() {
    if !has_tools() {
        return;
    }
    let p = Project::new(&["broken.xml", "missing.xml"]);
    let id = p.run(None);
    assert_eq!(p.path(&id), ["gather error failed"]);
    assert_eq!(p.final_state(&id), "failed");
    assert!(p.log(&id, "gather").contains("all 2 feeds failed"));
    assert!(p.calls().is_empty());
    assert!(!p.root().join("newsletter").exists());
}

#[test]
fn deliver_pings_ntfy_once_when_ntfy_url_is_set() {
    if !has_tools() {
        return;
    }
    let p = Project::new(&["rss.xml", "atom.xml"]);
    let id = p.run(Some("http://ntfy.test/"));
    assert_eq!(p.final_state(&id), "done");
    let today = today();
    assert_eq!(
        p.calls(),
        [
            "http://127.0.0.1:11434/api/chat ".to_string(),
            format!("http://ntfy.test/news {today}.md"),
        ]
    );
    // The issue's first three lines: the title, a blank line and the first pick.
    assert_eq!(
        p.body(2),
        format!(
            "# Newsletter, {today}\n\n{}\n",
            PICKS.lines().next().unwrap()
        )
    );
    assert!(p
        .log(&id, "deliver")
        .contains("pinged http://ntfy.test/news"));
}

#[test]
fn deliver_without_ntfy_url_skips_the_ping_and_says_so() {
    if !has_tools() {
        return;
    }
    let p = Project::new(&["rss.xml", "atom.xml"]);
    let id = p.run(None);
    assert_eq!(p.final_state(&id), "done");
    assert!(p.calls().iter().all(|c| c.contains("/api/chat")));
    assert!(p
        .log(&id, "deliver")
        .contains("NTFY_URL is not set; skipping the ntfy ping"));
}

/// Re-running `deliver` (as `decree process --retry` does) neither copies the issue again nor
/// adds a link to `seen.tsv` twice.
#[test]
fn deliver_is_safe_to_re_run() {
    if !has_tools() {
        return;
    }
    let p = Project::new(&["rss.xml", "atom.xml"]);
    let id = p.run(None);
    let script = example().join(".decree/scripts/newsletter/deliver.sh");
    let out = Command::new(script)
        .current_dir(p.root())
        .env("DECREE_RUN_DIR", p.run_dir(&id))
        .env("DECREE_STORE", p.root().join(".decree/store/newsletter"))
        .env("NEWSLETTER_DIR", "newsletter")
        .env_remove("NTFY_URL")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let mut files: Vec<String> = fs::read_dir(p.root().join("newsletter"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(files, [format!("{}.md", today())]);
    assert_eq!(p.seen(), LINKS);
}

/// One machine, `newsletter`, with the states gather, write, deliver, done and failed,
/// `seen.tsv` declared in its store, and a weekly cron file for it.
#[test]
fn newsletter_is_the_only_machine_and_has_five_states() {
    let machines: Vec<String> = fs::read_dir(example().join(".decree/machines"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(machines, ["newsletter.yml"]);
    let text = fs::read_to_string(example().join(".decree/machines/newsletter.yml")).unwrap();
    let machine: serde_norway::Value = serde_norway::from_str(&text).unwrap();
    let states: Vec<&str> = machine["states"]
        .as_mapping()
        .unwrap()
        .keys()
        .map(|k| k.as_str().unwrap())
        .collect();
    assert_eq!(states, ["gather", "write", "deliver", "done", "failed"]);
    let store: Vec<&str> = machine["store"]
        .as_mapping()
        .unwrap()
        .keys()
        .map(|k| k.as_str().unwrap())
        .collect();
    assert_eq!(store, ["seen.tsv"]);
    let cron = fs::read_to_string(example().join(".decree/cron/newsletter.md")).unwrap();
    assert!(
        cron.contains("cron: \"0 7 * * 1\"\nmachine: newsletter\n"),
        "{cron}"
    );
}

/// `seen.tsv` lives in the machine's store: declared under `store:`, drawn in the graph, and
/// read and written by the scripts through `$DECREE_STORE` (docs/reference/scripts.md, Store).
#[test]
fn seen_tsv_is_in_the_store() {
    let graph = fs::read_to_string(example().join(".decree/graph/newsletter.md")).unwrap();
    assert!(
        graph.contains("    note left of gather\n        store: seen.tsv\n    end note\n"),
        "{graph}"
    );
    for script in ["gather.py", "deliver.sh"] {
        let text =
            fs::read_to_string(example().join(".decree/scripts/newsletter").join(script)).unwrap();
        assert!(text.contains("DECREE_STORE"), "{script}");
        assert!(!text.contains("NEWSLETTER_DIR/seen.tsv"), "{script}");
    }
}
