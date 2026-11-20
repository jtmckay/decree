//! `decree check`: validate machines (V1–V21), queued messages and cron files (M2, M3) and the `.env`
//! files (E1) before anything runs (docs/reference/machines.md, Validation). Prints one line per error:
//! `<path relative to .decree/>: <state path or line>: <message>`. Warns, on stderr and
//! without failing, when `.decree/graph/` differs from what `decree graph` would write, or
//! `.decree/schema/`, when it exists, from what `decree schema` would write, when a machine's
//! `state/` holds what its `state_files:` does not declare, when an `.env` file uses a variable
//! that is not set, when a machine's `env_file` is missing, and when `.decree/.gitignore` does
//! not list `.env*`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_norway::Value;

use crate::cli::CheckFormat;
use crate::commands::{graph, print_json, schema};
use crate::cron;
use crate::dotenv;
use crate::error::DecreeError;
use crate::layout::{self, DECREE_DIR};
use crate::machine::validate::CheckEnv;
use crate::machine::{self, LoadedMachine};
use crate::message::{self, validate, Message};
use crate::plugin;

mod sarif;

pub fn run(project_root: &Path, format: CheckFormat) -> Result<(), DecreeError> {
    let synced = plugin::status(&project_root.join(DECREE_DIR))?;
    let project = Project::load(project_root)?;
    let mut problems = plugin_problems(&synced);
    problems.extend(check(&project)?);
    let mut warnings = warnings(project_root, &project)?;
    warnings.extend(pending_plugins(&synced));
    warnings.extend(plugin_warnings(&project)?);
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
    /// `V1`–`V21`, `M2`, `M3` or `E1`; `None` for an error no rule names, such as bad YAML.
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

/// `message` without a trailing ` (V4)` or ` (M2)`, and that rule.
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

/// A graph file or a file in `.decree/schema/` that differs from what `decree graph` or
/// `decree schema` would write, one in a machine's `state/` that it does not declare, or a
/// problem with an `.env` file that does not stop it being read. `decree check` warns about it without failing.
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

/// P1–P3 (docs/reference/plugins.md, Checks): each plugin file that fails the schema, each
/// `needs` that is not in `plugins/`, and each plugin not installed because its installed
/// copy was edited, by plugin name.
fn plugin_problems(synced: &plugin::Synced) -> Vec<CheckError> {
    let mut out: Vec<(String, CheckError)> = Vec::new();
    for (name, problems) in &synced.invalid {
        for p in problems {
            out.push((
                name.clone(),
                CheckError {
                    rule: Some("P1".to_string()),
                    file: plugin::plugin_file(name),
                    line: p.line,
                    state: None,
                    message: p.message.clone(),
                },
            ));
        }
    }
    for (name, missing) in &synced.missing_needs {
        let list = missing
            .iter()
            .map(|n| format!("`{n}`"))
            .collect::<Vec<_>>()
            .join(", ");
        out.push((
            name.clone(),
            CheckError {
                rule: Some("P2".to_string()),
                file: plugin::plugin_file(name),
                line: None,
                state: None,
                message: format!(
                    "`needs` names {list}, not in `{}/`: import it with `decree plugin import`",
                    plugin::PLUGINS_DIR
                ),
            },
        ));
    }
    for name in &synced.conflicts {
        out.push((
            name.clone(),
            CheckError {
                rule: Some("P3".to_string()),
                file: plugin::plugin_file(name),
                line: None,
                state: None,
                message: plugin::conflict_message(name),
            },
        ));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.into_iter().map(|(_, e)| e).collect()
}

/// Each machine no plugin installed, in a project with `plugins/`.
/// A warning for each plugin `decree plugin install` would install, remove, or leave as a
/// machine no plugin installed: `check` installs nothing (docs/reference/plugins.md,
/// Installing).
fn pending_plugins(synced: &plugin::Synced) -> Vec<CheckWarning> {
    let mut out = Vec::new();
    for name in &synced.installed {
        out.push(CheckWarning {
            file: plugin::plugin_file(name),
            message:
                "not installed, or changed since it was installed: run `decree plugin install`"
                    .to_string(),
        });
    }
    for name in &synced.removed {
        out.push(CheckWarning {
            file: plugin::plugin_file(name),
            message: "deleted, but its installed copy is still there: run `decree plugin install` to remove it".to_string(),
        });
    }
    for name in &synced.kept {
        out.push(CheckWarning {
            file: plugin::plugin_file(name),
            message: "deleted, but its installed copy was edited: `decree plugin install` keeps it as files no plugin installed".to_string(),
        });
    }
    out
}

fn plugin_warnings(project: &Project) -> Result<Vec<CheckWarning>, DecreeError> {
    Ok(
        plugin::unplugged(&project.decree_dir, &project.machine_ids)?
            .into_iter()
            .map(|id| CheckWarning {
                file: layout::machine_file_rel(&id),
                message: format!(
                "no plugin installed this machine; run `decree plugin export {id}` to write `{}`",
                plugin::plugin_file(&id)
            ),
            })
            .collect(),
    )
}

/// Every warning: stale graphs, then stale schemas, then state files, then the `.env` files.
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
    out.extend(state_warnings(&project_root.join(DECREE_DIR))?);
    out.extend(resolve_env(project)?.1);
    Ok(out)
}

/// What a machine's `state/` holds that its `state_files:` does not declare
/// (docs/reference/machines.md, State): one warning per file or folder directly in
/// `machines/<name>/state/`. A machine that fails to load is reported as an error, and its
/// folder is not looked into.
/// What the official `order` plugin writes in each machine's `state/`: the order files that
/// ran, with their hashes (plugins/order.md).
const ORDER_RECORD: &str = "order.git.txt";

fn state_warnings(decree_dir: &Path) -> Result<Vec<CheckWarning>, DecreeError> {
    let mut out = Vec::new();
    for (name, path) in machine::machine_paths(decree_dir)? {
        let dir = layout::machine_dir(decree_dir, &name).join(layout::STATE_DIR);
        if !dir.is_dir() {
            continue;
        }
        let Ok(m) = std::fs::read_to_string(&path)
            .map_err(DecreeError::from)
            .and_then(|text| machine::load_machine_text(&name, &text))
        else {
            continue;
        };
        let mut kept: Vec<String> = std::fs::read_dir(&dir)?
            .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect::<Result<_, _>>()?;
        kept.sort();
        let file = format!("{}/{}", layout::machine_dir_rel(&name), layout::STATE_DIR);
        out.extend(
            kept.into_iter()
                // `order.git.txt` is the `order` plugin's record of the machine's `order/`.
                .filter(|kept| kept != ORDER_RECORD && !m.state_files.contains_key(kept))
                .map(|kept| CheckWarning {
                    file: format!("{file}/{kept}"),
                    message: format!("is not declared in {name}'s `state_files:`"),
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

/// Every error `decree check` reports, in order: machines by id, then `inbox/` and the cron
/// files, each by filename, then the `.env` files by line.
fn check(project: &Project) -> Result<Vec<CheckError>, DecreeError> {
    let mut problems = project.problems.clone();
    let decree_dir = &project.decree_dir;
    for name in md_files(&decree_dir.join(layout::INBOX_DIR))? {
        project.check_file(layout::INBOX_DIR, &name, "M2", None, &mut problems)?;
    }
    for (owner, file) in crate::cron::file_names(decree_dir)? {
        let dir = format!("{}/{}", layout::machine_dir_rel(&owner), layout::CRON_DIR);
        project.check_file(&dir, &file, "M3", Some(&owner), &mut problems)?;
    }
    let env_problems = env_problems(project)?;
    if env_problems.is_empty() {
        let (env, _) = resolve_env(project)?;
        if let Err(bad) = message::max_depth(dotenv::process_var, &env.decree) {
            problems.push(max_depth_problem(project, bad)?);
        }
    }
    problems.extend(env_problems);
    Ok(problems)
}

/// E1 for a `DECREE_MAX_DEPTH` in `.decree/.env` that is not a whole number of at least 1,
/// at its line. One from decree's own environment is in no file: it is the error itself.
fn max_depth_problem(
    project: &Project,
    bad: message::BadMaxDepth,
) -> Result<CheckError, DecreeError> {
    if !bad.in_dotenv {
        return Err(DecreeError::Other(bad.to_string()));
    }
    let line = match dotenv::read(&project.decree_dir.join(layout::ENV_FILE))? {
        Some(Ok(entries)) => entries
            .iter()
            .rev()
            .find(|e| e.key == message::MAX_DEPTH_VAR)
            .map(|e| e.line),
        _ => None,
    };
    Ok(CheckError {
        rule: Some("E1".to_string()),
        file: layout::ENV_FILE.to_string(),
        line,
        state: None,
        message: format!(
            "`{}` is `{}`: it must be a whole number of at least 1",
            message::MAX_DEPTH_VAR,
            bad.value
        ),
    })
}

/// Each machine `env_file` with a valid name (V14), to the machines that name it.
fn machine_env_files(project: &Project) -> BTreeMap<&str, Vec<&str>> {
    let mut out: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (id, m) in project.machines.iter() {
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
        // decree reads its own settings from `.decree/.env` only.
        let own = file != layout::ENV_FILE;
        for entry in parsed.as_ref().ok().into_iter().flatten() {
            if own && dotenv::is_setting(&entry.key) {
                out.push(CheckError {
                    rule: Some("E1".to_string()),
                    file: file.clone(),
                    line: Some(entry.line),
                    state: None,
                    message: format!(
                        "key `{}` is reserved: only `.decree/.env` sets it",
                        entry.key
                    ),
                });
            }
        }
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
            (env.decree, env.base) = resolved
                .vars
                .iter()
                .cloned()
                .partition(|(key, _)| dotenv::is_setting(key));
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
    let mut env = resolve_env(project)?.0;
    env.max_depth = message::max_depth(dotenv::process_var, &env.decree)
        .map_err(|bad| DecreeError::Other(bad.to_string()))?;
    Ok(env)
}

/// `max_depth` for `decree emit`: `DECREE_MAX_DEPTH` from decree's own environment, which a
/// script in a run gets, else `.decree/.env` (`load_env`), else 10.
pub(crate) fn max_depth(project: &Project) -> Result<u32, DecreeError> {
    let in_shell = dotenv::process_var(message::MAX_DEPTH_VAR).is_some_and(|v| !v.is_empty());
    if in_shell {
        message::max_depth(dotenv::process_var, &[])
            .map_err(|bad| DecreeError::Other(bad.to_string()))
    } else {
        Ok(load_env(project)?.max_depth)
    }
}

/// A project's machines, loaded and checked against V1–V21: where `check` and
/// `process` start.
pub(crate) struct Project {
    pub(crate) decree_dir: PathBuf,
    /// Every machine file, including those that fail to load.
    pub(crate) machine_ids: BTreeSet<String>,
    /// The machines that loaded.
    pub(crate) machines: Arc<BTreeMap<String, LoadedMachine>>,
    /// Every machine error, by machine id, each machine's errors in rule order.
    pub(crate) problems: Vec<CheckError>,
    /// Every machine file's text, by machine id: what a reload compares.
    pub(crate) sources: BTreeMap<String, String>,
}

impl Project {
    pub(crate) fn load(project_root: &Path) -> Result<Project, DecreeError> {
        let decree_dir = project_root.join(DECREE_DIR);

        let paths = machine::machine_paths(&decree_dir)?;
        let machine_ids: BTreeSet<String> = paths.keys().cloned().collect();

        // Load every machine first: some rules look into the machines a state invokes.
        let mut problems = Vec::new();
        let mut machines = BTreeMap::new();
        let mut sources = BTreeMap::new();
        let file = |id: &str| layout::machine_file_rel(id);
        for (id, path) in paths {
            let text = std::fs::read_to_string(&path)?;
            match machine::load_machine_located(&id, &text) {
                Ok(m) => {
                    machines.insert(id.clone(), m);
                }
                Err(e) => {
                    let error = CheckError::new(file(&id), e.at.as_deref(), &e.message);
                    problems.push((id.clone(), error));
                }
            }
            sources.insert(id, text);
        }
        let env = CheckEnv {
            decree_dir: &decree_dir,
            machine_ids: &machine_ids,
            machines: &machines,
        };
        for (id, m) in &machines {
            for p in m.validate(&sources[id], &env) {
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
            machines: Arc::new(machines),
            problems,
            sources,
        })
    }

    /// Check `.decree/<dir>/<name>` and append its errors, each tagged with `rule`.
    pub(crate) fn check_file(
        &self,
        dir: &str,
        name: &str,
        rule: &str,
        cron: Option<&str>,
        problems: &mut Vec<CheckError>,
    ) -> Result<(), DecreeError> {
        let rel = format!("{dir}/{name}");
        let bytes = std::fs::read(self.decree_dir.join(dir).join(name))?;
        let errors = match String::from_utf8(bytes) {
            Ok(text) => self.check_text(&text, cron),
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
    /// Errors in a message's text. `cron`, for a cron file, is the machine whose folder holds
    /// it, its `machine` when it names none.
    fn check_text(&self, text: &str, cron: Option<&str>) -> Vec<(usize, String)> {
        let mut fm = match Message::parse(text) {
            Ok(fm) => fm,
            Err(e) => return vec![e],
        };
        let mut errors = Vec::new();
        if let Some(owner) = cron {
            cron::default_machine(&mut fm, owner);
            match fm.frontmatter.get("cron") {
                None => errors.push((1, "no `cron:` expression".to_string())),
                Some(Value::String(expr)) => {
                    if let Err(e) = cron::parse_when(expr) {
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
