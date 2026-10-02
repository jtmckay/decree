//! `decree graph`: write `.decree/graph/<machine>.md` for every machine and
//! `.decree/graph/system.md`, each a Markdown document holding a Mermaid diagram, and remove
//! stale `.md` files there (spec section 9). `decree check` uses `stale` to warn when the
//! committed files are out of date.

use std::collections::BTreeMap;
use std::path::Path;

use serde_norway::Value;

use crate::commands::check::{md_files, parse_frontmatter};
use crate::config::{self, AppConfig, DECREE_DIR};
use crate::error::DecreeError;
use crate::graph;
use crate::machine;

/// Directory under `.decree/` that `decree graph` writes.
pub const GRAPH_DIR: &str = "graph";

pub fn run(project_root: &Path) -> Result<(), DecreeError> {
    for name in write(project_root)? {
        println!("{DECREE_DIR}/{GRAPH_DIR}/{name}");
    }
    Ok(())
}

/// Write every document into `.decree/graph/` and remove the `.md` files there that no
/// machine produces. Returns the filenames written, in name order.
pub fn write(project_root: &Path) -> Result<Vec<String>, DecreeError> {
    let documents = render(project_root)?;
    let dir = AppConfig::decree_dir(project_root).join(GRAPH_DIR);
    std::fs::create_dir_all(&dir)?;
    for name in md_files(&dir)? {
        if !documents.contains_key(&name) {
            std::fs::remove_file(dir.join(&name))?;
        }
    }
    for (name, text) in &documents {
        std::fs::write(dir.join(name), text)?;
    }
    Ok(documents.into_keys().collect())
}

/// How `.decree/graph/` differs from what `decree graph` would write, one line per file,
/// each naming the file relative to `.decree/`. Empty when it is up to date.
pub fn stale(project_root: &Path) -> Result<Vec<String>, DecreeError> {
    let documents = render(project_root)?;
    let dir = AppConfig::decree_dir(project_root).join(GRAPH_DIR);
    let mut out = Vec::new();
    for (name, text) in &documents {
        match std::fs::read_to_string(dir.join(name)) {
            Ok(on_disk) if on_disk == *text => {}
            Ok(_) => out.push(format!("{GRAPH_DIR}/{name}: out of date")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                out.push(format!("{GRAPH_DIR}/{name}: missing"))
            }
            Err(e) => return Err(e.into()),
        }
    }
    for name in md_files(&dir)? {
        if !documents.contains_key(&name) {
            out.push(format!("{GRAPH_DIR}/{name}: no machine draws it"));
        }
    }
    Ok(out)
}

/// Every document `decree graph` writes, by filename: one per machine, then `system.md`.
pub fn render(project_root: &Path) -> Result<BTreeMap<String, String>, DecreeError> {
    // 0.4 config: `default_routine` and `routine_source` are the section 3
    // `default_machine` and `shared_source` until the config is rewritten.
    let config = AppConfig::load_from_project(project_root)?;
    let decree_dir = AppConfig::decree_dir(project_root);
    let machines =
        machine::load_machines(&decree_dir, config.resolved_routine_source().as_deref())?;
    let default_router = config.default_router.as_deref();
    let mut documents = BTreeMap::new();
    for (id, m) in &machines {
        let text = graph::machine_document(m, default_router).map_err(DecreeError::Other)?;
        documents.insert(format!("{id}.md"), text);
    }
    let crons = cron_machines(&decree_dir, &config.default_routine)?;
    documents.insert(
        graph::SYSTEM_FILE.to_string(),
        graph::system_document(&machines, &crons, default_router),
    );
    Ok(documents)
}

/// `(stem, machine)` for every `cron/*.md` in filename order. A file without `machine:`
/// (or its alias `routine:`) points at the default machine.
fn cron_machines(
    decree_dir: &Path,
    default_machine: &str,
) -> Result<Vec<(String, String)>, DecreeError> {
    let dir = decree_dir.join(config::CRON_DIR);
    let mut crons = Vec::new();
    for name in md_files(&dir)? {
        let rel = format!("{}/{name}", config::CRON_DIR);
        let text = std::fs::read_to_string(dir.join(&name))?;
        let fm = parse_frontmatter(&text)
            .map_err(|(line, msg)| DecreeError::Other(format!("{rel}: line {line}: {msg}")))?;
        let machine = match fm.map.get("machine").or_else(|| fm.map.get("routine")) {
            None => default_machine.to_string(),
            Some(Value::String(m)) => m.clone(),
            Some(_) => {
                return Err(DecreeError::Other(format!(
                    "{rel}: `machine` must be a string (run `decree check`)"
                )))
            }
        };
        let stem = name.strip_suffix(".md").unwrap_or(&name).to_string();
        crons.push((stem, machine));
    }
    Ok(crons)
}
