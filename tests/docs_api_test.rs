//! The documents and examples agree with the binary: every `decree <command>` they write
//! names a real command with flags it accepts, every whole machine passes `decree check`,
//! every state fragment, message frontmatter and `events.jsonl` line validates against its
//! JSON Schema, and the `DECREE_*` variables they name are the ones `src/runtime.rs` sets.
//! So an interface change fails `cargo test` instead of leaving stale docs.
//!
//! The documents are `docs/reference/`, `README.md`, `docs/routers.md`, `docs/services.md`,
//! `SECURITY.md`, `tests/README.md`, every `examples/*/README.md`, the decree skill
//! (`src/templates/skills/decree/`) and `src/templates/help.txt`. `CHANGELOG.md`,
//! `docs/decisions.md` and `docs/code-review.md` are history and are not checked.

use assert_cmd::cargo::cargo_bin_cmd;
use regex::Regex;
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

mod common;
use common::write_script;
#[path = "common/schema.rs"]
mod schema;

/// Machine fragments (a block of states without `name:`) that do not validate against the
/// machine schema's state definition on their own: (document, first state id, reason).
const FRAGMENT_SKIPS: &[(&str, &str, &str)] = &[];

/// Event examples elided with `…`, which cannot be validated: (document, a substring of the
/// example, reason).
const ELIDED_EVENTS: &[(&str, &str, &str)] = &[];

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every file under `dir`, recursively, in path order.
fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.map(|e| e.unwrap().path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            files_under(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// The documents checked, relative to the repository root.
fn documents() -> Vec<String> {
    let root = repo();
    let mut docs = vec![
        "README.md".to_string(),
        "SECURITY.md".to_string(),
        "docs/routers.md".to_string(),
        "docs/services.md".to_string(),
        "tests/README.md".to_string(),
        "src/templates/help.txt".to_string(),
    ];
    let mut more = Vec::new();
    files_under(&root.join("docs/reference"), &mut more);
    files_under(&root.join("src/templates/skills/decree"), &mut more);
    for entry in fs::read_dir(root.join("examples")).unwrap() {
        let readme = entry.unwrap().path().join("README.md");
        if readme.is_file() {
            more.push(readme);
        }
    }
    docs.extend(
        more.iter()
            .filter(|p| p.extension().is_some_and(|e| e == "md"))
            .map(|p| p.strip_prefix(&root).unwrap().display().to_string()),
    );
    docs.sort();
    docs
}

fn read(rel: &str) -> String {
    fs::read_to_string(repo().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

// ---------------------------------------------------------------------------------------
// Markdown: fenced blocks and inline code spans
// ---------------------------------------------------------------------------------------

/// A fenced code block: its info string's first word and its text, dedented by the fence's
/// indentation. `line` is the line number of its first text line.
struct Block {
    lang: String,
    line: usize,
    text: String,
}

/// An inline code span outside fences, with the line it is on.
struct Span {
    line: usize,
    text: String,
}

/// The fenced blocks and inline code spans of a Markdown document.
fn markdown(text: &str) -> (Vec<Block>, Vec<Span>) {
    let mut blocks = Vec::new();
    let mut spans = Vec::new();
    // (marker char, marker length, indent, lang, first line, text)
    let mut open: Option<(char, usize, usize, String, usize, String)> = None;
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let marker = trimmed.chars().next().filter(|c| *c == '`' || *c == '~');
        let run = marker.map_or(0, |m| trimmed.chars().take_while(|c| *c == m).count());
        match &mut open {
            None if run >= 3 => {
                let m = marker.unwrap();
                let lang = trimmed[run..].split_whitespace().next().unwrap_or("");
                open = Some((m, run, indent, lang.to_string(), n + 1, String::new()));
            }
            None => spans.extend(
                code_spans(line)
                    .into_iter()
                    .map(|text| Span { line: n, text }),
            ),
            Some((m, len, _, _, _, _))
                if run >= *len && Some(*m) == marker && trimmed.trim_end().len() == run =>
            {
                let (_, _, _, lang, first, body) = open.take().unwrap();
                blocks.push(Block {
                    lang,
                    line: first,
                    text: body,
                });
            }
            Some((_, _, fence_indent, _, _, body)) => {
                let strip = line.len() - line.trim_start().len();
                body.push_str(&line[strip.min(*fence_indent)..]);
                body.push('\n');
            }
        }
    }
    (blocks, spans)
}

/// The inline code spans of one line: a run of backticks up to the next run of the same
/// length (CommonMark). A table cell's `\|` is a `|`.
fn code_spans(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '`' {
            i += 1;
            continue;
        }
        let len = chars[i..].iter().take_while(|c| **c == '`').count();
        let start = i + len;
        let mut j = start;
        let mut end = None;
        while j < chars.len() {
            if chars[j] == '`' {
                let run = chars[j..].iter().take_while(|c| **c == '`').count();
                if run == len {
                    end = Some(j);
                    break;
                }
                j += run;
            } else {
                j += 1;
            }
        }
        match end {
            Some(e) => {
                let text: String = chars[start..e].iter().collect();
                out.push(text.trim().replace("\\|", "|"));
                i = e + len;
            }
            None => i = start,
        }
    }
    out
}

/// The heredoc bodies in a shell block (`<<'EOF'` … `EOF`), with the line each starts on,
/// relative to the block.
fn heredocs(text: &str) -> Vec<(usize, String)> {
    let start = Regex::new(r#"<<-?\s*['"]?([A-Za-z_]+)['"]?"#).unwrap();
    let mut out = Vec::new();
    let mut open: Option<(String, usize, String)> = None;
    for (i, line) in text.lines().enumerate() {
        match &mut open {
            Some((word, _, body)) => {
                if line.trim() == word {
                    let (_, first, body) = open.take().unwrap();
                    out.push((first, body));
                } else {
                    body.push_str(line);
                    body.push('\n');
                }
            }
            None => {
                if let Some(c) = start.captures(line) {
                    open = Some((c[1].to_string(), i + 1, String::new()));
                }
            }
        }
    }
    out
}

fn is_shell(lang: &str) -> bool {
    matches!(lang, "bash" | "sh" | "shell" | "console" | "zsh")
}

fn is_yaml(lang: &str) -> bool {
    matches!(lang, "yaml" | "yml")
}

// ---------------------------------------------------------------------------------------
// Commands and flags, read from the binary's --help
// ---------------------------------------------------------------------------------------

/// Whether a flag takes a value.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Takes {
    Nothing,
    Value,
    OptionalValue,
}

/// What one command accepts, as its `-h` prints it.
#[derive(Default, Debug)]
struct Accepts {
    flags: BTreeMap<String, Takes>,
    positionals: usize,
}

/// The binary's commands, and the flags `decree` itself takes.
struct Cli {
    commands: BTreeMap<String, Accepts>,
    top: Accepts,
}

fn help(args: &[&str]) -> String {
    let out = cargo_bin_cmd!("decree")
        .args(args)
        .arg("-h")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "decree {args:?} -h failed");
    String::from_utf8(out.stdout).unwrap()
}

/// The lines of the section that starts with `heading` (such as `Options:`).
fn section<'t>(text: &'t str, heading: &str) -> Vec<&'t str> {
    text.lines()
        .skip_while(|l| *l != heading)
        .skip(1)
        .take_while(|l| !l.is_empty())
        .collect()
}

