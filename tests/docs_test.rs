//! The documentation holds together: relative Markdown links resolve, the machine examples
//! in `docs/reference/machines.md` are the files in `examples/feature/`, nothing points at the
//! removed implementation spec, `CHANGELOG.md` has the 0.5.0 entry and links the
//! versioning rule, and `SECURITY.md` states the permission mode the built-in machines use. Reads this repository's files only; writes nothing.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every file under `dir`, recursively, skipping the directories in `skip` (paths relative
/// to the repository root).
fn files_under(dir: &Path, skip: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        let rel = path.strip_prefix(repo()).unwrap();
        if skip.iter().any(|s| rel == Path::new(s)) {
            continue;
        }
        if path.is_dir() {
            files_under(&path, skip, out);
        } else {
            out.push(path);
        }
    }
}

/// The Markdown files whose links must resolve: `README.md`, `SECURITY.md`, `docs/`,
/// `examples/*/README.md` and the decree skill (`src/templates/skills/`; `.claude/skills/decree` and
/// `.github/skills/decree` are symlinks to it).
fn markdown_files() -> Vec<PathBuf> {
    let root = repo();
    let mut all = vec![root.join("README.md"), root.join("SECURITY.md")];
    for entry in std::fs::read_dir(root.join("examples")).unwrap() {
        let readme = entry.unwrap().path().join("README.md");
        if readme.is_file() {
            all.push(readme);
        }
    }
    for dir in ["docs", "src/templates/skills"] {
        files_under(&root.join(dir), &[], &mut all);
    }
    all.retain(|p| p.extension().is_some_and(|e| e == "md"));
    all.sort();
    all
}

/// The lines of `text` outside fenced code blocks, with their line numbers.
fn prose_lines(text: &str) -> Vec<(usize, &str)> {
    let mut fence: Option<&str> = None;
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        let marker = ["````", "```", "~~~"]
            .into_iter()
            .find(|m| trimmed.starts_with(m));
        match (fence, marker) {
            (None, Some(m)) => fence = Some(m),
            (Some(f), Some(m)) if m == f && trimmed.trim_end() == f => fence = None,
            (None, None) => out.push((i + 1, line)),
            _ => {}
        }
    }
    out
}

/// `line` with inline code spans removed.
fn without_code_spans(line: &str) -> String {
    let mut out = String::new();
    let mut parts = line.split('`');
    if let Some(first) = parts.next() {
        out.push_str(first);
    }
    for (i, part) in parts.enumerate() {
        if i % 2 == 1 {
            out.push_str(part);
        }
    }
    out
}

/// The targets of the inline links `[text](target)` in `line`.
fn link_targets(line: &str) -> Vec<String> {
    let line = without_code_spans(line);
    let mut targets = Vec::new();
    let mut rest = line.as_str();
    while let Some(i) = rest.find("](") {
        let after = &rest[i + 2..];
        let Some(end) = after.find(')') else { break };
        let target = match after.strip_prefix('<').and_then(|a| a.split_once('>')) {
            Some((t, _)) => t,
            None => {
                let t = after[..end].trim();
                t.split_once(' ').map_or(t, |(t, _)| t)
            }
        };
        targets.push(target.to_string());
        rest = &after[end..];
    }
    targets
}

/// GitHub's anchor for a heading: lowercase, drop everything but letters, digits, `-`,
/// `_` and spaces, then spaces to `-`.
fn slug(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == ' ')
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// The heading anchors of a Markdown file, with GitHub's `-1`, `-2` suffixes for repeats.
fn anchors(text: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    for (_, line) in prose_lines(text) {
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if !(1..=6).contains(&hashes) || !line[hashes..].starts_with(' ') {
            continue;
        }
        let base = slug(&line[hashes..].replace('`', ""));
        let mut anchor = base.clone();
        let mut n = 0;
        while seen.contains(&anchor) {
            n += 1;
            anchor = format!("{base}-{n}");
        }
        seen.insert(anchor);
    }
    seen
}

