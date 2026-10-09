//! `decree check`: validate machines (V1–V21), pending messages (M1–M3) and the `.env`
//! files (E1) before anything runs (docs/reference/machines.md, Validation). Prints one line per error:
//! `<path relative to .decree/>: <state path or line>: <message>`. Warns, on stderr and
//! without failing, when `.decree/graph/` differs from what `decree graph` would write, or
//! `.decree/schema/`, when it exists, from what `decree schema` would write, when `.decree/store/` holds
//! what no machine's `store:` declares, when an `.env` file uses a variable that is not set,
//! when a machine's `env_file` is missing, and when `.decree/.gitignore` does not list `.env*`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_norway::Value;

use crate::cli::CheckFormat;
use crate::commands::{graph, print_json, schema};
use crate::cron;
use crate::dotenv;
use crate::error::DecreeError;
use crate::layout::{self, DECREE_DIR};
use crate::machine::validate::CheckEnv;
use crate::machine::{self, LoadedMachine, MACHINES_DIR};
use crate::message::{validate, Message};

mod sarif;

pub fn run(project_root: &Path, format: CheckFormat) -> Result<(), DecreeError> {
    let project = Project::load(project_root)?;
    let problems = check(&project)?;
    let warnings = warnings(project_root, &project)?;
    match format {
        CheckFormat::Text => {
            for problem in &problems {
                println!("{problem}");
            }
            for w in &warnings {
                eprintln!("{}: {w}", colored::Colorize::yellow("warning"));
            }
        }
        CheckFormat::Json => print_json(&json_document(&problems, &warnings))?,
        CheckFormat::Sarif => print_json(&sarif::log(&problems, &warnings))?,
    }
    match problems.len() {
        0 => Ok(()),
        n => Err(DecreeError::Other(format!(
            "decree check found {n} error(s)"
        ))),
    }
}

/// One error `decree check` reports. As text it is one line,
/// `<file>: <line n | state path>: <message> (<rule>)` (docs/reference/machines.md,
/// Validation); the parts are kept apart for `--format json` and `sarif`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckError {
    /// `V1`–`V21`, `M1`–`M3` or `E1`; `None` for an error no rule names, such as bad YAML.
    pub(crate) rule: Option<String>,
    /// Path relative to `.decree/`.
    pub(crate) file: String,
    pub(crate) line: Option<usize>,
    /// Dotted state path, for an error inside a state.
    pub(crate) state: Option<String>,
    /// What is wrong, without the rule.
    pub(crate) message: String,
}

impl CheckError {
    /// An error in `file` at `at` (`line <n>` or a state path), whose message may end with
    /// its rule, `(V4)`.
    fn new(file: String, at: Option<&str>, message: &str) -> CheckError {
        let (message, rule) = split_rule(message);
        let line = at
            .and_then(|at| at.strip_prefix("line "))
            .and_then(|n| n.parse().ok());
        let state = at.filter(|_| line.is_none()).map(String::from);
        CheckError {
            rule,
            file,
            line,
            state,
            message,
        }
    }

    fn json(&self) -> serde_json::Value {
        let mut out = serde_json::json!({ "rule": self.rule, "file": self.file });
        if let Some(line) = self.line {
            out["line"] = line.into();
        }
        if let Some(state) = &self.state {
            out["state"] = state.as_str().into();
        }
        out["message"] = self.message.as_str().into();
        out
    }
}

impl std::fmt::Display for CheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: ", self.file)?;
        if let Some(line) = self.line {
            write!(f, "line {line}: ")?;
        }
        if let Some(state) = &self.state {
            write!(f, "{state}: ")?;
        }
        f.write_str(&self.message)?;
        if let Some(rule) = &self.rule {
            write!(f, " ({rule})")?;
        }
        Ok(())
    }
}

/// `message` without a trailing ` (V4)` or ` (M1)`, and that rule.
fn split_rule(message: &str) -> (String, Option<String>) {
    let rule = message
        .strip_suffix(')')
        .and_then(|m| m.rsplit_once(" ("))
        .filter(|(_, rule)| sarif::RULES.iter().any(|(id, _)| id == rule));
    match rule {
        Some((message, rule)) => (message.to_string(), Some(rule.to_string())),
        None => (message.to_string(), None),
    }
}

/// A file in `.decree/graph/` or `.decree/schema/` that differs from what `decree graph`
/// or `decree schema` would write, one in `.decree/store/` that no machine declares, or a
/// problem with an `.env` file that does not stop it being read.
/// `decree check` warns about it without failing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckWarning {
    /// Path relative to `.decree/`.
    pub(crate) file: String,
    pub(crate) message: String,
}

impl std::fmt::Display for CheckWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.file, self.message)
    }
}