/// The flags and positional arguments in one `-h` output.
fn accepts(text: &str, command: Option<&str>) -> Accepts {
    let option = Regex::new(
        r"^\s+(?:(-[A-Za-z])(?:, )?)?(--[a-z][a-z0-9-]*)?( \[<[^>]+>\]| <[^>]+>)?(?:\.\.\.)?(?:\s{2,}|$)",
    )
    .unwrap();
    let mut out = Accepts::default();
    for line in section(text, "Options:") {
        let c = option
            .captures(line)
            .unwrap_or_else(|| panic!("unexpected option line in -h: {line:?}"));
        let takes = match c.get(3).map(|m| m.as_str()) {
            None => Takes::Nothing,
            Some(v) if v.starts_with(" [") => Takes::OptionalValue,
            Some(_) => Takes::Value,
        };
        for name in [c.get(1), c.get(2)].into_iter().flatten() {
            out.flags.insert(name.as_str().to_string(), takes);
        }
    }
    if let Some(command) = command {
        let usage = text
            .lines()
            .find_map(|l| l.strip_prefix("Usage: "))
            .unwrap();
        let prefix = format!("decree {command}");
        let rest = usage
            .strip_prefix(&prefix)
            .unwrap_or_else(|| panic!("{usage}"));
        out.positionals = rest
            .split_whitespace()
            .filter(|w| *w != "[OPTIONS]")
            .count();
    }
    out
}

fn cli() -> Cli {
    let top = help(&[]);
    let mut commands = BTreeMap::new();
    for line in section(&top, "Commands:") {
        let name = line.split_whitespace().next().unwrap();
        commands.insert(name.to_string(), accepts(&help(&[name]), Some(name)));
    }
    assert!(commands.contains_key("process"), "{top}");
    Cli {
        commands,
        top: accepts(&top, None),
    }
}

/// Shell words of `text` up to the first unquoted `|`, `;`, `&`, `)`, `#`, redirect, or a
/// quote that does not close on the line. Quotes are removed; `<id>` is a word.
fn shell_words(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            ' ' | '\t' => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            '\'' | '"' => match chars[i + 1..].iter().position(|d| *d == c) {
                Some(len) => {
                    word.extend(&chars[i + 1..i + 1 + len]);
                    in_word = true;
                    i += len + 1;
                }
                None => return words,
            },
            '|' | ';' | '&' | ')' | '`' => break,
            '#' if !in_word => break,
            '>' if !in_word || word.chars().all(|d| d.is_ascii_digit()) => {
                return words;
            }
            '<' if next == Some('<') || next.is_none_or(char::is_whitespace) => break,
            // A placeholder, such as `<wait id>`, is one word.
            '<' if !in_word && chars[i..].contains(&'>') => {
                let len = chars[i..].iter().position(|d| *d == '>').unwrap();
                word.extend(&chars[i..=i + len]);
                in_word = true;
                i += len;
            }
            _ => {
                word.push(c);
                in_word = true;
            }
        }
        i += 1;
    }
    if in_word {
        words.push(word);
    }
    words
}

/// Whether a `decree …` string is usage syntax (`[--flag]`, `<A|B>`, `ID`, `...`) rather
/// than a command as typed: then only its flags are checked.
fn is_usage(rest: &str) -> bool {
    let placeholder = Regex::new(r"(^|\s)[A-Z][A-Z0-9_]*(=[A-Z]+)?(\s|$)").unwrap();
    rest.contains('[')
        || rest.contains("...")
        || rest.contains('…')
        || rest.contains('|')
        || placeholder.is_match(rest)
}

