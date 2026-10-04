//! `decree graph`: write `.decree/graph/<machine>.md` for every machine and
//! `.decree/graph/system.md`, each a Markdown document holding a Mermaid diagram, and remove
//! stale `.md` files there (docs/reference/graph.md). `decree check` uses `stale_files` to warn when the
//! committed files are out of date.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::json;
use serde_norway::Value;

use crate::cli::Format;
use crate::commands::check::md_files;
use crate::commands::print_json;
use crate::error::DecreeError;
use crate::graph;
use crate::layout::{self, DECREE_DIR};
use crate::machine;
use crate::message::Message;

/// Directory under `.decree/` that `decree graph` writes.
const GRAPH_DIR: &str = "graph";

pub fn run(project_root: &Path, format: Format) -> Result<(), DecreeError> {
    let dir = project_root.join(DECREE_DIR).join(GRAPH_DIR);
    let before = md_files(&dir)?;
    let written = write(project_root)?;
    let path = |name: &String| format!("{DECREE_DIR}/{GRAPH_DIR}/{name}");
    match format {
        Format::Text => {
            for name in &written {
                println!("{}", path(name));
            }
            Ok(())
        }
        Format::Json => print_json(&json!({
            "written": written.iter().map(path).collect::<Vec<_>>(),
            "removed": before
                .iter()
                .filter(|name| !written.contains(name))
                .map(path)
                .collect::<Vec<_>>(),
        })),
    }
}

/// Write every document into `.decree/graph/` and remove the `.md` files there that no
/// machine produces. Returns the filenames written, in name order.
pub fn write(project_root: &Path) -> Result<Vec<String>, DecreeError> {
    let documents = render(project_root)?;
    let dir = project_root.join(DECREE_DIR).join(GRAPH_DIR);
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

/// How `.decree/graph/` differs from what `decree graph` would write, one (file relative to
/// `.decree/`, what is wrong) pair per file. Empty when it is up to date.
pub fn stale_files(project_root: &Path) -> Result<Vec<(String, String)>, DecreeError> {
    let documents = render(project_root)?;
    let dir = project_root.join(DECREE_DIR).join(GRAPH_DIR);
    let mut out = Vec::new();
    for (name, text) in &documents {
        match std::fs::read_to_string(dir.join(name)) {
            Ok(on_disk) if on_disk == *text => {}
            Ok(_) => out.push((format!("{GRAPH_DIR}/{name}"), "out of date".to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                out.push((format!("{GRAPH_DIR}/{name}"), "missing".to_string()))
            }
            Err(e) => return Err(e.into()),
        }
    }
    for name in md_files(&dir)? {
        if !documents.contains_key(&name) {
            out.push((
                format!("{GRAPH_DIR}/{name}"),
                "no machine draws it".to_string(),
            ));
        }
    }
    Ok(out)
}

/// Every document `decree graph` writes, by filename: one per machine, then `system.md`.
fn render(project_root: &Path) -> Result<BTreeMap<String, String>, DecreeError> {
    let decree_dir = project_root.join(DECREE_DIR);
    let machines = machine::load_machines(&decree_dir)?;
    let mut documents = BTreeMap::new();
    for (id, m) in &machines {
        let text = graph::machine_document(m).map_err(DecreeError::Other)?;
        documents.insert(format!("{id}.md"), text);
    }
    let crons = cron_machines(&decree_dir)?;
    documents.insert(
        graph::SYSTEM_FILE.to_string(),
        graph::system_document(&machines, &crons),
    );
    Ok(documents)
}

/// `(stem, machine)` for every `cron/*.md` in filename order. A file without `machine:` is
/// an error.
fn cron_machines(decree_dir: &Path) -> Result<Vec<(String, String)>, DecreeError> {
    let dir = decree_dir.join(layout::CRON_DIR);
    let mut crons = Vec::new();
    for name in md_files(&dir)? {
        let rel = format!("{}/{name}", layout::CRON_DIR);
        let text = std::fs::read_to_string(dir.join(&name))?;
        let fm = Message::parse(&text)
            .map_err(|(line, msg)| DecreeError::Other(format!("{rel}: line {line}: {msg}")))?;
        let machine = match fm.frontmatter.get("machine") {
            None => {
                return Err(DecreeError::Other(format!(
                    "{rel}: no `machine` key (run `decree check`)"
                )))
            }
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