/// Every warning: stale graphs, then stale schemas, then the store, then the `.env` files.
fn warnings(project_root: &Path, project: &Project) -> Result<Vec<CheckWarning>, DecreeError> {
    let mut out = Vec::new();
    // A machine that fails to load cannot be drawn; its error is reported as an error.
    if let Ok(stale) = graph::stale_files(project_root) {
        out.extend(stale.into_iter().map(|(file, message)| CheckWarning {
            file,
            message: format!("{message}; run `decree graph`"),
        }));
    }
    out.extend(
        schema::stale_files(project_root)?
            .into_iter()
            .map(|(file, message)| CheckWarning {
                file,
                message: format!("{message}; run `decree schema`"),
            }),
    );
    out.extend(store_warnings(&project_root.join(DECREE_DIR))?);
    out.extend(resolve_env(project)?.1);
    Ok(out)
}

/// What `.decree/store/` holds that no machine declares (docs/reference/machines.md, Store):
/// a file or folder in `store/<machine>/` that the machine's `store:` does not name, and
/// anything in `store/` that is not a machine's folder. A machine that fails to load is
/// reported as an error, and its folder is not looked into.
fn store_warnings(decree_dir: &Path) -> Result<Vec<CheckWarning>, DecreeError> {
    let store_dir = decree_dir.join(layout::STORE_DIR);
    let entries = match std::fs::read_dir(&store_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let paths = machine::machine_paths(decree_dir)?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry?;
        names.push((
            entry.file_name().to_string_lossy().into_owned(),
            entry.path(),
        ));
    }
    names.sort();
    let mut out = Vec::new();
    for (name, path) in names {
        let file = format!("{}/{name}", layout::STORE_DIR);
        let message = if !path.is_dir() {
            "is not a folder: `store/` holds one folder per machine".to_string()
        } else if !paths.contains_key(&name) {
            format!("is a store folder with no machine: no `{MACHINES_DIR}/{name}.yml`")
        } else {
            String::new()
        };
        if !message.is_empty() {
            out.push(CheckWarning { file, message });
            continue;
        }
        let Ok(m) = std::fs::read_to_string(&paths[&name])
            .map_err(DecreeError::from)
            .and_then(|text| machine::load_machine_text(&name, &text))
        else {
            continue;
        };
        let mut kept: Vec<String> = std::fs::read_dir(&path)?
            .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect::<Result<_, _>>()?;
        kept.sort();
        out.extend(
            kept.into_iter()
                .filter(|kept| !m.store.contains_key(kept))
                .map(|kept| CheckWarning {
                    file: format!("{file}/{kept}"),
                    message: format!("is not declared in {name}'s `store:`"),
                }),
        );
    }
    Ok(out)
}

/// The `--format json` document (`.decree/schema/v1/cli/check.schema.json`).
fn json_document(problems: &[CheckError], warnings: &[CheckWarning]) -> serde_json::Value {
    serde_json::json!({
        "valid": problems.is_empty(),
        "errors": problems.iter().map(CheckError::json).collect::<Vec<_>>(),
        "warnings": warnings
            .iter()
            .map(|w| serde_json::json!({ "file": w.file, "message": w.message }))
            .collect::<Vec<_>>(),
    })
}

/// Every error `decree check` reports, in order: machines by id, then pending migrations,
/// `inbox/` and `cron/`, each by filename, then the `.env` files by line.
fn check(project: &Project) -> Result<Vec<CheckError>, DecreeError> {
    let mut problems = project.problems.clone();
    let decree_dir = &project.decree_dir;
    for name in project.pending_migrations()? {
        project.check_file(layout::MIGRATIONS_DIR, &name, "M1", false, &mut problems)?;
    }
    for name in md_files(&decree_dir.join(layout::INBOX_DIR))? {
        project.check_file(layout::INBOX_DIR, &name, "M2", false, &mut problems)?;
    }
    for name in md_files(&decree_dir.join(layout::CRON_DIR))? {
        project.check_file(layout::CRON_DIR, &name, "M3", true, &mut problems)?;
    }
    problems.extend(env_problems(project)?);
    Ok(problems)
}

/// Each machine `env_file` with a valid name (V14), to the machines that name it.
fn machine_env_files(project: &Project) -> BTreeMap<&str, Vec<&str>> {
    let mut out: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (id, m) in &project.machines {
        if let Some(file) = m
            .env_file
            .as_deref()
            .filter(|f| machine::is_env_file_name(f))
        {
            out.entry(file).or_default().push(id);
        }
    }
    out
}

