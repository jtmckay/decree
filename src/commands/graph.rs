//! `decree graph`: write `.decree/machines/<machine>/graph.md` for every machine and
//! `.decree/system.md`, each a Markdown document holding a Mermaid diagram, and remove a
//! stale `graph.md` (docs/reference/graph.md). `decree check` uses `stale_files` to warn when the
//! committed files are out of date.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::json;
use serde_norway::Value;

use crate::cli::Format;
use crate::commands::print_json;
use crate::error::DecreeError;
use crate::graph;
use crate::layout::{self, DECREE_DIR};
use crate::machine;
use crate::message::Message;

pub fn run(project_root: &Path, format: Format) -> Result<(), DecreeError> {
    let decree_dir = project_root.join(DECREE_DIR);
    let before = graph_files(&decree_dir)?;
    let written = write(project_root)?;
    let path = |name: &String| format!("{DECREE_DIR}/{name}");
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

/// Every graph file there is, relative to `.decree/`: each `machines/<id>/graph.md`, and
/// `system.md`, sorted.
fn graph_files(decree_dir: &Path) -> Result<Vec<String>, DecreeError> {
    let mut out = Vec::new();
    let machines = decree_dir.join(layout::MACHINES_DIR);
    if machines.is_dir() {
        for entry in std::fs::read_dir(&machines)? {
            let entry = entry?;
            let Ok(id) = entry.file_name().into_string() else {
                continue;
            };
            if entry.path().join(layout::GRAPH_FILE).is_file() {
                out.push(format!(
                    "{}/{}",
                    layout::machine_dir_rel(&id),
                    layout::GRAPH_FILE
                ));
            }
        }
    }
    if decree_dir.join(layout::SYSTEM_GRAPH_FILE).is_file() {
        out.push(layout::SYSTEM_GRAPH_FILE.to_string());
    }
    out.sort();
    Ok(out)
}

/// Write every document, `machines/<id>/graph.md` and `system.md`, and remove a `graph.md`
/// in a folder with no machine, and that folder once it is empty. Returns the files written,
/// relative to `.decree/`, sorted.
pub fn write(project_root: &Path) -> Result<Vec<String>, DecreeError> {
    let documents = render(project_root)?;
    let decree_dir = project_root.join(DECREE_DIR);
    for name in graph_files(&decree_dir)? {
        if !documents.contains_key(&name) {
            let path = decree_dir.join(&name);
            std::fs::remove_file(&path)?;
            if let Some(dir) = path.parent() {
                // Fails, as it should, unless the folder is now empty.
                let _ = std::fs::remove_dir(dir);
            }
        }
    }
    for (name, text) in &documents {
        std::fs::write(decree_dir.join(name), text)?;
    }
    Ok(documents.into_keys().collect())
}

/// How the graph files differ from what `decree graph` would write, one (file relative to
/// `.decree/`, what is wrong) pair per file. Empty when they are up to date.
pub fn stale_files(project_root: &Path) -> Result<Vec<(String, String)>, DecreeError> {
    let documents = render(project_root)?;
    let decree_dir = project_root.join(DECREE_DIR);
    let mut out = Vec::new();
    for (name, text) in &documents {
        match std::fs::read_to_string(decree_dir.join(name)) {
            Ok(on_disk) if on_disk == *text => {}
            Ok(_) => out.push((name.clone(), "out of date".to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                out.push((name.clone(), "missing".to_string()))
            }
            Err(e) => return Err(e.into()),
        }
    }
    for name in graph_files(&decree_dir)? {
        if !documents.contains_key(&name) {
            out.push((name, "no machine draws it".to_string()));
        }
    }
    Ok(out)
}

/// Every document `decree graph` writes, by path relative to `.decree/`: each machine's
/// `machines/<id>/graph.md`, and `system.md`.
fn render(project_root: &Path) -> Result<BTreeMap<String, String>, DecreeError> {
    let decree_dir = project_root.join(DECREE_DIR);
    let machines = machine::load_machines(&decree_dir)?;
    let mut documents = BTreeMap::new();
    for (id, m) in &machines {
        let text = graph::machine_document(m).map_err(DecreeError::Other)?;
        documents.insert(
            format!("{}/{}", layout::machine_dir_rel(id), layout::GRAPH_FILE),
            text,
        );
    }
    let crons = cron_machines(&decree_dir)?;
    documents.insert(
        layout::SYSTEM_GRAPH_FILE.to_string(),
        graph::system_document(&machines, &crons),
    );
    Ok(documents)
}

/// `(<machine folder>/<stem>, machine)` for every cron file, `machines/*/cron/*.md`, in
/// order. A file without `machine:` runs the machine whose folder holds it.
fn cron_machines(decree_dir: &Path) -> Result<Vec<(String, String)>, DecreeError> {
    let mut crons = Vec::new();
    for (owner, file) in crate::cron::file_names(decree_dir)? {
        let rel = format!(
            "{}/{}/{file}",
            layout::machine_dir_rel(&owner),
            layout::CRON_DIR
        );
        let text = std::fs::read_to_string(decree_dir.join(&rel))?;
        let fm = Message::parse(&text)
            .map_err(|(line, msg)| DecreeError::Other(format!("{rel}: line {line}: {msg}")))?;
        let machine = match fm.frontmatter.get("machine") {
            None => owner.clone(),
            Some(Value::String(m)) => m.clone(),
            Some(_) => {
                return Err(DecreeError::Other(format!(
                    "{rel}: `machine` must be a string (run `decree check`)"
                )))
            }
        };
        let stem = file.strip_suffix(".md").unwrap_or(&file);
        crons.push((format!("{owner}/{stem}"), machine));
    }
    Ok(crons)
}
