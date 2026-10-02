//! `decree check`: validate machines (V1–V14) and pending messages (M1–M3) before anything
//! runs (spec section 5, Validation). Prints one line per error:
//! `<path relative to .decree/>: <state path or line>: <message>`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_norway::{Mapping, Value};

use crate::config::{self, AppConfig};
use crate::cron;
use crate::error::DecreeError;
use crate::machine::{self, CheckEnv, LoadedMachine};

pub fn run(project_root: &Path) -> Result<(), DecreeError> {
    let problems = check(project_root)?;
    for problem in &problems {
        println!("{problem}");
    }
    match problems.len() {
        0 => Ok(()),
        n => Err(DecreeError::Other(format!(
            "decree check found {n} error(s)"
        ))),
    }
}

/// Every error `decree check` reports, in order: machines by id, then pending migrations,
/// `inbox/` and `cron/`, each by filename.
pub fn check(project_root: &Path) -> Result<Vec<String>, DecreeError> {
    // 0.4 config: `default_routine` and `routine_source` are the section 3
    // `default_machine` and `shared_source` until the config is rewritten.
    let config = AppConfig::load_from_project(project_root)?;
    let decree_dir = AppConfig::decree_dir(project_root);
    let shared_source = config.resolved_routine_source();

    let paths = machine::machine_paths(&decree_dir, shared_source.as_deref())?;
    let machine_ids: BTreeSet<String> = paths.keys().cloned().collect();
    let env = CheckEnv {
        decree_dir: &decree_dir,
        shared_source: shared_source.as_deref(),
        machine_ids: &machine_ids,
    };

    let mut problems = Vec::new();
    let mut machines = BTreeMap::new();
    for (id, path) in paths {
        let text = std::fs::read_to_string(&path)?;
        match machine::load_machine_text(&id, &path, &text) {
            Ok(m) => {
                for p in m.validate(&text, &env) {
                    problems.push(format!("machines/{id}.yml: {}: {}", p.at, p.message));
                }
                machines.insert(id, m);
            }
            Err(e) => problems.push(e.to_string()),
        }
    }

    let messages = Messages {
        machines: &machines,
        machine_ids: &machine_ids,
        default_machine: Some(config.default_routine.as_str()),
    };
    let processed = read_processed(&decree_dir)?;
    for name in md_files(&decree_dir.join(config::MIGRATIONS_DIR))? {
        if !processed.contains(&name) {
            messages.check_file(
                &decree_dir,
                config::MIGRATIONS_DIR,
                &name,
                "M1",
                false,
                &mut problems,
            )?;
        }
    }
    for name in md_files(&decree_dir.join(config::INBOX_DIR))? {
        messages.check_file(
            &decree_dir,
            config::INBOX_DIR,
            &name,
            "M2",
            false,
            &mut problems,
        )?;
    }
    for name in md_files(&decree_dir.join(config::CRON_DIR))? {
        messages.check_file(
            &decree_dir,
            config::CRON_DIR,
            &name,
            "M3",
            true,
            &mut problems,
        )?;
    }
    Ok(problems)
}

