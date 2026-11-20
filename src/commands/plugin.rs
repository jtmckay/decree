//! `decree plugin list|add|export|import|install` (docs/reference/plugins.md): share a machine
//! as one file, `.decree/plugins/<name>.md`. `list` shows the official plugins built into
//! decree, `add` copies official plugins into `plugins/` and installs them, `export` writes a
//! plugin file from the installed copy, `import` checks a plugin file, copies it into
//! `plugins/` and installs it, and `install` installs plugins: the only way they are installed
//! but `add` and `import` (docs/reference/plugins.md, Installing). decree fetches nothing:
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
use crate::official;
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

    let changes = old
        .as_ref()
        .map(|old| changes(old.as_ref(), &plugin, false));
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

/// `decree plugin install [<name>...]`. With no name: install every plugin that is new or
/// changed, remove the installed copy of each deleted one, and keep every edited copy,
/// naming each conflict and exiting 1 on a conflict or a plugin that fails the schema
/// (docs/reference/plugins.md, Installing). With names: install those now, replacing an
/// installed copy that was edited; nothing is installed if one is missing or fails the
/// schema.
pub fn install(project_root: &Path, names: &[String], format: Format) -> Result<(), DecreeError> {
    let decree_dir = project_root.join(DECREE_DIR);
    if names.is_empty() {
        return install_all(&decree_dir, format);
    }
    let found = {
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
        Format::Json => print_json(&json!({
            "action": "install",
            "installed": installed,
            "removed": [],
            "conflicts": [],
        })),
        Format::Text => {
            for name in &installed {
                println!("installed {DECREE_DIR}/{}", plugin::plugin_file(name));
            }
            Ok(())
        }
    }
}