#[test]
fn relative_markdown_links_resolve() {
    let root = repo();
    let mut broken = Vec::new();
    let files = markdown_files();
    assert!(files.len() > 10, "found only {files:?}");
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        for (line_no, line) in prose_lines(&text) {
            for target in link_targets(line) {
                if target.is_empty() || target.contains("://") || target.starts_with("mailto:") {
                    continue;
                }
                let (path, anchor) = match target.split_once('#') {
                    Some((p, a)) => (p, Some(a)),
                    None => (target.as_str(), None),
                };
                let resolved = if path.is_empty() {
                    file.clone()
                } else {
                    file.parent().unwrap().join(path)
                };
                let where_ = format!(
                    "{}:{line_no}: {target}",
                    file.strip_prefix(&root).unwrap().display()
                );
                if !resolved.exists() {
                    broken.push(format!("{where_}: no such file"));
                    continue;
                }
                if let Some(anchor) = anchor {
                    let ok = resolved.extension().is_some_and(|e| e == "md")
                        && anchors(&std::fs::read_to_string(&resolved).unwrap()).contains(anchor);
                    if !ok {
                        broken.push(format!("{where_}: no such heading"));
                    }
                }
            }
        }
    }
    assert!(broken.is_empty(), "broken links:\n{}", broken.join("\n"));
}

#[test]
fn machine_examples_are_the_feature_example_machines() {
    let root = repo();
    let text = std::fs::read_to_string(root.join("docs/reference/machines.md")).unwrap();
    let mut examples = Vec::new();
    let mut block: Option<Vec<&str>> = None;
    for line in text.lines() {
        match &mut block {
            None if line == "```yaml" => block = Some(Vec::new()),
            Some(lines) if line == "```" => {
                let name = lines
                    .iter()
                    .find(|l| !l.trim().is_empty() && !l.starts_with('#'))
                    .and_then(|l| l.strip_prefix("name: "))
                    .map(str::trim);
                if let Some(name) = name {
                    examples.push((name.to_string(), format!("{}\n", lines.join("\n"))));
                }
                block = None;
            }
            Some(lines) => lines.push(line),
            None => {}
        }
    }
    let names: Vec<&str> = examples.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["hello", "deploy", "ship", "feature"]);
    for (name, example) in &examples {
        let file = root.join(format!("examples/feature/.decree/machines/{name}.yml"));
        let machine = std::fs::read_to_string(&file).unwrap();
        assert_eq!(
            example,
            &machine,
            "docs/reference/machines.md example `{name}` differs from {}",
            file.display()
        );
    }
}

/// The repository's files as git sees them: tracked, plus untracked ones not ignored, so
/// `target/`, `.decree/inbox/`, `.decree/runs/` and other ignored local state are left out.
/// Without git (a source tarball), every file except `.git/` and `target/`.
fn repository_files() -> Vec<PathBuf> {
    let root = repo();
    let listed = std::process::Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(&root)
        .output()
        .ok()
        .filter(|o| o.status.success());
    match listed {
        Some(out) => String::from_utf8(out.stdout)
            .unwrap()
            .split('\0')
            .filter(|p| !p.is_empty())
            .map(|p| root.join(p))
            .filter(|p| p.is_file())
            .collect(),
        None => {
            let mut files = Vec::new();
            files_under(&root, &[".git", "target"], &mut files);
            files
        }
    }
}

#[test]
fn nothing_mentions_the_removed_spec() {
    let root = repo();
    // Migrations are immutable, so theirs stay.
    let migrations = root.join(".decree/migrations");
    let needle = concat!("0.5", "-spec").as_bytes();
    let hits: Vec<String> = repository_files()
        .iter()
        .filter(|f| !f.starts_with(&migrations))
        .filter(|f| {
            std::fs::read(f).is_ok_and(|bytes| bytes.windows(needle.len()).any(|w| w == needle))
        })
        .map(|f| f.strip_prefix(&root).unwrap().display().to_string())
        .collect();
    assert!(
        hits.is_empty(),
        "files that mention the removed spec: {hits:?}"
    );
}