/// `processed.md` as a set of filenames. A missing ledger is empty; `check` writes nothing.
fn read_processed(decree_dir: &Path) -> Result<BTreeSet<String>, DecreeError> {
    match std::fs::read_to_string(decree_dir.join(config::PROCESSED_FILE)) {
        Ok(text) => Ok(text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        Err(e) => Err(e.into()),
    }
}

/// `*.md` files in `dir` in byte order, skipping names that start with `.` (section 4).
pub(crate) fn md_files(dir: &Path) -> Result<Vec<String>, DecreeError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if name.ends_with(".md") && !name.starts_with('.') && entry.path().is_file() {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// A message's frontmatter (section 4, Parsing and writing), with the file line of each
/// top-level key for error locations.
#[derive(Debug, Default)]
pub(crate) struct Frontmatter {
    pub(crate) map: Mapping,
    key_lines: BTreeMap<String, usize>,
}

impl Frontmatter {
    fn line_of(&self, key: &str) -> usize {
        self.key_lines.get(key).copied().unwrap_or(1)
    }
}

/// Parse frontmatter per section 4: an optional UTF-8 BOM, `\n` or `\r\n` line ends, fences
/// that are `---` once trailing whitespace is removed, and a YAML mapping without duplicate
/// keys between them. No opening fence means an empty map. Errors carry the file line.
pub(crate) fn parse_frontmatter(text: &str) -> Result<Frontmatter, (usize, String)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines: Vec<&str> = text
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    let is_fence = |l: &str| l.trim_end() == "---";
    if !lines.first().is_some_and(|l| is_fence(l)) {
        return Ok(Frontmatter::default());
    }
    let Some(close) = lines
        .iter()
        .skip(1)
        .position(|l| is_fence(l))
        .map(|i| i + 1)
    else {
        return Err((
            1,
            "frontmatter has an opening `---` but no closing `---`".into(),
        ));
    };
    let yaml_lines = &lines[1..close];
    let yaml = yaml_lines.join("\n");
    // YAML line `n` is file line `n + 1`: the opening fence is line 1.
    let value: Value = serde_norway::from_str(&yaml).map_err(|e| {
        let msg = strip_yaml_location(&e.to_string());
        let line = duplicate_key_line(yaml_lines, &msg)
            .or_else(|| e.location().map(|l| l.line() + 1))
            .unwrap_or(1);
        (line, msg)
    })?;
    let map = match value {
        Value::Null => Mapping::new(),
        Value::Mapping(map) => map,
        _ => return Err((2, "frontmatter is not a YAML mapping".into())),
    };
    let mut key_lines = BTreeMap::new();
    for (i, line) in yaml_lines.iter().enumerate() {
        if let Some((key, _)) = line.split_once(':') {
            if !key.is_empty() && !key.starts_with([' ', '\t', '#', '-']) {
                key_lines.entry(key.trim().to_string()).or_insert(i + 2);
            }
        }
    }
    Ok(Frontmatter { map, key_lines })
}

/// serde_norway reports a duplicate key at the start of its mapping; find the file line of
/// the second top-level occurrence instead.
fn duplicate_key_line(yaml_lines: &[&str], msg: &str) -> Option<usize> {
    let key = msg
        .strip_prefix("duplicate entry with key \"")?
        .strip_suffix('"')?;
    yaml_lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.strip_prefix(key).is_some_and(|r| r.starts_with(':')))
        .nth(1)
        .map(|(i, _)| i + 2)
}

/// serde_norway messages end with `at line N column M`, a line inside the YAML; the caller
/// reports the file line instead.
fn strip_yaml_location(msg: &str) -> String {
    match msg.find(" at line ") {
        Some(i) => msg[..i].to_string(),
        None => msg.to_string(),
    }
}

struct Messages<'a> {
    machines: &'a BTreeMap<String, LoadedMachine>,
    machine_ids: &'a BTreeSet<String>,
    default_machine: Option<&'a str>,
}

impl Messages<'_> {
    /// Check `<decree_dir>/<dir>/<name>` and append its errors, each tagged with `rule`.
    fn check_file(
        &self,
        decree_dir: &Path,
        dir: &str,
        name: &str,
        rule: &str,
        is_cron: bool,
        problems: &mut Vec<String>,
    ) -> Result<(), DecreeError> {
        let rel = format!("{dir}/{name}");
        let bytes = std::fs::read(decree_dir.join(dir).join(name))?;
        let errors = match String::from_utf8(bytes) {
            Ok(text) => self.check_text(&text, is_cron),
            Err(_) => vec![(1, "file is not valid UTF-8".to_string())],
        };
        for (line, msg) in errors {
            problems.push(format!("{rel}: line {line}: {msg} ({rule})"));
        }
        Ok(())
    }

    /// Errors in one message as `(file line, message)`: it parses, names a known machine (or
    /// one is configured by default), and its `params` fit that machine's `data`. Cron files
    /// also need a `cron:` expression that parses.
    fn check_text(&self, text: &str, is_cron: bool) -> Vec<(usize, String)> {
        let fm = match parse_frontmatter(text) {
            Ok(fm) => fm,
            Err(e) => return vec![e],
        };
        let mut errors = Vec::new();
        if is_cron {
            match fm.map.get("cron") {
                None => errors.push((1, "no `cron:` expression".to_string())),
                Some(Value::String(expr)) => {
                    if let Err(e) = cron::parse_schedule(expr) {
                        errors.push((
                            fm.line_of("cron"),
                            format!("cron `{expr}` does not parse: {e}"),
                        ));
                    }
                }
                Some(_) => errors.push((fm.line_of("cron"), "`cron` must be a string".to_string())),
            }
        }
        // A reply (`to:`) is delivered to a waiting run, not started as one (section 4).
        if fm.map.contains_key("to") {
            return errors;
        }

        let machine = match (fm.map.get("machine"), fm.map.get("routine")) {
            (Some(_), Some(_)) => {
                errors.push((
                    fm.line_of("routine"),
                    "both `machine` and its alias `routine` are set".to_string(),
                ));
                return errors;
            }
            (Some(v), None) => Some(("machine", v)),
            (None, Some(v)) => Some(("routine", v)),
            (None, None) => None,
        };
        let (name, line) = match machine {
            Some((key, Value::String(name))) => (name.as_str(), fm.line_of(key)),
            Some((key, _)) => {
                errors.push((fm.line_of(key), format!("`{key}` must be a string")));
                return errors;
            }
            None => match self.default_machine {
                Some(name) => (name, 1),
                None => {
                    errors.push((
                        1,
                        "no `machine` key and no default machine is configured".to_string(),
                    ));
                    return errors;
                }
            },
        };
        if !self.machine_ids.contains(name) {
            let msg = match machine {
                Some(_) => format!("unknown machine `{name}`"),
                None => {
                    format!("no `machine` key, and the default machine `{name}` does not exist")
                }
            };
            errors.push((line, msg));
            return errors;
        }
        // A machine that fails to load is reported on its own; its `data` is unknown.
        if let Some(m) = self.machines.get(name) {
            errors.extend(check_params(&fm, m));
        }
        errors
    }
}