/// Every `.env` file decree reads that exists, parsed: `.env`, then the machines' files by
/// name.
fn env_files(project: &Project) -> Result<Vec<(String, ParsedEnv)>, DecreeError> {
    let mut out = Vec::new();
    for file in std::iter::once(layout::ENV_FILE).chain(machine_env_files(project).into_keys()) {
        if let Some(parsed) = dotenv::read(&project.decree_dir.join(file))? {
            out.push((file.to_string(), parsed));
        }
    }
    Ok(out)
}

type ParsedEnv = Result<Vec<dotenv::Entry>, dotenv::LineErrors>;

/// E1: every malformed line of every `.env` file decree reads, none if they are valid or
/// missing.
fn env_problems(project: &Project) -> Result<Vec<CheckError>, DecreeError> {
    let mut out = Vec::new();
    for (file, parsed) in env_files(project)? {
        for (line, message) in parsed.err().unwrap_or_default() {
            out.push(CheckError {
                rule: Some("E1".to_string()),
                file: file.clone(),
                line: Some(line),
                state: None,
                message,
            });
        }
    }
    Ok(out)
}

/// The variables scripts get from the `.env` files, and the warnings about them: each
/// variable a file uses that is set nowhere (it reads as empty, as in Compose), each
/// machine `env_file` that does not exist, and a `.decree/.gitignore` that does not list
/// `.env*` while one exists. A malformed file is left out; `env_problems` reports it.
fn resolve_env(project: &Project) -> Result<(dotenv::ProjectEnv, Vec<CheckWarning>), DecreeError> {
    let mut env = dotenv::ProjectEnv::default();
    let mut warnings = Vec::new();
    let mut by_file = BTreeMap::new();
    let files = env_files(project)?;
    for (file, parsed) in &files {
        let Ok(entries) = parsed else { continue };
        let resolved = if file == layout::ENV_FILE {
            let resolved = dotenv::resolve(entries, dotenv::process_var);
            env.base.clone_from(&resolved.vars);
            resolved
        } else {
            // A machine's file reads what its scripts would see without it.
            let base = &env.base;
            dotenv::resolve_over(entries, |name| {
                dotenv::process_var(name).or_else(|| {
                    base.iter()
                        .rev()
                        .find(|(key, _)| key == name)
                        .map(|(_, value)| value.clone())
                })
            })
        };
        warnings.extend(resolved.unset.iter().map(|(line, name)| CheckWarning {
            file: file.clone(),
            message: format!("line {line}: `${{{name}}}` is not set"),
        }));
        by_file.insert(file.as_str(), resolved.vars);
    }
    for (file, machines) in machine_env_files(project) {
        match by_file.get(file) {
            Some(vars) => {
                for id in machines {
                    env.machines.insert(id.to_string(), vars.clone());
                }
            }
            None if !project.decree_dir.join(file).exists() => warnings.push(CheckWarning {
                file: file.to_string(),
                message: format!(
                    "does not exist, so the scripts of {} get no variables from it; `{}` shows what goes in it, if the project has one",
                    machines
                        .iter()
                        .map(|id| format!("`{id}`"))
                        .collect::<Vec<_>>()
                        .join(", "),
                    layout::ENV_EXAMPLE_FILE
                ),
            }),
            None => {}
        }
    }
    if !files.is_empty() && !gitignores_env_files(&project.decree_dir) {
        warnings.push(CheckWarning {
            file: layout::GITIGNORE_FILE.to_string(),
            message: format!(
                "does not list `.env*`, so git may commit {}; add the lines `.env*` and `!{}`",
                files
                    .iter()
                    .map(|(file, _)| format!("`{file}`"))
                    .collect::<Vec<_>>()
                    .join(", "),
                layout::ENV_EXAMPLE_FILE
            ),
        });
    }
    Ok((env, warnings))
}

/// Whether `.decree/.gitignore` has a `.env*` line.
fn gitignores_env_files(decree_dir: &Path) -> bool {
    std::fs::read_to_string(decree_dir.join(layout::GITIGNORE_FILE))
        .is_ok_and(|text| text.lines().any(|l| matches!(l.trim(), ".env*" | "/.env*")))
}

/// The variables scripts get from the `.env` files, interpolated (docs/reference/scripts.md,
/// Environment). A malformed file is an error that lists each E1 problem, as `decree check`
/// prints them.
pub(crate) fn load_env(project: &Project) -> Result<dotenv::ProjectEnv, DecreeError> {
    let problems = env_problems(project)?;
    if !problems.is_empty() {
        let lines: Vec<String> = problems.iter().map(ToString::to_string).collect();
        let files: BTreeSet<String> = problems
            .iter()
            .map(|p| format!("{DECREE_DIR}/{}", p.file))
            .collect();
        return Err(DecreeError::Other(format!(
            "{}\n{} error(s) in {}; nothing was processed. Run `decree check`.",
            lines.join("\n"),
            problems.len(),
            files.into_iter().collect::<Vec<_>>().join(", ")
        )));
    }
    Ok(resolve_env(project)?.0)
}