#[test]
fn slugs_follow_github() {
    assert_eq!(
        slug("Invoke: the state's function"),
        "invoke-the-states-function"
    );
    assert_eq!(slug("events.jsonl"), "eventsjsonl");
    assert_eq!(
        slug("D27: Q1: the processed.md ledger"),
        "d27-q1-the-processedmd-ledger"
    );
    assert_eq!(
        link_targets("see [a](x.md#y) and `[b](no.md)` and [c](<z w.md>)"),
        ["x.md#y", "z w.md"]
    );
}

/// `CHANGELOG.md` follows Keep a Changelog, has a 0.5.0 entry, and links the versioning
/// rule (docs/reference/README.md, Versioning), whose anchor exists.
#[test]
fn the_changelog_has_the_0_5_0_entry_and_links_the_versioning_rule() {
    let changelog = std::fs::read_to_string(repo().join("CHANGELOG.md")).unwrap();
    assert!(changelog.starts_with("# Changelog\n"));
    assert!(changelog.contains("https://keepachangelog.com/en/1.1.0/"));
    assert!(changelog.lines().any(|l| l.starts_with("## [0.5.0]")));
    assert!(changelog.contains("(docs/reference/README.md#versioning)"));
    let reference = std::fs::read_to_string(repo().join("docs/reference/README.md")).unwrap();
    assert!(anchors(&reference).contains("versioning"));
    assert!(reference.contains("(../../CHANGELOG.md)"));
}

/// The value of `KEY="${KEY:-value}"` in a shell script.
fn shell_default(script: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=\"${{{key}:-");
    script
        .lines()
        .find_map(|l| l.trim().strip_prefix(&prefix)?.strip_suffix("}\""))
        .map(str::to_string)
}

/// `SECURITY.md` has the usual sections, and says what `src/templates/ai/claude.sh` does:
/// Claude runs with `--permission-mode auto` unless `CLAUDE_PERMISSION_MODE` says otherwise.
#[test]
fn the_security_policy_matches_the_claude_script() {
    let root = repo();
    let policy = std::fs::read_to_string(root.join("SECURITY.md")).unwrap();
    let headings = anchors(&policy);
    for heading in [
        "supported-versions",
        "reporting-a-vulnerability",
        "the-security-model",
        "recommendations",
    ] {
        assert!(headings.contains(heading), "SECURITY.md has no {heading}");
    }
    assert!(policy.contains("Report a vulnerability"));
    assert!(policy.contains("`--permission-mode auto`"));
    assert!(policy.contains("`CLAUDE_PERMISSION_MODE`"));
    for dir in ["inbox", "migrations", "cron"] {
        assert!(policy.contains(&format!("`.decree/{dir}/`")), "{dir}");
    }

    let script = std::fs::read_to_string(root.join("src/templates/ai/claude.sh")).unwrap();
    assert_eq!(
        shell_default(&script, "CLAUDE_PERMISSION_MODE").as_deref(),
        Some("auto"),
        "claude.sh's default permission mode changed; update SECURITY.md"
    );
    assert!(script.contains("--permission-mode \"${CLAUDE_PERMISSION_MODE}\""));
}

/// The README links `SECURITY.md` and the changelog, and says there is no upgrade path
/// from 0.4.
#[test]
fn the_readme_links_the_security_policy_and_the_0_4_note() {
    let readme = std::fs::read_to_string(repo().join("README.md")).unwrap();
    assert!(readme.contains("(SECURITY.md)"));
    assert!(readme.contains("no upgrade path"));
    assert!(readme.contains("(CHANGELOG.md#050---"));
}

/// The removed statement of work is gone, and nothing points at it.
#[test]
fn nothing_mentions_the_removed_statement_of_work() {
    let root = repo();
    // Migrations are immutable, so theirs stay.
    let migrations = root.join(".decree/migrations");
    let needle = concat!("S", "OW.md").as_bytes();
    assert!(!root.join(concat!("S", "OW.md")).exists());
    let hits: Vec<String> = repository_files()
        .iter()
        .filter(|f| !f.starts_with(&migrations))
        .filter(|f| {
            std::fs::read(f).is_ok_and(|bytes| bytes.windows(needle.len()).any(|w| w == needle))
        })
        .map(|f| f.strip_prefix(&root).unwrap().display().to_string())
        .collect();
    assert!(hits.is_empty(), "files that mention it: {hits:?}");
}