/// `params` must be a map from a name in the machine's `data` to a value of its type.
fn check_params(fm: &Frontmatter, m: &LoadedMachine) -> Vec<(usize, String)> {
    let line = fm.line_of("params");
    let params = match fm.map.get("params") {
        None => return Vec::new(),
        Some(Value::Mapping(params)) => params,
        Some(_) => return vec![(line, "`params` must be a mapping".to_string())],
    };
    let mut errors = Vec::new();
    for (key, value) in params {
        let Some(key) = key.as_str() else {
            errors.push((line, "`params` keys must be strings".to_string()));
            continue;
        };
        match m.data.get(key) {
            None => errors.push((
                line,
                format!(
                    "unknown param `{key}`: machine `{}` has no data `{key}`",
                    m.id
                ),
            )),
            Some(spec) if !spec.kind.matches(value) => errors.push((
                line,
                format!("param `{key}` must be of type `{}`", spec.kind.as_str()),
            )),
            Some(_) => {}
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(text: &str) -> (usize, String) {
        parse_frontmatter(text).unwrap_err()
    }

    #[test]
    fn no_fence_is_empty_map() {
        let fm = parse_frontmatter("# Title\nbody\n").unwrap();
        assert!(fm.map.is_empty());
    }

    #[test]
    fn bom_crlf_and_trailing_spaces_on_fences_parse() {
        let fm = parse_frontmatter(
            "\u{feff}---  \r\nmachine: hello\r\nparams:\n  x: 1\n--- \r\nbody\r\n",
        )
        .unwrap();
        assert_eq!(fm.map.get("machine").and_then(Value::as_str), Some("hello"));
        assert_eq!(fm.line_of("machine"), 2);
        assert_eq!(fm.line_of("params"), 3);
    }

    #[test]
    fn empty_frontmatter_is_empty_map() {
        assert!(parse_frontmatter("---\n---\nbody").unwrap().map.is_empty());
    }

    #[test]
    fn unclosed_fence_is_an_error() {
        let (line, msg) = err("---\nmachine: hello\nbody\n");
        assert_eq!(line, 1);
        assert!(msg.contains("no closing"), "{msg}");
    }

    #[test]
    fn duplicate_key_names_the_file_line() {
        let (line, msg) = err("---\nmachine: a\nid: x\nmachine: b\n---\n");
        assert_eq!(line, 4, "{msg}");
        assert!(msg.contains("duplicate"), "{msg}");
        assert!(!msg.contains("at line"), "{msg}");
    }

    #[test]
    fn non_mapping_is_an_error() {
        let (_, msg) = err("---\n- a\n- b\n---\n");
        assert!(msg.contains("not a YAML mapping"), "{msg}");
    }
}