/// A project's machines, loaded and checked against V1–V21: where `check` and
/// `process` start.
pub(crate) struct Project {
    pub(crate) decree_dir: PathBuf,
    /// Every machine file, including those that fail to load.
    pub(crate) machine_ids: BTreeSet<String>,
    /// The machines that loaded.
    pub(crate) machines: BTreeMap<String, LoadedMachine>,
    /// Every machine error, by machine id, each machine's errors in rule order.
    pub(crate) problems: Vec<CheckError>,
}

impl Project {
    pub(crate) fn load(project_root: &Path) -> Result<Project, DecreeError> {
        let decree_dir = project_root.join(DECREE_DIR);

        let paths = machine::machine_paths(&decree_dir)?;
        let machine_ids: BTreeSet<String> = paths.keys().cloned().collect();

        // Load every machine first: some rules look into the machines a state invokes.
        let mut problems = Vec::new();
        let mut machines = BTreeMap::new();
        let mut texts = BTreeMap::new();
        let file = |id: &str| format!("{MACHINES_DIR}/{id}.yml");
        for (id, path) in paths {
            let text = std::fs::read_to_string(&path)?;
            match machine::load_machine_located(&id, &text) {
                Ok(m) => {
                    machines.insert(id.clone(), m);
                    texts.insert(id, text);
                }
                Err(e) => {
                    let error = CheckError::new(file(&id), e.at.as_deref(), &e.message);
                    problems.push((id, error));
                }
            }
        }
        let env = CheckEnv {
            decree_dir: &decree_dir,
            machine_ids: &machine_ids,
            machines: &machines,
        };
        for (id, m) in &machines {
            for p in m.validate(&texts[id], &env) {
                problems.push((
                    id.clone(),
                    CheckError::new(file(id), Some(&p.at), &p.message),
                ));
            }
        }
        problems.sort_by(|a, b| a.0.cmp(&b.0));
        let problems = problems.into_iter().map(|(_, line)| line).collect();
        Ok(Project {
            decree_dir,
            machine_ids,
            machines,
            problems,
        })
    }

    /// `migrations/*.md` not in `processed.md`, in byte order (docs/reference/messages.md, Migrations).
    pub(crate) fn pending_migrations(&self) -> Result<Vec<String>, DecreeError> {
        let processed = read_processed(&self.decree_dir)?;
        Ok(md_files(&self.decree_dir.join(layout::MIGRATIONS_DIR))?
            .into_iter()
            .filter(|name| !processed.contains(name))
            .collect())
    }

    /// Check `.decree/<dir>/<name>` and append its errors, each tagged with `rule`.
    pub(crate) fn check_file(
        &self,
        dir: &str,
        name: &str,
        rule: &str,
        is_cron: bool,
        problems: &mut Vec<CheckError>,
    ) -> Result<(), DecreeError> {
        let rel = format!("{dir}/{name}");
        let bytes = std::fs::read(self.decree_dir.join(dir).join(name))?;
        let errors = match String::from_utf8(bytes) {
            Ok(text) => self.check_text(&text, is_cron),
            Err(_) => vec![(1, "file is not valid UTF-8".to_string())],
        };
        for (line, message) in errors {
            problems.push(CheckError {
                rule: Some(rule.to_string()),
                file: rel.clone(),
                line: Some(line),
                state: None,
                message,
            });
        }
        Ok(())
    }
}

/// `processed.md` as a set of filenames. A missing ledger is empty; `check` writes nothing.
pub(crate) fn read_processed(decree_dir: &Path) -> Result<BTreeSet<String>, DecreeError> {
    match std::fs::read_to_string(decree_dir.join(layout::PROCESSED_FILE)) {
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

/// `*.md` files in `dir` in byte order, skipping names that start with `.` (docs/reference/messages.md).
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

impl Project {
    /// Errors in one message as `(file line, message)`: it parses, names a known machine,
    /// and its `params` fit that machine's `data`. Cron files
    /// also need a `cron:` expression that parses.
    fn check_text(&self, text: &str, is_cron: bool) -> Vec<(usize, String)> {
        let fm = match Message::parse(text) {
            Ok(fm) => fm,
            Err(e) => return vec![e],
        };
        let mut errors = Vec::new();
        if is_cron {
            match fm.frontmatter.get("cron") {
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
        // A reply (`to:`) is delivered to a waiting run, not started as one (docs/reference/messages.md).
        if fm.frontmatter.contains_key("to") {
            return errors;
        }

        if let Err(e) = validate(&fm, &self.machines, &self.machine_ids) {
            errors.extend(e);
        }
        errors
    }
}