/// Every `decree …` in shell text: the text after `decree`, for each occurrence that starts
/// a command (not `.decree/`, `projects/decree` or `decree-dash`).
fn invocations(text: &str) -> Vec<String> {
    let re = Regex::new(r#"(?m)(?:^|[\s(;|&'"$])decree(?:[ \t]+|$)"#).unwrap();
    after(&re, text)
}

/// Every `decree …` in inline code: at its start, or after a nested backtick, `(`, `|`, `;`
/// or `&`, so an error message such as ``not inside a decree project`` is not a command.
fn inline_invocations(text: &str) -> Vec<String> {
    let re = Regex::new(r"(?:^|[`(;|&]\s*)decree(?:[ \t]+|$)").unwrap();
    after(&re, text)
}

/// The rest of the line after each match of `re`.
fn after(re: &Regex, text: &str) -> Vec<String> {
    re.find_iter(text)
        .map(|m| text[m.end()..].lines().next().unwrap_or("").to_string())
        .collect()
}

/// Problems with one `decree <rest>` against the binary's commands.
fn check_invocation(cli: &Cli, rest: &str, usage: bool) -> Vec<String> {
    let mut errors = Vec::new();
    let words: Vec<String> = if usage {
        rest.split_whitespace().map(str::to_string).collect()
    } else {
        shell_words(rest)
    };
    let Some(first) = words.first() else {
        return errors;
    };
    if first.starts_with('<') || first.starts_with('[') || first.starts_with('$') {
        return errors; // `decree <command> …`: a placeholder for any command
    }
    let (accepts, args) = if first.starts_with('-') {
        (&cli.top, &words[..])
    } else {
        match cli.commands.get(first.as_str()) {
            Some(a) => (a, &words[1..]),
            None => return vec![format!("no command `{first}`")],
        }
    };
    let name = if first.starts_with('-') {
        "decree".to_string()
    } else {
        format!("decree {first}")
    };
    if usage {
        let flag = Regex::new(r"(?:^|[\[(|])(--?[A-Za-z][A-Za-z0-9-]*)").unwrap();
        for word in args {
            for c in flag.captures_iter(word) {
                if !accepts.flags.contains_key(&c[1]) {
                    errors.push(format!("{name} has no flag {}", &c[1]));
                }
            }
        }
        return errors;
    }
    let mut positionals = 0;
    let mut i = 0;
    while i < args.len() {
        let word = &args[i];
        if word.starts_with('-') && word.len() > 1 {
            let flag = word.split('=').next().unwrap();
            match accepts.flags.get(flag) {
                None => errors.push(format!("{name} has no flag {flag}")),
                Some(Takes::Value) if !word.contains('=') => i += 1,
                Some(Takes::OptionalValue)
                    if !word.contains('=')
                        && args.get(i + 1).is_some_and(|w| !w.starts_with('-')) =>
                {
                    i += 1
                }
                Some(_) => {}
            }
        } else {
            positionals += 1;
        }
        i += 1;
    }
    if positionals > accepts.positionals {
        errors.push(format!(
            "{name} takes {} argument(s), not {positionals}",
            accepts.positionals
        ));
    }
    errors
}

/// The `decree …` commands a document writes, with their line and whether each is usage
/// syntax: in shell blocks, in GitHub Actions `run:` lines, in inline code, and in
/// `help.txt`'s `Commands:` and `Getting started:` sections.
fn commands_in(rel: &str, text: &str) -> Vec<(usize, String, bool)> {
    let mut out = Vec::new();
    if rel.ends_with(".txt") {
        let mut in_commands = false;
        for (i, line) in text.lines().enumerate() {
            if !line.starts_with(' ') && !line.is_empty() {
                in_commands = matches!(line, "Commands:" | "Getting started:");
            }
            let trimmed = line.trim_start();
            let trimmed = trimmed
                .split_once(". ")
                .filter(|(n, _)| n.chars().all(|c| c.is_ascii_digit()))
                .map_or(trimmed, |(_, rest)| rest);
            if in_commands {
                if let Some(rest) = trimmed.strip_prefix("decree ") {
                    // The description follows after two or more spaces.
                    let command = rest.split("  ").next().unwrap();
                    out.push((i + 1, command.to_string(), true));
                }
            }
            for span in code_spans(line) {
                for rest in inline_invocations(&span) {
                    let usage = is_usage(&rest);
                    out.push((i + 1, rest, usage));
                }
            }
        }
        return out;
    }
    let (blocks, spans) = markdown(text);
    for span in spans {
        for rest in inline_invocations(&span.text) {
            let usage = is_usage(&rest);
            out.push((span.line, rest, usage));
        }
    }
    for block in blocks {
        let shell = is_shell(&block.lang);
        if !shell && !is_yaml(&block.lang) {
            continue;
        }
        let joined = block.text.replace("\\\n", " ");
        for (i, line) in joined.lines().enumerate() {
            let line = if shell {
                line
            } else {
                match line
                    .trim_start()
                    .trim_start_matches("- ")
                    .strip_prefix("run:")
                {
                    Some(command) => command,
                    None => continue,
                }
            };
            for rest in invocations(line) {
                out.push((block.line + i, rest, false));
            }
        }
    }
    out
}

/// Every problem with the commands in one document, as `<file>:<line>: …`.
fn command_errors(cli: &Cli, rel: &str, text: &str) -> Vec<String> {
    let mut errors = Vec::new();
    for (line, rest, usage) in commands_in(rel, text) {
        for e in check_invocation(cli, &rest, usage) {
            errors.push(format!("{rel}:{line}: `decree {rest}`: {e}"));
        }
    }
    errors
}

#[test]
fn every_documented_command_and_flag_exists() {
    let cli = cli();
    let mut errors = Vec::new();
    let mut checked = 0;
    for rel in documents() {
        let text = read(&rel);
        checked += commands_in(&rel, &text).len();
        errors.extend(command_errors(&cli, &rel, &text));
    }
    assert!(checked > 100, "only {checked} commands found");
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn a_flag_decree_does_not_have_fails_naming_the_file_and_the_flag() {
    let cli = cli();
    for doc in [
        "Run:\n\n```bash\ndecree process --no-such-flag\n```\n",
        "Run `decree process --no-such-flag` first.\n",
        "Run `decree process [--no-such-flag]` first.\n",
    ] {
        let errors = command_errors(&cli, "docs/scratch.md", doc);
        assert_eq!(errors.len(), 1, "{doc}: {errors:?}");
        assert!(errors[0].starts_with("docs/scratch.md:"), "{}", errors[0]);
        assert!(errors[0].contains("--no-such-flag"), "{}", errors[0]);
    }
    let errors = command_errors(&cli, "README.md", "`decree retry 01-a`, `decree graph x`\n");
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors[0].contains("no command `retry`"), "{}", errors[0]);
    assert!(errors[1].contains("decree graph takes 0"), "{}", errors[1]);
    // Values, placeholders, quotes and what follows the command are not flags or arguments.
    for ok in [
        "```bash\ndecree event 02-a.w15 retry -m \"a --note\"   # --nothing\n```\n",
        "```bash\necho x | decree emit --machine m --param a=1 <<'EOF'\n--x\nEOF\n```\n",
        "```bash\ndecree process --retry 01-a --state implement > out.txt || true\n```\n",
        "`decree status <id> --format json`, `decree prune --older-than 30d --dry-run`\n",
        "`decree event ID EVENT [-m NOTE]`, `cd projects/decree`, `.decree/inbox/`\n",
    ] {
        let errors = command_errors(&cli, "docs/scratch.md", ok);
        assert!(errors.is_empty(), "{ok}: {errors:?}");
    }
}

// ---------------------------------------------------------------------------------------
// Machines
// ---------------------------------------------------------------------------------------

/// Keys only a state has, by which a block of states is told from other YAML.
const STATE_KEYS: &[&str] = &[
    "invoke",
    "transitions",
    "final",
    "onentry",
    "onexit",
    "states",
];

/// The YAML blocks of every document that are machines or state fragments.
enum MachineBlock {
    Whole(String),
    Fragment(serde_norway::Mapping),
}

fn machine_blocks(text: &str) -> Vec<(usize, MachineBlock)> {
    let (blocks, _) = markdown(text);
    let mut out = Vec::new();
    for block in blocks.into_iter().filter(|b| is_yaml(&b.lang)) {
        if block.text.trim_start().starts_with("---") {
            continue; // a message's frontmatter
        }
        let Ok(serde_norway::Value::Mapping(map)) =
            serde_norway::from_str::<serde_norway::Value>(&block.text)
        else {
            assert!(
                !block.text.contains("states:"),
                "line {}: a machine that is not YAML:\n{}",
                block.line,
                block.text
            );
            continue;
        };
        if map.contains_key("name") && map.contains_key("states") {
            out.push((block.line, MachineBlock::Whole(block.text)));
        } else if !map.is_empty()
            && map.values().all(|v| {
                v.as_mapping()
                    .is_some_and(|m| STATE_KEYS.iter().any(|k| m.contains_key(*k)))
            })
        {
            out.push((block.line, MachineBlock::Fragment(map)));
        }
    }
    out
}

/// What a machine needs around it to pass `decree check`: the scripts it names, and the
/// machines it invokes, routes through or emits to, each with the final states the invoking
/// state handles and the data its `params` set.
#[derive(Default)]
struct Needs {
    scripts: BTreeSet<String>,
    machines: BTreeMap<String, (BTreeSet<String>, BTreeMap<String, &'static str>)>,
}

fn script_list(v: Option<&serde_norway::Value>, needs: &mut Needs) {
    for s in v.and_then(|v| v.as_sequence()).into_iter().flatten() {
        needs.scripts.insert(s.as_str().unwrap().to_string());
    }
}

fn name_of(v: &serde_norway::Value) -> String {
    match v {
        serde_norway::Value::String(s) => s.clone(),
        other => other["name"].as_str().unwrap().to_string(),
    }
}

fn collect_needs(states: &serde_norway::Mapping, needs: &mut Needs) {
    for state in states.values() {
        script_list(state.get("onentry"), needs);
        script_list(state.get("onexit"), needs);
        for m in state
            .get("emits")
            .and_then(|v| v.as_sequence())
            .into_iter()
            .flatten()
        {
            needs
                .machines
                .entry(m.as_str().unwrap().to_string())
                .or_default();
        }
        let finals: BTreeSet<String> = state
            .get("transitions")
            .and_then(|t| t.as_mapping())
            .into_iter()
            .flatten()
            .filter_map(|(k, _)| k.as_str().map(str::to_string))
            .filter(|k| k != "error" && !k.contains('.'))
            .collect();
        match state.get("invoke") {
            Some(serde_norway::Value::String(s)) => {
                needs.scripts.insert(s.clone());
            }
            Some(serde_norway::Value::Mapping(invoke)) => {
                if let Some(s) = invoke.get("script") {
                    needs.scripts.insert(name_of(s));
                }
                if let Some(p) = invoke.get("person") {
                    needs.scripts.insert(p["ask"].as_str().unwrap().to_string());
                }
                if let Some(m) = invoke.get("model") {
                    let router = m.get("router").and_then(|r| r.as_str()).unwrap_or("router");
                    needs.machines.entry(router.to_string()).or_default();
                }
                if let Some(m) = invoke.get("machine") {
                    let entry = needs.machines.entry(name_of(m)).or_default();
                    entry.0.extend(finals);
                    for (k, v) in m
                        .get("params")
                        .and_then(|p| p.as_mapping())
                        .into_iter()
                        .flatten()
                    {
                        let ty = match v {
                            serde_norway::Value::Bool(_) => "bool",
                            serde_norway::Value::Number(n) if n.is_i64() => "int",
                            _ => "string",
                        };
                        entry.1.insert(k.as_str().unwrap().to_string(), ty);
                    }
                }
            }
            _ => {}
        }
        if let Some(children) = state.get("states").and_then(|s| s.as_mapping()) {
            collect_needs(children, needs);
        }
    }
}

/// A machine that passes `decree check` and reaches each of `finals` (plus `failed`), with
/// `data` for the params a parent sets.
fn stub_machine(name: &str, finals: &BTreeSet<String>, data: &BTreeMap<String, &str>) -> String {
    let mut finals = finals.clone();
    finals.remove("failed");
    if finals.is_empty() {
        finals.insert("done".to_string());
    }
    let mut yaml = format!("name: {name}\ndescription: A stub for the docs test.\n");
    if !data.is_empty() {
        yaml.push_str("data:\n");
        for (k, ty) in data {
            let default = match *ty {
                "bool" => "false",
                "int" => "0",
                _ => "\"\"",
            };
            yaml.push_str(&format!("  {k}: {{ type: {ty}, default: {default} }}\n"));
        }
    }
    yaml.push_str("initial: work\nstates:\n  work:\n    invoke: docs_stub\n    transitions:\n");
    for f in &finals {
        yaml.push_str(&format!("      {f}: {f}\n"));
    }
    for f in &finals {
        yaml.push_str(&format!("  {f}: {{ final: true }}\n"));
    }
    yaml.push_str("  failed: { final: true }\n");
    yaml
}

/// `decree check`'s output for `yaml` in a project of its own, with a stub for every script
/// and machine it names. `None` if it passes.
fn check_machine(yaml: &str) -> Option<String> {
    let machine: serde_norway::Value = serde_norway::from_str(yaml).unwrap();
    let name = machine["name"].as_str().unwrap().to_string();
    let mut needs = Needs::default();
    script_list(machine.get("onentry"), &mut needs);
    script_list(machine.get("onexit"), &mut needs);
    collect_needs(machine["states"].as_mapping().unwrap(), &mut needs);
    needs.machines.remove(&name);

    let tmp = TempDir::new().unwrap();
    let decree = tmp.path().join(".decree");
    fs::create_dir_all(decree.join("machines")).unwrap();
    fs::create_dir_all(decree.join("scripts")).unwrap();
    fs::write(decree.join(format!("machines/{name}.yml")), yaml).unwrap();
    needs.scripts.insert("docs_stub".to_string());
    for script in &needs.scripts {
        write_script(&decree.join("scripts").join(script), "#!/bin/sh\n");
    }
    for (child, (finals, data)) in &needs.machines {
        fs::write(
            decree.join(format!("machines/{child}.yml")),
            stub_machine(child, finals, data),
        )
        .unwrap();
    }
    let out = cargo_bin_cmd!("decree")
        .arg("check")
        .current_dir(tmp.path())
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    (!out.status.success() || !stdout.is_empty()).then(|| {
        format!(
            "{stdout}{}",
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .filter(|l| !l.starts_with("warning"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    })
}

/// A validator for one state: the machine schema's `state` definition.
fn state_validator() -> jsonschema::Validator {
    let machine: Json = serde_json::from_str(schema::MACHINE_SCHEMA).unwrap();
    let id = machine["$id"].as_str().unwrap();
    schema::validator(
        &serde_json::json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$ref": format!("{id}#/$defs/state"),
        })
        .to_string(),
    )
}

#[test]
fn every_documented_machine_passes_decree_check() {
    let states = state_validator();
    let mut errors = Vec::new();
    let (mut whole, mut fragments) = (0, 0);
    let mut skips_used = BTreeSet::new();
    for rel in documents() {
        for (line, block) in machine_blocks(&read(&rel)) {
            match block {
                MachineBlock::Whole(yaml) => {
                    whole += 1;
                    if let Some(out) = check_machine(&yaml) {
                        errors.push(format!("{rel}:{line}: decree check fails:\n{out}"));
                    }
                }
                MachineBlock::Fragment(map) => {
                    let first = map
                        .keys()
                        .next()
                        .unwrap()
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    if let Some(skip) = FRAGMENT_SKIPS
                        .iter()
                        .find(|(doc, id, _)| *doc == rel && *id == first)
                    {
                        skips_used.insert(skip.0.to_string() + skip.1);
                        continue;
                    }
                    fragments += 1;
                    for (id, state) in &map {
                        let json = serde_json::to_value(state).unwrap();
                        for e in schema::errors(&states, &json) {
                            errors.push(format!(
                                "{rel}:{line}: state `{}`: {e}",
                                id.as_str().unwrap_or("?")
                            ));
                        }
                    }
                }
            }
        }
    }
    for (doc, id, reason) in FRAGMENT_SKIPS {
        assert!(!reason.is_empty(), "{doc} {id}: give a reason");
        assert!(
            skips_used.contains(&(doc.to_string() + id)),
            "FRAGMENT_SKIPS lists {doc} `{id}`, which is no longer a fragment there"
        );
    }
    assert!(whole >= 10, "only {whole} machines found");
    assert!(fragments >= 2, "only {fragments} fragments found");
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn the_machine_stubs_catch_a_broken_machine() {
    let good = "name: m\ndescription: d.\ninitial: a\nstates:\n  a:\n    invoke: { machine: child }\n    transitions: { done: done, rejected: done }\n  done: { final: true }\n  failed: { final: true }\n";
    assert_eq!(check_machine(good), None);
    let broken = good.replace(
        "transitions: { done: done, rejected: done }",
        "transitions: { done: nowhere }",
    );
    let out = check_machine(&broken).expect("a target that does not exist fails");
    assert!(out.contains("V4"), "{out}");
}

// ---------------------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------------------

/// Every frontmatter example in a document: fenced blocks and heredoc bodies that start
/// with `---`, and in `help.txt` the indented ones.
fn frontmatters(rel: &str, text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    if rel.ends_with(".txt") {
        let lines: Vec<&str> = text.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            if lines[i] == "  ---" {
                if let Some(end) = lines[i + 1..].iter().position(|l| *l == "  ---") {
                    let body: Vec<&str> = lines[i..=i + 1 + end]
                        .iter()
                        .map(|l| l.strip_prefix("  ").unwrap_or(l))
                        .collect();
                    out.push((i + 1, body.join("\n") + "\n"));
                    i += end + 2;
                    continue;
                }
            }
            i += 1;
        }
        return out;
    }
    let (blocks, _) = markdown(text);
    for block in blocks {
        if block.text.starts_with("---") {
            out.push((block.line, block.text.clone()));
        }
        if is_shell(&block.lang) {
            for (offset, body) in heredocs(&block.text) {
                if body.starts_with("---") {
                    out.push((block.line + offset, body));
                }
            }
        }
    }
    out
}

#[test]
fn every_documented_message_validates() {
    let validator = schema::message_validator();
    let mut errors = Vec::new();
    let mut checked = 0;
    for rel in documents() {
        for (line, text) in frontmatters(&rel, &read(&rel)) {
            let Some(json) = schema::frontmatter_to_json(&text) else {
                if text.contains("machine:") || text.contains("to:") {
                    errors.push(format!("{rel}:{line}: frontmatter does not parse"));
                }
                continue;
            };
            let is_message = json.get("machine").is_some()
                || (json.get("to").is_some() && json.get("event").is_some());
            if !is_message {
                continue;
            }
            checked += 1;
            for e in schema::errors(&validator, &json) {
                errors.push(format!("{rel}:{line}: {e}"));
            }
        }
    }
    assert!(checked >= 12, "only {checked} messages found");
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

// ---------------------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------------------

/// The `type`s of `events.schema.json`, and its common required fields.
fn event_types() -> (BTreeSet<String>, Vec<String>) {
    let s: Json = serde_json::from_str(schema::EVENTS_SCHEMA).unwrap();
    let types = s["$defs"]
        .as_object()
        .unwrap()
        .values()
        .filter_map(|d| d["properties"]["type"]["const"].as_str())
        .map(str::to_string)
        .collect();
    let required = s["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    (types, required)
}

/// `schema` without any `required`: what a partial example is checked against.
fn without_required(schema: &mut Json) {
    match schema {
        Json::Object(map) => {
            map.remove("required");
            map.values_mut().for_each(without_required);
        }
        Json::Array(items) => items.iter_mut().for_each(without_required),
        _ => {}
    }
}

/// Every JSON object shown as an `events.jsonl` line in a document: a `json` block parsed
/// whole or line by line, or inline code, whose top-level `type` is an event type. The text
/// is returned with the parsed value, `None` if it is elided with `…`.
fn event_examples(text: &str, types: &BTreeSet<String>) -> Vec<(usize, String, Option<Json>)> {
    let (blocks, spans) = markdown(text);
    let mut candidates: Vec<(usize, String)> = Vec::new();
    for block in blocks.iter().filter(|b| b.lang.starts_with("json")) {
        if serde_json::from_str::<Json>(&block.text).is_ok() {
            candidates.push((block.line, block.text.clone()));
        } else {
            for (i, line) in block.text.lines().enumerate() {
                if line.trim_start().starts_with('{') {
                    candidates.push((block.line + i, line.to_string()));
                }
            }
        }
    }
    for span in spans.iter().filter(|s| s.text.starts_with('{')) {
        candidates.push((span.line, span.text.clone()));
    }
    let type_re =
        Regex::new(r#"^\{\s*(?:"[^"]*"\s*:\s*[^,]*,\s*)*"type"\s*:\s*"([a-z_]+)""#).unwrap();
    let mut out = Vec::new();
    for (line, text) in candidates {
        match serde_json::from_str::<Json>(&text) {
            Ok(json) => {
                if json
                    .get("type")
                    .and_then(Json::as_str)
                    .is_some_and(|t| types.contains(t))
                {
                    out.push((line, text, Some(json)));
                }
            }
            Err(_) => {
                let elided = text.contains('…');
                let typed = type_re
                    .captures(&text)
                    .is_some_and(|c| types.contains(&c[1]));
                if elided && (typed || text.contains("\"type\"")) {
                    out.push((line, text, None));
                }
            }
        }
    }
    out
}

#[test]
fn every_documented_event_validates() {
    let (types, common) = event_types();
    let full = schema::events_validator();
    let mut relaxed: Json = serde_json::from_str(schema::EVENTS_SCHEMA).unwrap();
    without_required(&mut relaxed);
    let partial = schema::validator(&relaxed.to_string());

    let mut errors = Vec::new();
    let mut complete_in_runs_md = 0;
    let mut elided_seen = BTreeSet::new();
    for rel in documents() {
        for (line, text, json) in event_examples(&read(&rel), &types) {
            let Some(json) = json else {
                match ELIDED_EVENTS
                    .iter()
                    .find(|(doc, part, _)| *doc == rel && text.contains(part))
                {
                    Some((doc, part, _)) => {
                        elided_seen.insert(format!("{doc}{part}"));
                    }
                    None => errors.push(format!(
                        "{rel}:{line}: an elided event example; list it in ELIDED_EVENTS"
                    )),
                }
                continue;
            };
            let complete = common.iter().all(|f| json.get(f).is_some());
            let validator = if complete { &full } else { &partial };
            if complete && rel == "docs/reference/runs.md" {
                complete_in_runs_md += 1;
            }
            for e in schema::errors(validator, &json) {
                errors.push(format!("{rel}:{line}: {e}"));
            }
        }
    }
    for (doc, part, reason) in ELIDED_EVENTS {
        assert!(!reason.is_empty(), "{doc}: give a reason");
        assert!(
            elided_seen.contains(&format!("{doc}{part}")),
            "ELIDED_EVENTS lists {doc} `{part}`, which is not there"
        );
    }
    assert!(
        complete_in_runs_md >= 1,
        "docs/reference/runs.md shows no complete events.jsonl line"
    );
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn a_wrong_event_example_fails() {
    let (types, _) = event_types();
    let doc = "```json\n{\"type\":\"interrupted\",\"cause\":\"power\",\"state\":\"a\"}\n```\n";
    let examples = event_examples(doc, &types);
    assert_eq!(examples.len(), 1);
    let mut relaxed: Json = serde_json::from_str(schema::EVENTS_SCHEMA).unwrap();
    without_required(&mut relaxed);
    let partial = schema::validator(&relaxed.to_string());
    assert!(!schema::errors(&partial, examples[0].2.as_ref().unwrap()).is_empty());
    let elided = "`{\"type\":\"script\", …}`\n";
    assert!(event_examples(elided, &types)[0].2.is_none());
}

// ---------------------------------------------------------------------------------------
// Environment
// ---------------------------------------------------------------------------------------

/// `DECREE_DATA_<NAME>` for any `DECREE_DATA_*`, else the name itself.
fn normalized(name: &str) -> String {
    if name == "DECREE_DATA" || name.starts_with("DECREE_DATA_") {
        "DECREE_DATA_<NAME>".to_string()
    } else {
        name.to_string()
    }
}

/// The variables in `docs/reference/scripts.md`'s Environment table.
fn documented_environment() -> BTreeSet<String> {
    let text = read("docs/reference/scripts.md");
    let table: Vec<&str> = text
        .lines()
        .skip_while(|l| *l != "## Environment")
        .skip_while(|l| !l.starts_with("| Variable"))
        .skip(2)
        .take_while(|l| l.starts_with('|'))
        .collect();
    assert!(!table.is_empty(), "no Environment table in scripts.md");
    table
        .iter()
        .map(|row| code_spans(row.split('|').nth(1).unwrap())[0].clone())
        .collect()
}

/// The variables `src/runtime.rs` sets for a script: the `DECREE_` names in its list, the
/// `DECREE_DATA_` ones, and the trace variables it pushes, by their constants' values.
fn runtime_environment() -> BTreeSet<String> {
    let runtime = read("src/runtime.rs");
    let start = runtime
        .find("let mut vars: Vec<(String, std::ffi::OsString)> = [")
        .expect("the variable list in runtime.rs");
    let end = start
        + runtime[start..]
            .find("format!(\"DECREE_{name}\")")
            .expect("the DECREE_ prefix in runtime.rs");
    let entry = Regex::new(r#"\(\s*"([A-Z_]+)","#).unwrap();
    let mut vars: BTreeSet<String> = entry
        .captures_iter(&runtime[start..end])
        .map(|c| format!("DECREE_{}", &c[1]))
        .collect();
    if runtime.contains("format!(\"DECREE_DATA_{}\"") {
        vars.insert("DECREE_DATA_<NAME>".to_string());
    }
    let pushed = Regex::new(r"vars\.push\(\(([A-Z_]+)\.to_string\(\)").unwrap();
    let mut sources = read("src/trace.rs");
    sources.push_str(&runtime);
    for c in pushed.captures_iter(&runtime) {
        let constant = Regex::new(&format!(r#"const {}: &str = "([A-Z_]+)";"#, &c[1])).unwrap();
        let value = constant
            .captures(&sources)
            .unwrap_or_else(|| panic!("const {} not found", &c[1]));
        vars.insert(value[1].to_string());
    }
    assert!(vars.len() > 20, "{vars:?}");
    vars
}

/// Every `DECREE_*` name in the documents, the example and template scripts, and the other
/// templates, with where it first appears.
fn named_environment() -> BTreeMap<String, String> {
    let root = repo();
    let mut files: Vec<String> = documents();
    let mut more = Vec::new();
    files_under(&root.join("src/templates"), &mut more);
    for entry in fs::read_dir(root.join("examples")).unwrap() {
        files_under(&entry.unwrap().path().join(".decree/scripts"), &mut more);
    }
    files.extend(
        more.iter()
            .map(|p| p.strip_prefix(&root).unwrap().display().to_string()),
    );
    let name = Regex::new(r"\bDECREE_[A-Z0-9_]*[A-Z0-9]").unwrap();
    let mut out = BTreeMap::new();
    for rel in files {
        let Ok(text) = fs::read_to_string(root.join(&rel)) else {
            continue; // not UTF-8, such as an image
        };
        for (i, line) in text.lines().enumerate() {
            for m in name.find_iter(line) {
                out.entry(normalized(m.as_str()))
                    .or_insert_with(|| format!("{rel}:{}", i + 1));
            }
        }
    }
    out
}

#[test]
fn the_environment_table_matches_what_decree_sets() {
    let table = documented_environment();
    let runtime = runtime_environment();
    let only_documented: Vec<_> = table.difference(&runtime).collect();
    let only_set: Vec<_> = runtime.difference(&table).collect();
    assert!(
        only_documented.is_empty(),
        "in scripts.md's Environment table but not set by src/runtime.rs: {only_documented:?}"
    );
    assert!(
        only_set.is_empty(),
        "set by src/runtime.rs but not in scripts.md's Environment table: {only_set:?}"
    );
    let unknown: Vec<String> = named_environment()
        .into_iter()
        .filter(|(name, _)| !table.contains(name))
        .map(|(name, at)| format!("{at}: {name}"))
        .collect();
    assert!(
        unknown.is_empty(),
        "not in scripts.md's Environment table:\n{}",
        unknown.join("\n")
    );
}

/// The skill starts with the simplest machine: it is the first rule, the old rule that put
/// every decision in a state is gone, and the worked example's first `develop` machine is
/// script states in a straight line, ending in `done`.
#[test]
fn the_skill_writes_the_simplest_machine_first() {
    let skill = read("src/templates/skills/decree/SKILL.md");
    let first_rule = skill
        .lines()
        .skip_while(|l| *l != "## Rules")
        .find(|l| l.starts_with("- "))
        .unwrap();
    assert!(
        first_rule.starts_with("- **Start with the simplest machine.**"),
        "{first_rule}"
    );
    assert!(!skill.contains("Machines decide, scripts work"));

    let develops: Vec<serde_norway::Value> = machine_blocks(&skill)
        .into_iter()
        .filter_map(|(_, b)| match b {
            MachineBlock::Whole(yaml) => serde_norway::from_str(&yaml).ok(),
            MachineBlock::Fragment(_) => None,
        })
        .filter(|m: &serde_norway::Value| m["name"].as_str() == Some("develop"))
        .collect();
    assert_eq!(develops.len(), 3, "the worked example grows in three steps");
    let first = &develops[0];
    let states = first["states"].as_mapping().unwrap();
    let mut at = first["initial"].as_str().unwrap().to_string();
    let mut seen = 0;
    while at != "done" {
        let state = &states[at.as_str()];
        assert!(state["invoke"].is_string(), "`{at}` invokes one script");
        let transitions = state["transitions"].as_mapping().unwrap();
        assert_eq!(transitions.len(), 1, "`{at}` has only `done`");
        at = transitions["done"].as_str().unwrap().to_string();
        seen += 1;
    }
    assert_eq!(
        seen + 2,
        states.len(),
        "only script states, `done` and `failed`"
    );
}

/// Rules learned running decree 0.5 (migration 97): shared files in `lib/`, variables in the
/// gitignored `.env` files (D57),
/// run folders kept, and waiting prep invoked with a timeout.
#[test]
fn the_skill_has_the_lib_env_runs_and_timeout_rules() {
    let skill = fs::read_to_string(repo().join("src/templates/skills/decree/SKILL.md")).unwrap();
    let skill = skill.split_whitespace().collect::<Vec<_>>().join(" ");
    for rule in [
        "Shared code, config and data go in `.decree/lib/`** (`$DECREE_LIB`); `scripts/` holds only what states invoke.",
        "Variables and secrets go in `.decree/.env` (every script) or a machine's `env_file: .env.<name>` (that machine's scripts only); both are gitignored,",
        "Never delete run folders to clean up**; they are the record. Use `decree prune --older-than <age>`.",
        "A step that waits for a shared resource is an invoked state with a `timeout`**",
    ] {
        assert!(skill.contains(rule), "the skill lacks: {rule}");
    }
}

/// Migration 101: the reference and the skill say that only `error` moves an attempt list on,
/// and their child-machine fragment for answering a `STOP` passes `decree check` as a whole
/// machine.
#[test]
fn the_escalate_fragment_is_a_machine_that_passes_decree_check() {
    for rel in [
        "docs/reference/machines.md",
        "src/templates/skills/decree/reference/machines.md",
    ] {
        let text = read(rel);
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            flat.contains("Only `error`") && flat.contains("ends the list at once"),
            "{rel} does not say that only `error` moves an attempt list on"
        );
        assert!(flat.contains("$DECREE_PARENT_RUN_DIR/STOP"), "{rel}");
        let fragments: Vec<serde_norway::Mapping> = machine_blocks(&text)
            .into_iter()
            .filter_map(|(_, b)| match b {
                MachineBlock::Fragment(map) if map.contains_key("escalate") => Some(map),
                _ => None,
            })
            .collect();
        assert_eq!(fragments.len(), 1, "{rel}: one escalate fragment");
        let mut states = fragments[0].clone();
        assert_eq!(
            states["implement"]["transitions"]["stop"].as_str(),
            Some("escalate")
        );
        for (id, state) in [
            ("gate", "{ invoke: gate, transitions: { done: done } }"),
            ("done", "{ final: true }"),
            ("failed", "{ final: true }"),
        ] {
            states.insert(id.into(), serde_norway::from_str(state).unwrap());
        }
        let machine = format!(
            "name: develop\ndescription: The escalate fragment.\ninitial: implement\nstates:\n{}",
            serde_norway::to_string(&states)
                .unwrap()
                .lines()
                .map(|l| format!("  {l}\n"))
                .collect::<String>()
        );
        assert_eq!(check_machine(&machine), None, "{rel}:\n{machine}");
    }
    let skill = read("src/templates/skills/decree/SKILL.md");
    let skill = skill.split_whitespace().collect::<Vec<_>>().join(" ");
    let line = "`attempts` retries after `error` only; to have a stronger model answer a weaker one's `STOP`, transition to a child machine.";
    assert!(skill.contains(line), "the skill lacks: {line}");
}