/// `decree plugin install` with no name: `plugin::sync`.
fn install_all(decree_dir: &Path, format: Format) -> Result<(), DecreeError> {
    let synced = plugin::sync(decree_dir)?;
    match format {
        Format::Json => print_json(&json!({
            "action": "install",
            "installed": synced.installed,
            "removed": synced.removed,
            "conflicts": synced.conflicts,
        }))?,
        Format::Text => {
            for name in &synced.installed {
                println!("installed {DECREE_DIR}/{}", plugin::plugin_file(name));
            }
            for name in &synced.removed {
                println!(
                    "removed the installed copy of {DECREE_DIR}/{}, which was deleted",
                    plugin::plugin_file(name)
                );
            }
            let quiet = synced.conflicts.is_empty() && synced.invalid.is_empty();
            if quiet && synced.installed.is_empty() && synced.removed.is_empty() {
                println!("every plugin is installed as its file says: nothing to install");
            }
        }
    }
    let notices = synced.notices();
    for note in &notices {
        eprintln!("{note}");
    }
    let failed = synced.conflicts.len() + synced.invalid.len();
    if failed > 0 {
        return Err(DecreeError::Other(format!(
            "{failed} plugin(s) not installed; see above"
        )));
    }
    Ok(())
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

/// `decree plugin export` with no name: export every plugin whose installed copy differs from
/// its plugin file (edited in place, or by a run), so the edits reach `plugins/`. `commit`
/// runs it before it commits. A plugin that fails the schema, or is not installed, is skipped,
/// and so is a conflict (its file changed since it was installed): exporting would write the
/// old installed version over the new file, so it is named and left for a person to merge.
/// With `dry_run`, it names them and writes nothing: `keep` holds the queue on them.
pub fn export_edited(
    project_root: &Path,
    dry_run: bool,
    format: Format,
) -> Result<(), DecreeError> {
    let decree_dir = project_root.join(DECREE_DIR);
    let mut exported = Vec::new();
    let mut conflicts = Vec::new();
    for found in plugin::read_all(&decree_dir)? {
        let Ok(p) = found.parsed else { continue };
        if !plugin::edited(&decree_dir, &p)? {
            continue;
        }
        if plugin::conflicted(&decree_dir, &p.name, &found.text)? {
            conflicts.push(p.name);
            continue;
        }
        if !dry_run {
            let fresh = plugin::export(&decree_dir, &p.name)?;
            plugin::write_and_install(&decree_dir, &fresh)?;
        }
        exported.push(p.name);
    }
    match format {
        Format::Json => print_json(&json!({
            "action": "export",
            "exported": exported,
            "conflicts": conflicts,
            "dry_run": dry_run,
        })),
        Format::Text => {
            for name in &conflicts {
                println!(
                    "not exported: {} changed since `{name}` was installed, and its installed copy was edited; merge by hand (`decree plugin export {name}` writes the copy, `decree plugin install {name}` takes the file)",
                    plugin::plugin_file(name)
                );
            }
            if exported.is_empty() && conflicts.is_empty() {
                println!("no plugin's installed copy differs from its file: nothing to export");
            }
            let verb = if dry_run { "would write" } else { "wrote" };
            for name in &exported {
                println!("{verb} {DECREE_DIR}/{}", plugin::plugin_file(name));
            }
            Ok(())
        }
    }
}

/// One official plugin as `decree plugin list` shows it.
fn listing(decree_dir: &Path, o: &official::Official) -> Result<Json, DecreeError> {
    let path = decree_dir.join(plugin::plugin_file(o.name));
    let (added, differs) = match std::fs::read(&path) {
        Ok(bytes) => (true, bytes != o.text.as_bytes()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (false, false),
        Err(e) => return Err(DecreeError::Other(format!("{}: {e}", path.display()))),
    };
    let needs = o.plugin().map(|p| p.needs).unwrap_or_default();
    Ok(json!({
        "name": o.name,
        "description": o.description(),
        "needs": needs,
        "added": added,
        "differs": differs,
    }))
}

/// Print the official plugins, one line each, marked `added` and `differs`.
fn print_listing(plugins: &[Json]) {
    let width = plugins
        .iter()
        .filter_map(|p| p["name"].as_str())
        .map(str::len)
        .max()
        .unwrap_or(0);
    for p in plugins {
        let mut marks = Vec::new();
        if p["added"] == true {
            marks.push("added");
        }
        if p["differs"] == true {
            marks.push("differs");
        }
        let marks = if marks.is_empty() {
            String::new()
        } else {
            format!(" [{}]", marks.join(", "))
        };
        println!(
            "  {:width$}  {}{marks}",
            p["name"].as_str().unwrap_or(""),
            p["description"].as_str().unwrap_or("")
        );
    }
}

/// The official plugins, as `decree plugin list` prints them; `init` prints them too.
pub fn print_official(decree_dir: &Path) -> Result<(), DecreeError> {
    let plugins = official::OFFICIAL
        .iter()
        .map(|o| listing(decree_dir, o))
        .collect::<Result<Vec<_>, _>>()?;
    println!(
        "Official plugins (built into decree {}):",
        env!("CARGO_PKG_VERSION")
    );
    print_listing(&plugins);
    Ok(())
}

/// `decree plugin list`: every official plugin with its description, marked `added` if
/// `plugins/` has it and `differs` if that copy is not this decree's.
pub fn list(project_root: &Path, format: Format) -> Result<(), DecreeError> {
    let decree_dir = project_root.join(DECREE_DIR);
    match format {
        Format::Json => {
            let plugins = official::OFFICIAL
                .iter()
                .map(|o| listing(&decree_dir, o))
                .collect::<Result<Vec<_>, _>>()?;
            print_json(&json!({
                "action": "list",
                "version": env!("CARGO_PKG_VERSION"),
                "plugins": plugins,
            }))
        }
        Format::Text => {
            print_official(&decree_dir)?;
            println!("Add one with `decree plugin add <name>`; `differs` means `decree plugin add --force <name>` would change it.");
            Ok(())
        }
    }
}

/// What `decree plugin add` did with one plugin.
struct Added {
    name: String,
    /// `added`, `needed` (added because a plugin needs it), `kept` or `replaced`.
    status: &'static str,
    /// For `needed`: the plugins that need it.
    needed_by: Vec<String>,
    /// For `replaced`: each file added, changed or removed, with the lines it drops and adds.
    changes: Vec<Json>,
    /// For `replaced`: whether the installed copy had edits in place that were not exported.
    dropped_edits_in_place: bool,
}

/// `decree plugin add <name>...`: copy the official plugins, and every official plugin they
/// need that `plugins/` does not have, into `plugins/`, then install them. A named plugin
/// already there is kept, unless `force`, which replaces it with this decree's copy and prints
/// what changes. An unknown name fails before anything is written.
pub fn add(
    project_root: &Path,
    names: &[String],
    force: bool,
    format: Format,
) -> Result<(), DecreeError> {
    let decree_dir = project_root.join(DECREE_DIR);
    let unknown: Vec<&String> = names
        .iter()
        .filter(|n| official::get(n).is_none())
        .collect();
    if !unknown.is_empty() {
        let list = unknown
            .iter()
            .map(|n| format!("`{n}`"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(DecreeError::Other(format!(
            "no official plugin {list}; nothing was added. The official plugins are: {}",
            official::names().join(", ")
        )));
    }

    // The named plugins first, then each need not yet in plugins/, breadth first.
    let mut plan: Vec<Added> = Vec::new();
    let mut queue: Vec<(String, Option<String>)> =
        names.iter().map(|n| (n.clone(), None)).collect();
    let mut i = 0;
    while i < queue.len() {
        let (name, needed_by) = queue[i].clone();
        i += 1;
        if let Some(existing) = plan.iter_mut().find(|a| a.name == name) {
            if let Some(by) = needed_by {
                if existing.status == "needed" && !existing.needed_by.contains(&by) {
                    existing.needed_by.push(by);
                }
            }
            continue;
        }
        let o = official::get(&name).ok_or_else(|| {
            DecreeError::Other(format!("official plugin `{name}` is not built into decree"))
        })?;
        let plugin = o.plugin().map_err(DecreeError::Other)?;
        let present = decree_dir.join(plugin::plugin_file(&name)).is_file();
        let status = match (needed_by.is_some(), present, force) {
            // A need already in plugins/ is the project's: left as it is, without a word.
            (true, true, _) => continue,
            (true, false, _) => "needed",
            (false, true, false) => "kept",
            (false, true, true) => "replaced",
            (false, false, _) => "added",
        };
        if machine_without_plugin(&decree_dir, &name) && status != "replaced" {
            return Err(DecreeError::Other(format!(
                "{DECREE_DIR}/{MACHINES_DIR}/{name}.yml exists and no plugin installed it; export it (`decree plugin export {name}`) or move it, then add `{name}` again; nothing was added"
            )));
        }
        for need in &plugin.needs {
            queue.push((need.clone(), Some(name.clone())));
        }
        plan.push(Added {
            name,
            status,
            needed_by: needed_by.into_iter().collect(),
            changes: Vec::new(),
            dropped_edits_in_place: false,
        });
    }

    for a in plan.iter_mut().filter(|a| a.status != "kept") {
        let o = official::get(&a.name).ok_or_else(|| {
            DecreeError::Other(format!(
                "official plugin `{}` is not built into decree",
                a.name
            ))
        })?;
        let new = o.plugin().map_err(DecreeError::Other)?;
        let target = decree_dir.join(plugin::plugin_file(&a.name));
        if a.status == "replaced" {
            let old = plugin::read_one(&decree_dir, &a.name)?;
            let old_plugin = old.parsed.as_ref().ok();
            a.changes = changes(old_plugin, &new, true);
            a.dropped_edits_in_place = match old_plugin {
                Some(p) => plugin::edited(&decree_dir, p)?,
                None => false,
            };
        }
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| DecreeError::Other(format!("{}: {e}", dir.display())))?;
        }
        crate::message::write_replace(&target, o.text.as_bytes())
            .map_err(|(path, e)| DecreeError::Other(format!("{}: {e}", path.display())))?;
        plugin::install(&decree_dir, &new, o.text.as_bytes())?;
    }
    if plan.iter().any(|a| a.status != "kept") {
        // Draw the machines added, so `.decree/graph/` stays current. The plugins are added
        // either way: a project whose own machines do not load yet is told, not refused.
        if let Err(e) = crate::commands::graph::write(project_root) {
            eprintln!("warning: .decree/graph/ not redrawn ({e}); run `decree check`, then `decree graph`");
        }
    }

    match format {
        Format::Json => {
            let plugins: Vec<Json> = plan
                .iter()
                .map(|a| {
                    json!({
                        "plugin": a.name,
                        "file": format!("{DECREE_DIR}/{}", plugin::plugin_file(&a.name)),
                        "status": a.status,
                        "needed_by": a.needed_by,
                        "changes": a.changes,
                        "dropped_edits_in_place": a.dropped_edits_in_place,
                    })
                })
                .collect();
            print_json(&json!({ "action": "add", "plugins": plugins }))
        }
        Format::Text => {
            for a in &plan {
                let file = format!("{DECREE_DIR}/{}", plugin::plugin_file(&a.name));
                match a.status {
                    "added" => println!("added {file}"),
                    "needed" => println!(
                        "added {file}, needed by {}",
                        a.needed_by.join(", ")
                    ),
                    "kept" => println!(
                        "kept {file}: already in {DECREE_DIR}/{PLUGINS_DIR}/, so nothing changed; `decree plugin add --force {}` replaces it with this decree's copy",
                        a.name
                    ),
                    _ => {
                        println!("replaced {file} with this decree's copy:");
                        print_line_changes(&a.changes);
                        if a.dropped_edits_in_place {
                            println!(
                                "  and dropped the edits in place to {DECREE_DIR}/{MACHINES_DIR}/{0}.yml, {DECREE_DIR}/scripts/{0}/ or {DECREE_DIR}/cron/{0}/, which were not exported",
                                a.name
                            );
                        }
                    }
                }
            }
            let installed: Vec<&str> = plan
                .iter()
                .filter(|a| a.status != "kept")
                .map(|a| a.name.as_str())
                .collect();
            if !installed.is_empty() {
                println!("installed {}", installed.join(", "));
            }
            Ok(())
        }
    }
}

/// Whether `machines/<name>.yml` exists and no plugin file names it.
fn machine_without_plugin(decree_dir: &Path, name: &str) -> bool {
    decree_dir
        .join(MACHINES_DIR)
        .join(format!("{name}.yml"))
        .exists()
        && !decree_dir.join(plugin::plugin_file(name)).is_file()
}

/// What replacing `old` with `new` changes, file by file in path order: each file added,
/// changed or removed, relative to the project root, and `needs` when it changes. With
/// `lines`, each also names the lines it drops (`-`) and adds (`+`). An `old` that fails the
/// schema counts as nothing installed.
fn changes(old: Option<&Plugin>, new: &Plugin, lines: bool) -> Vec<Json> {
    let before: BTreeMap<String, String> = old
        .map(Plugin::files)
        .unwrap_or_default()
        .into_iter()
        .collect();
    let after: BTreeMap<String, String> = new.files().into_iter().collect();
    let mut paths: Vec<&String> = before.keys().chain(after.keys()).collect();
    paths.sort();
    paths.dedup();
    let mut out = Vec::new();
    for path in paths {
        let (a, b) = (before.get(path), after.get(path));
        let change = match (a, b) {
            (None, Some(_)) => "added",
            (Some(_), None) => "removed",
            (Some(x), Some(y)) if x != y => "changed",
            _ => continue,
        };
        let mut entry = json!({ "path": format!("{DECREE_DIR}/{path}"), "change": change });
        if lines {
            entry["lines"] = json!(diff_lines(
                a.map_or("", String::as_str),
                b.map_or("", String::as_str)
            ));
        }
        out.push(entry);
    }
    let old_needs = old.map(|o| o.needs.clone()).unwrap_or_default();
    if old_needs != new.needs {
        let mut entry = json!({ "path": "needs", "change": "changed" });
        if lines {
            entry["lines"] = json!([
                format!("- {}", old_needs.join(", ")),
                format!("+ {}", new.needs.join(", "))
            ]);
        }
        out.push(entry);
    }
    out
}

fn print_line_changes(changes: &[Json]) {
    if changes.is_empty() {
        println!("  no change");
    }
    for c in changes {
        println!(
            "  {} {}",
            c["change"].as_str().unwrap_or(""),
            c["path"].as_str().unwrap_or("")
        );
        for line in c["lines"].as_array().into_iter().flatten() {
            println!("    {}", line.as_str().unwrap_or(""));
        }
    }
}

/// The lines `a` has and `b` does not (`- `) and those `b` has and `a` does not (`+ `), in
/// order, from their longest common subsequence: the changed lines of a diff, without context.
fn diff_lines(a: &str, b: &str) -> Vec<String> {
    let a: Vec<&str> = a.lines().collect();
    let b: Vec<&str> = b.lines().collect();
    // Lines common at both ends need no table.
    let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let tail = a[head..]
        .iter()
        .rev()
        .zip(b[head..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (a, b) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
    let (n, m) = (a.len(), b.len());
    // lcs[i][j]: the longest common subsequence of a[i..] and b[j..].
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            i += 1;
            j += 1;
        } else if i < n && (j == m || lcs[i + 1][j] >= lcs[i][j + 1]) {
            out.push(format!("- {}", a[i]));
            i += 1;
        } else {
            out.push(format!("+ {}", b[j]));
            j += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_lines_names_only_the_changed_lines() {
        assert!(diff_lines("a\nb\n", "a\nb\n").is_empty());
        assert_eq!(diff_lines("a\nb\nc\n", "a\nx\nc\n"), ["- b", "+ x"]);
        assert_eq!(diff_lines("", "a\n"), ["+ a"]);
        assert_eq!(diff_lines("a\nb\n", "b\n"), ["- a"]);
        assert_eq!(diff_lines("a\nb\nc\nd\n", "a\nc\nd\ne\n"), ["- b", "+ e"]);
    }

    #[test]
    fn changes_with_lines_shows_a_dropped_edit_and_needs() {
        let text = official::get("develop").unwrap().text;
        let new = Plugin::parse(text, None).unwrap();
        let mut old = new.clone();
        old.scripts
            .insert("gate.sh".into(), "#!/usr/bin/env bash\ncargo test\n".into());
        old.needs.clear();
        let changes = changes(Some(&old), &new, true);
        assert_eq!(changes.len(), 2, "{changes:?}");
        assert_eq!(changes[0]["path"], ".decree/scripts/develop/gate.sh");
        assert_eq!(changes[0]["change"], "changed");
        let lines = changes[0]["lines"].as_array().unwrap();
        assert!(lines.iter().any(|l| l == "- cargo test"), "{lines:?}");
        assert_eq!(changes[1]["path"], "needs");
    }

    #[test]
    fn export_edited_leaves_a_conflict_alone() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let decree_dir = root.join(DECREE_DIR);
        let plugins = decree_dir.join(plugin::PLUGINS_DIR);
        std::fs::create_dir_all(&plugins).unwrap();
        let text = official::get("develop").unwrap().text;
        let file = plugins.join("develop.md");
        std::fs::write(&file, text).unwrap();
        plugin::sync(&decree_dir).unwrap();
        let gate = decree_dir.join("scripts/develop/gate.sh");
        std::fs::write(&gate, "#!/usr/bin/env bash\ncargo test\n").unwrap();
        // A pull changes the plugin file under the edit.
        let pulled = format!("{text}\nA teammate's note.\n");
        std::fs::write(&file, &pulled).unwrap();
        assert_eq!(plugin::sync(&decree_dir).unwrap().conflicts, ["develop"]);
        export_edited(root, false, Format::Json).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), pulled);
        assert!(std::fs::read_to_string(&gate)
            .unwrap()
            .contains("cargo test"));
        // Without the pull, the edit is exported.
        std::fs::write(&file, text).unwrap();
        plugin::sync(&decree_dir).unwrap();
        export_edited(root, false, Format::Json).unwrap();
        assert!(std::fs::read_to_string(&file)
            .unwrap()
            .contains("cargo test"));
    }
}
