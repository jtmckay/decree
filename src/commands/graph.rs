//! `decree graph [<machine>]`: print one machine, or the whole system, as a Markdown
//! document holding a Mermaid diagram (spec section 9).

use std::path::Path;

use serde_norway::Value;

use crate::commands::check::{md_files, parse_frontmatter};
use crate::config::{self, AppConfig};
use crate::error::DecreeError;
use crate::graph;
use crate::machine;

pub fn run(project_root: &Path, machine: Option<&str>) -> Result<(), DecreeError> {
    print!("{}", render(project_root, machine)?);
    Ok(())
}

/// The document `decree graph` prints.
pub fn render(project_root: &Path, machine: Option<&str>) -> Result<String, DecreeError> {
    // 0.4 config: `default_routine` and `routine_source` are the section 3
    // `default_machine` and `shared_source` until the config is rewritten.
    let config = AppConfig::load_from_project(project_root)?;
    let decree_dir = AppConfig::decree_dir(project_root);
    let machines =
        machine::load_machines(&decree_dir, config.resolved_routine_source().as_deref())?;
    match machine {
        Some(id) => {
            let m = machines
                .get(id)
                .ok_or_else(|| DecreeError::Other(format!("unknown machine `{id}`")))?;
            graph::machine_document(m).map_err(DecreeError::Other)
        }
        None => {
            let crons = cron_machines(&decree_dir, &config.default_routine)?;
            Ok(graph::system_document(&machines, &crons))
        }
    }
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
