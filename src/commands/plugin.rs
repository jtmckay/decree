//! `decree plugin export|import|install` (docs/reference/plugins.md): share a machine as one
//! file, `.decree/plugins/<name>.md`. `export` writes it from the installed copy, `import`
//! checks a plugin file, copies it into `plugins/` and installs it, and `install` installs
//! plugins now, replacing an installed copy that was edited. decree fetches nothing:
//! `curl -fsSL <url> | decree plugin import -` imports from a URL.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use serde_json::{json, Value as Json};

use crate::cli::Format;
use crate::commands::print_json;
use crate::error::DecreeError;
use crate::layout::DECREE_DIR;
use crate::machine::MACHINES_DIR;
use crate::plugin::{self, Plugin, PLUGINS_DIR};

/// `decree plugin export <name>`: write `plugins/<name>.md` from the installed copy.
pub fn export(project_root: &Path, name: &str, format: Format) -> Result<(), DecreeError> {
    let decree_dir = project_root.join(DECREE_DIR);
    let plugin = plugin::export(&decree_dir, name)?;
    plugin::write_and_install(&decree_dir, &plugin)?;
    let file = format!("{DECREE_DIR}/{}", plugin::plugin_file(name));
    let summary = summary(&plugin);
    match format {
        Format::Json => {
            let mut doc = json!({ "action": "export", "plugin": name, "file": file });
            merge(&mut doc, summary);
            print_json(&doc)
        }
        Format::Text => {
            println!("wrote {file}");
            print_summary(&summary);
            Ok(())
        }
    }
}

/// `decree plugin import <file|->`: check the plugin, copy it to `plugins/<name>.md` and
/// install it. Refuses, writing nothing, a plugin that fails the schema, whose `needs` are
/// not in `plugins/`, or whose name is taken, unless `force`.
pub fn import(
    project_root: &Path,
    source: &str,
    force: bool,
    format: Format,
) -> Result<(), DecreeError> {
    let decree_dir = project_root.join(DECREE_DIR);
    let (shown, bytes) = if source == "-" {
        let mut bytes = Vec::new();
        std::io::stdin()
            .read_to_end(&mut bytes)
            .map_err(|e| DecreeError::Other(format!("cannot read stdin: {e}")))?;
        ("stdin".to_string(), bytes)
    } else {
        let bytes = std::fs::read(source)
            .map_err(|e| DecreeError::Other(format!("cannot read {source}: {e}")))?;
        (source.to_string(), bytes)
    };
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| DecreeError::Other(format!("{shown}: not valid UTF-8")))?;
    let plugin = Plugin::parse(text, None).map_err(|problems| {
        let lines: Vec<String> = problems
            .iter()
            .map(|p| match p.line {
                Some(line) => format!("{shown}: line {line}: {} (P1)", p.message),
                None => format!("{shown}: {} (P1)", p.message),
            })
            .collect();
        DecreeError::Other(format!(
            "{}\nnot a valid plugin; nothing was imported",
            lines.join("\n")
        ))
    })?;
    let name = plugin.name.clone();

    let missing: Vec<&String> = plugin
        .needs
        .iter()
        .filter(|n| !decree_dir.join(plugin::plugin_file(n)).is_file())
        .collect();
    if !missing.is_empty() {
        let list = missing
            .iter()
            .map(|n| format!("`{n}`"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(DecreeError::Other(format!(
            "plugin `{name}` needs {list}, not in {DECREE_DIR}/{PLUGINS_DIR}/: import {} first; nothing was imported",
            if missing.len() == 1 { "it" } else { "them" }
        )));
    }

    let target = decree_dir.join(plugin::plugin_file(&name));
    let old = if target.is_file() {
        let found = plugin::read_one(&decree_dir, &name)?;
        Some(found.parsed.ok())
    } else {
        None
    };
    let machine = decree_dir.join(MACHINES_DIR).join(format!("{name}.yml"));
    if !force {
        if old.is_some() {
            return Err(DecreeError::Other(format!(
                "{DECREE_DIR}/{} exists; run with --force to replace it",
                plugin::plugin_file(&name)
            )));
        }
        if machine.exists() {
            return Err(DecreeError::Other(format!(
                "{DECREE_DIR}/{MACHINES_DIR}/{name}.yml exists and no plugin installed it; run with --force to replace it"
            )));
        }
    }

    let changes = old.as_ref().map(|old| changes(old.as_ref(), &plugin));
    let summary = summary(&plugin);
    let file = format!("{DECREE_DIR}/{}", plugin::plugin_file(&name));
    if format == Format::Text {
        println!("plugin `{name}`: {}", plugin.description().trim());
        print_summary(&summary);
        if let Some(changes) = &changes {
            println!("replaces {file}:");
            if changes.is_empty() {
                println!("  no change");
            }
            for c in changes {
                println!(
                    "  {} {}",
                    c["change"].as_str().unwrap_or(""),
                    c["path"].as_str().unwrap_or("")
                );
            }
        }
    }

    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| DecreeError::Other(format!("{}: {e}", dir.display())))?;
    }
    crate::message::write_replace(&target, &bytes)
        .map_err(|(path, e)| DecreeError::Other(format!("{}: {e}", path.display())))?;
    plugin::install(&decree_dir, &plugin, &bytes)?;

    match format {
        Format::Json => {
            let mut doc = json!({
                "action": "import",
                "plugin": name,
                "file": file,
                "description": plugin.description(),
                "replaced": changes.is_some(),
                "changes": changes.unwrap_or_default(),
            });
            merge(&mut doc, summary);
            print_json(&doc)
        }
        Format::Text => {
            println!("installed {file}");
            Ok(())
        }
    }
}

