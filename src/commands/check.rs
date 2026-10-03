//! `decree check`: validate machines (V1–V21) and pending messages (M1–M3) before anything
//! runs (spec section 5, Validation). Prints one line per error:
//! `<path relative to .decree/>: <state path or line>: <message>`. Warns, on stderr and
//! without failing, when `.decree/graph/` differs from what `decree graph` would write.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_norway::Value;

use crate::commands::graph;
use crate::config::{self, AppConfig};
use crate::cron;
use crate::error::DecreeError;
use crate::machine::{self, CheckEnv, LoadedMachine};
use crate::message::{validate, Message};

pub fn run(project_root: &Path) -> Result<(), DecreeError> {
    let problems = check(project_root)?;
    for problem in &problems {
        println!("{problem}");
    }
    // A machine that fails to load cannot be drawn; its error is reported above.
    if let Ok(stale) = graph::stale(project_root) {
        for line in stale {
            eprintln!(
                "{}: {line}; run `decree graph`",
                colored::Colorize::yellow("warning")
            );
        }
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
    let project = Project::load(project_root)?;
    let mut problems = project.problems.clone();
    let decree_dir = &project.decree_dir;
    for name in project.pending_migrations()? {
        project.check_file(config::MIGRATIONS_DIR, &name, "M1", false, &mut problems)?;
    }
    for name in md_files(&decree_dir.join(config::INBOX_DIR))? {
        project.check_file(config::INBOX_DIR, &name, "M2", false, &mut problems)?;
    }
    for name in md_files(&decree_dir.join(config::CRON_DIR))? {
        project.check_file(config::CRON_DIR, &name, "M3", true, &mut problems)?;
    }
    Ok(problems)
}

/// A project's config and machines, loaded and checked against V1–V21: where `check` and
/// `process` start.
pub(crate) struct Project {
    pub(crate) config: AppConfig,
    pub(crate) decree_dir: PathBuf,
    pub(crate) shared_source: Option<PathBuf>,
    /// Every machine file, including those that fail to load.
    pub(crate) machine_ids: BTreeSet<String>,
    /// The machines that loaded.
    pub(crate) machines: BTreeMap<String, LoadedMachine>,
    /// One line per machine error, by machine id, each machine's errors in rule order.
    pub(crate) problems: Vec<String>,
}

impl Project {
    pub(crate) fn load(project_root: &Path) -> Result<Project, DecreeError> {
        let config = AppConfig::load_from_project(project_root)?;
        let decree_dir = AppConfig::decree_dir(project_root);
        let shared_source = config.resolved_shared_source();

        let paths = machine::machine_paths(&decree_dir, shared_source.as_deref())?;
        let machine_ids: BTreeSet<String> = paths.keys().cloned().collect();

        // Load every machine first: some rules look into the machines a state invokes.
        let mut problems = Vec::new();
        let mut machines = BTreeMap::new();
        let mut texts = BTreeMap::new();
        for (id, path) in paths {
            let text = std::fs::read_to_string(&path)?;
            match machine::load_machine_text(&id, &text) {
                Ok(m) => {
                    machines.insert(id.clone(), m);
                    texts.insert(id, text);
                }
                Err(e) => problems.push((id, e.to_string())),
            }
        }
        let env = CheckEnv {
            decree_dir: &decree_dir,
            shared_source: shared_source.as_deref(),
            machine_ids: &machine_ids,
            machines: &machines,
            default_router: config.default_router.as_deref(),
        };
        for (id, m) in &machines {
            for p in m.validate(&texts[id], &env) {
                problems.push((
                    id.clone(),
                    format!("machines/{id}.yml: {}: {}", p.at, p.message),
                ));
            }
        }
        problems.sort_by(|a, b| a.0.cmp(&b.0));
        let problems = problems.into_iter().map(|(_, line)| line).collect();
        Ok(Project {
            config,
            decree_dir,
            shared_source,
            machine_ids,
            machines,
            problems,
        })
    }

    /// The machine for messages with no `machine:` key.
    pub(crate) fn default_machine(&self) -> Option<&str> {
        self.config.default_machine.as_deref()
    }

    /// `migrations/*.md` not in `processed.md`, in byte order (section 4, Migrations).
    pub(crate) fn pending_migrations(&self) -> Result<Vec<String>, DecreeError> {
        let processed = read_processed(&self.decree_dir)?;
        Ok(md_files(&self.decree_dir.join(config::MIGRATIONS_DIR))?
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
        problems: &mut Vec<String>,
    ) -> Result<(), DecreeError> {
        let rel = format!("{dir}/{name}");
        let bytes = std::fs::read(self.decree_dir.join(dir).join(name))?;
        let errors = match String::from_utf8(bytes) {
            Ok(text) => self.check_text(&text, is_cron),
            Err(_) => vec![(1, "file is not valid UTF-8".to_string())],
        };
        for (line, msg) in errors {
            problems.push(format!("{rel}: line {line}: {msg} ({rule})"));
        }
        Ok(())
    }
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

impl Project {
    /// Errors in one message as `(file line, message)`: it parses, names a known machine (or
    /// one is configured by default), and its `params` fit that machine's `data`. Cron files
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
        // A reply (`to:`) is delivered to a waiting run, not started as one (section 4).
        if fm.frontmatter.contains_key("to") {
            return errors;
        }

        if let Err(e) = validate(
            &fm,
            &self.machines,
            &self.machine_ids,
            self.default_machine(),
        ) {
            errors.extend(e);
        }
        errors
    }
}