/// `decree plugin install [<name>...]`: install the named plugins, or every one, now,
/// replacing an installed copy that was edited. Nothing is installed if one is missing or
/// fails the schema.
pub fn install(project_root: &Path, names: &[String], format: Format) -> Result<(), DecreeError> {
    let decree_dir = project_root.join(DECREE_DIR);
    let found = if names.is_empty() {
        plugin::read_all(&decree_dir)?
    } else {
        let mut out = Vec::new();
        for name in names {
            if !crate::machine::is_ident(name)
                || !decree_dir.join(plugin::plugin_file(name)).is_file()
            {
                return Err(DecreeError::Other(format!(
                    "no plugin `{name}`: {DECREE_DIR}/{} does not exist",
                    plugin::plugin_file(name)
                )));
            }
            out.push(plugin::read_one(&decree_dir, name)?);
        }
        out
    };
    let mut ready = Vec::new();
    let mut problems = Vec::new();
    for f in found {
        match f.parsed {
            Ok(p) => ready.push((p, f.text)),
            Err(ps) => problems.extend(ps.into_iter().map(|p| match p.line {
                Some(line) => format!(
                    "{}: line {line}: {} (P1)",
                    plugin::plugin_file(&f.name),
                    p.message
                ),
                None => format!("{}: {} (P1)", plugin::plugin_file(&f.name), p.message),
            })),
        }
    }
    if !problems.is_empty() {
        return Err(DecreeError::Other(format!(
            "{}\nnothing was installed",
            problems.join("\n")
        )));
    }
    let mut installed = Vec::new();
    for (p, text) in &ready {
        plugin::install(&decree_dir, p, text)?;
        installed.push(p.name.clone());
    }
    match format {
        Format::Json => print_json(&json!({ "action": "install", "installed": installed })),
        Format::Text => {
            for name in &installed {
                println!("installed {DECREE_DIR}/{}", plugin::plugin_file(name));
            }
            Ok(())
        }
    }
}

/// What a plugin runs: each file it installs, relative to the project root, with its line
/// count, and `needs`.
fn summary(plugin: &Plugin) -> Json {
    let files: Vec<Json> = plugin
        .files()
        .iter()
        .map(|(path, text)| json!({ "path": format!("{DECREE_DIR}/{path}"), "lines": text.lines().count() }))
        .collect();
    json!({ "files": files, "needs": plugin.needs })
}

fn print_summary(summary: &Json) {
    for f in summary["files"].as_array().into_iter().flatten() {
        println!(
            "  {} ({} line{})",
            f["path"].as_str().unwrap_or(""),
            f["lines"].as_u64().unwrap_or(0),
            if f["lines"].as_u64() == Some(1) {
                ""
            } else {
                "s"
            }
        );
    }
    let needs: Vec<&str> = summary["needs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Json::as_str)
        .collect();
    if needs.is_empty() {
        println!("needs: none");
    } else {
        println!("needs: {}", needs.join(", "));
    }
}

fn merge(doc: &mut Json, extra: Json) {
    if let (Some(doc), Json::Object(extra)) = (doc.as_object_mut(), extra) {
        doc.extend(extra);
    }
}

/// What installing `new` over `old` changes: each file added, changed or removed, relative to
/// the project root, and `needs` when it changes. An `old` that fails the schema counts as
/// nothing installed.
fn changes(old: Option<&Plugin>, new: &Plugin) -> Vec<Json> {
    let before: BTreeMap<String, String> = old
        .map(Plugin::files)
        .unwrap_or_default()
        .into_iter()
        .collect();
    let after: BTreeMap<String, String> = new.files().into_iter().collect();
    let mut out = Vec::new();
    for (path, text) in &after {
        let change = match before.get(path) {
            None => "added",
            Some(t) if t != text => "changed",
            Some(_) => continue,
        };
        out.push(json!({ "path": format!("{DECREE_DIR}/{path}"), "change": change }));
    }
    for path in before.keys().filter(|p| !after.contains_key(*p)) {
        out.push(json!({ "path": format!("{DECREE_DIR}/{path}"), "change": "removed" }));
    }
    if old.map(|o| &o.needs) != Some(&new.needs) && !(old.is_none() && new.needs.is_empty()) {
        out.push(json!({ "path": "needs", "change": "changed" }));
    }
    out
}
