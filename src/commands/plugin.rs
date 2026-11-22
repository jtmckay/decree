//! `decree install` and `decree list|export` (docs/reference/plugins.md): share a
//! machine as one file, `.decree/plugins/<name>.md`, committed; what it installs into
//! `.decree/machines/<name>/` is ignored by git. `decree install` with no argument installs every
//! plugin as its file says (the step after a checkout); `decree install <name>` installs that
//! plugin, seeding its file from the official copy built into decree when the project has none;
//! `decree install <file>`, or `-` for stdin, copies it into `plugins/` and installs
//! it. decree fetches nothing: `curl -fsSL <url> | decree install -` installs from a URL. `list`
//! shows the official plugins, and `export` writes a plugin file from the installed copy.

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

/// `decree export <name>`: write `plugins/<name>.md` from the installed copy.
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

/// `decree install <file|->`: check the plugin, copy it to
/// `plugins/<name>.md` and install it, seeding each official plugin it needs that `plugins/`
/// lacks. Refuses, writing nothing, a plugin that fails the schema, that needs a plugin
/// neither in `plugins/` nor official, or whose name is taken, unless `force`.
fn import(
    decree_dir: &Path,
    shown: &str,
    bytes: Vec<u8>,
    force: bool,
    format: Format,
) -> Result<(), DecreeError> {
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
            "{}\nnot a valid plugin; nothing was installed",
            lines.join("\n")
        ))
    })?;
    let name = plugin.name.clone();

    // Each need not in plugins/ that decree ships is seeded from its official copy, with the
    // official plugins it needs in turn, as `decree install <name>` does; any other is missing.
    let mut seed: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    let mut queue: Vec<String> = plugin.needs.clone();
    while let Some(n) = queue.pop() {
        if n == name || seed.contains(&n) || decree_dir.join(plugin::plugin_file(&n)).is_file() {
            continue;
        }
        match official::get(&n).map(|o| o.plugin()) {
            Some(Ok(p)) => {
                queue.extend(p.needs.iter().cloned());
                seed.push(n);
            }
            _ => {
                if !missing.contains(&n) {
                    missing.push(n);
                }
            }
        }
    }
    if !missing.is_empty() {
        missing.sort();
        let list = missing
            .iter()
            .map(|n| format!("`{n}`"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(DecreeError::Other(format!(
            "plugin `{name}` needs {list}, not in {DECREE_DIR}/{PLUGINS_DIR}/ and not an official plugin: install {} first; nothing was installed",
            if missing.len() == 1 { "it" } else { "them" }
        )));
    }
    seed.sort();

    let target = decree_dir.join(plugin::plugin_file(&name));
    let old = if target.is_file() {
        let found = plugin::read_one(decree_dir, &name)?;
        Some(found.parsed.ok())
    } else {
        None
    };
    let machine = decree_dir.join(crate::layout::machine_file_rel(&name));
    if !force {
        if old.is_some() {
            return Err(DecreeError::Other(format!(
                "{DECREE_DIR}/{} exists; run with --force to replace it",
                plugin::plugin_file(&name)
            )));
        }
        if machine.exists() {
            return Err(DecreeError::Other(format!(
                "{DECREE_DIR}/{} exists and no plugin installed it; run with --force to replace it",
                crate::layout::machine_file_rel(&name)
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
    for n in &seed {
        let o = official::get(n).ok_or_else(|| {
            DecreeError::Other(format!("official plugin `{n}` is not built into decree"))
        })?;
        let p = o.plugin().map_err(DecreeError::Other)?;
        let path = decree_dir.join(plugin::plugin_file(n));
        crate::message::write_replace(&path, o.text.as_bytes())
            .map_err(|(path, e)| DecreeError::Other(format!("{}: {e}", path.display())))?;
        plugin::install(decree_dir, &p, o.text.as_bytes())?;
        if format == Format::Text {
            println!(
                "added {DECREE_DIR}/{}, needed by {name}",
                plugin::plugin_file(n)
            );
        }
    }
    crate::message::write_replace(&target, &bytes)
        .map_err(|(path, e)| DecreeError::Other(format!("{}: {e}", path.display())))?;
    plugin::install(decree_dir, &plugin, &bytes)?;

    match format {
        Format::Json => {
            let mut doc = json!({
                "action": "import",
                "plugin": name,
                "file": file,
                "description": plugin.description(),
                "replaced": changes.is_some(),
                "changes": changes.unwrap_or_default(),
                "added": seed,
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

/// `decree install [<name>...|<file>|-]` (docs/reference/plugins.md, Installing). No argument:
/// every plugin is installed as its file says (`plugin::sync`); stdin is never read, so a
/// script or a loop can run it safely. A name is a plugin, installed from `plugins/<name>.md`, or from
/// decree's official copy when `plugins/` has none or with `force`. Anything else is a plugin
/// file, or `-` for stdin, imported alone.
///
/// An install that succeeds, or one with no argument that installed what it could, also
/// writes `.decree/schema/`, quietly (it is generated, like a graph), so the step after a clone, a pull or an upgrade leaves the
/// project's schemas this decree's.
pub fn install(
    project_root: &Path,
    sources: &[String],
    force: bool,
    format: Format,
) -> Result<(), DecreeError> {
    let result = install_plugins(project_root, sources, force, format);
    // With no argument, plugins that installed stay installed when another is a conflict,
    // so the schemas are written then too; a refused install by name or file writes nothing.
    if result.is_ok() || (sources.is_empty() && !force) {
        // The install's own error, if any, is the one to report.
        let written = crate::commands::schema::write(project_root);
        result?;
        written?;
        return Ok(());
    }
    result
}

fn install_plugins(
    project_root: &Path,
    sources: &[String],
    force: bool,
    format: Format,
) -> Result<(), DecreeError> {
    let decree_dir = project_root.join(DECREE_DIR);
    let read_stdin = || -> Result<Vec<u8>, DecreeError> {
        let mut bytes = Vec::new();
        std::io::stdin()
            .read_to_end(&mut bytes)
            .map_err(|e| DecreeError::Other(format!("cannot read stdin: {e}")))?;
        Ok(bytes)
    };
    if sources.is_empty() {
        if force {
            return Err(DecreeError::Other(
                "--force replaces a plugin by name (`decree install --force <name>`) or from a file; with no argument, `decree install` installs every plugin as its file says".into(),
            ));
        }
        return install_all(&decree_dir, format);
    }
    if sources.iter().all(|s| crate::machine::is_ident(s)) {
        return install_named(project_root, sources, force, format);
    }
    let [source] = sources else {
        return Err(DecreeError::Other(
            "install one plugin file at a time, or plugins by name only".into(),
        ));
    };
    let (shown, bytes) = if source == "-" {
        ("stdin".to_string(), read_stdin()?)
    } else {
        let bytes = std::fs::read(source)
            .map_err(|e| DecreeError::Other(format!("cannot read {source}: {e}")))?;
        (source.clone(), bytes)
    };
    import(&decree_dir, &shown, bytes, force, format)
}

/// `decree install` with no argument: `plugin::sync`.
fn install_all(decree_dir: &Path, format: Format) -> Result<(), DecreeError> {
    let synced = plugin::sync(decree_dir)?;
    match format {
        Format::Json => print_json(&json!({
            "action": "sync",
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

/// `decree export` with no name: export every plugin whose installed copy differs from
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
                    "not exported: {} changed since `{name}` was installed, and its installed copy was edited; merge by hand (`decree export {name}` writes the copy, `decree install {name}` takes the file)",
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

/// `decree list`, as JSON: the project's plugins, the machines no plugin installed, and the
/// official plugins the project does not have.
fn listing(decree_dir: &Path) -> Result<Json, DecreeError> {
    let status = plugin::status(decree_dir)?;
    let mut plugins = Vec::new();
    for found in plugin::read_all(decree_dir)? {
        let o = official::get(&found.name);
        let differs = o.is_some_and(|o| found.text != o.text.as_bytes());
        let (description, state) = match &found.parsed {
            Err(_) => (String::new(), "invalid"),
            Ok(p) => {
                let state = if status.conflicts.contains(&p.name) {
                    "conflict"
                } else if status.installed.contains(&p.name) {
                    "not installed"
                } else if plugin::edited(decree_dir, p)? {
                    "edited"
                } else {
                    "installed"
                };
                (p.description().trim().to_string(), state)
            }
        };
        plugins.push(json!({
            "name": found.name,
            "description": description,
            "official": o.is_some(),
            "differs": differs,
            "state": state,
        }));
    }
    let ids: std::collections::BTreeSet<String> = crate::machine::machine_paths(decree_dir)?
        .into_keys()
        .collect();
    let unplugged = plugin::unplugged(decree_dir, &ids)?;
    let available: Vec<Json> = official::OFFICIAL
        .iter()
        .filter(|o| !decree_dir.join(plugin::plugin_file(o.name)).is_file())
        .map(|o| {
            json!({
                "name": o.name,
                "description": o.description(),
                "needs": o.plugin().map(|p| p.needs).unwrap_or_default(),
            })
        })
        .collect();
    Ok(json!({
        "action": "list",
        "version": env!("CARGO_PKG_VERSION"),
        "plugins": plugins,
        "unplugged": unplugged,
        "available": available,
    }))
}

/// Print `rows` (name, description, marks), the names in one column.
fn print_rows(rows: &[(String, String, Vec<String>)]) {
    let width = rows.iter().map(|r| r.0.len()).max().unwrap_or(0);
    for (name, description, marks) in rows {
        let marks = if marks.is_empty() {
            String::new()
        } else {
            format!(" [{}]", marks.join(", "))
        };
        println!("  {name:width$}  {description}{marks}");
    }
}

/// The official plugins the project does not have, as `decree list` prints them; `init`
/// prints them too.
pub fn print_available(decree_dir: &Path) -> Result<(), DecreeError> {
    let doc = listing(decree_dir)?;
    print_available_from(&doc);
    Ok(())
}

fn print_available_from(doc: &Json) {
    let rows: Vec<(String, String, Vec<String>)> = doc["available"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            (
                p["name"].as_str().unwrap_or("").to_string(),
                p["description"].as_str().unwrap_or("").to_string(),
                Vec::new(),
            )
        })
        .collect();
    if rows.is_empty() {
        println!(
            "Every official plugin built into decree {} is in {DECREE_DIR}/{PLUGINS_DIR}/.",
            env!("CARGO_PKG_VERSION")
        );
        return;
    }
    println!(
        "Official plugins to install (built into decree {}; `decree install <name>`):",
        env!("CARGO_PKG_VERSION")
    );
    print_rows(&rows);
}

/// `decree list`: the plugins in `plugins/`, each marked `official` when decree ships one of
/// that name (and `differs` when the file is not decree's copy) or `own`, and with its install
/// state when it is not `installed`; the machines no plugin installed; then the official
/// plugins the project does not have.
pub fn list(project_root: &Path, format: Format) -> Result<(), DecreeError> {
    let decree_dir = project_root.join(DECREE_DIR);
    let doc = listing(&decree_dir)?;
    if format == Format::Json {
        return print_json(&doc);
    }
    let rows: Vec<(String, String, Vec<String>)> = doc["plugins"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            let mut marks = vec![if p["official"] == true {
                "official"
            } else {
                "own"
            }
            .to_string()];
            if p["differs"] == true {
                marks.push("differs".into());
            }
            let state = p["state"].as_str().unwrap_or("");
            if state != "installed" {
                marks.push(state.to_string());
            }
            (
                p["name"].as_str().unwrap_or("").to_string(),
                p["description"].as_str().unwrap_or("").to_string(),
                marks,
            )
        })
        .collect();
    if rows.is_empty() {
        println!("No plugins in {DECREE_DIR}/{PLUGINS_DIR}/.");
    } else {
        println!("Plugins in {DECREE_DIR}/{PLUGINS_DIR}/:");
        print_rows(&rows);
    }
    let unplugged: Vec<&str> = doc["unplugged"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Json::as_str)
        .collect();
    if !unplugged.is_empty() {
        println!("Machines no plugin installed (`decree export <name>` writes one):");
        for name in unplugged {
            println!("  {name}");
        }
    }
    println!();
    print_available_from(&doc);
    let marks: Vec<&str> = doc["plugins"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|p| {
            [
                (p["differs"] == true).then_some("differs"),
                p["state"].as_str().filter(|s| *s != "installed"),
            ]
        })
        .flatten()
        .collect();
    let mut notes = Vec::new();
    if marks.contains(&"differs") {
        notes.push("`differs`: not this decree's copy; `decree install --force <name>` takes it, printing the edits it drops.");
    }
    if marks.contains(&"not installed") {
        notes.push("`not installed`: new or changed since it was installed; run `decree install`.");
    }
    if marks.contains(&"edited") {
        notes.push("`edited`: the installed copy was edited in place; `decree export <name>` keeps the edit, `decree install <name>` drops it.");
    }
    if marks.contains(&"conflict") {
        notes.push("`conflict`: changed while its installed copy was edited; `decree check` says how to merge.");
    }
    if marks.contains(&"invalid") {
        notes.push("`invalid`: fails the plugin schema; `decree check` names the problem.");
    }
    if !notes.is_empty() {
        println!();
        for n in notes {
            println!("{n}");
        }
    }
    Ok(())
}

/// What `decree install <name>` did with one plugin.
struct Installed {
    name: String,
    /// `installed` (from its file in `plugins/`), `added` (seeded from the official copy),
    /// `needed` (seeded because a plugin needs it) or `replaced` (--force: the file replaced
    /// with the official copy).
    status: &'static str,
    /// For `needed`: the plugins that need it.
    needed_by: Vec<String>,
    plugin: Plugin,
    text: Vec<u8>,
    /// For `replaced`: each file added, changed or removed, with the lines it drops and adds.
    changes: Vec<Json>,
    /// For `replaced`: whether the installed copy had edits in place that were not exported.
    dropped_edits_in_place: bool,
}

/// `decree install <name>...`: install each named plugin from `plugins/<name>.md`, replacing an
/// installed copy edited in place. A name `plugins/` lacks is seeded from the official plugin of
/// that name, and so is each official plugin a seeded or installed one needs that `plugins/`
/// lacks; `force` replaces a file in `plugins/` with the official copy, printing what changes.
/// A name that is neither, or a plugin file that fails the schema, fails before anything is
/// written.
fn install_named(
    project_root: &Path,
    names: &[String],
    force: bool,
    format: Format,
) -> Result<(), DecreeError> {
    let decree_dir = project_root.join(DECREE_DIR);
    let present = |name: &str| decree_dir.join(plugin::plugin_file(name)).is_file();
    let unknown: Vec<&String> = names
        .iter()
        .filter(|n| official::get(n).is_none() && !present(n))
        .collect();
    if !unknown.is_empty() {
        let list = unknown
            .iter()
            .map(|n| format!("`{n}`"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(DecreeError::Other(format!(
            "no plugin {list}: not in {DECREE_DIR}/{PLUGINS_DIR}/ and not an official plugin; nothing was installed. The official plugins are: {}",
            official::names().join(", ")
        )));
    }

    // The named plugins first, then each need not yet in plugins/, breadth first.
    let mut plan: Vec<Installed> = Vec::new();
    let mut problems = Vec::new();
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
        let o = official::get(&name);
        let status = match (needed_by.is_some(), present(&name), force && o.is_some()) {
            // A need already in plugins/ is the project's: left as it is, without a word.
            (true, true, _) => continue,
            (true, false, _) => "needed",
            (false, true, false) => "installed",
            (false, true, true) => "replaced",
            (false, false, _) => "added",
        };
        let (plugin, text) = if status == "installed" {
            let found = plugin::read_one(&decree_dir, &name)?;
            match found.parsed {
                Ok(p) => (p, found.text),
                Err(ps) => {
                    let file = plugin::plugin_file(&name);
                    problems.extend(ps.into_iter().map(|p| match p.line {
                        Some(line) => format!("{file}: line {line}: {} (P1)", p.message),
                        None => format!("{file}: {} (P1)", p.message),
                    }));
                    continue;
                }
            }
        } else {
            let Some(o) = o else {
                return Err(DecreeError::Other(format!(
                    "plugin `{}` needs `{name}`, which is not in {DECREE_DIR}/{PLUGINS_DIR}/ and not an official plugin: install it first (`decree install <file>`); nothing was installed",
                    needed_by.unwrap_or_default()
                )));
            };
            (
                o.plugin().map_err(DecreeError::Other)?,
                o.text.as_bytes().to_vec(),
            )
        };
        if machine_without_plugin(&decree_dir, &name) && status != "replaced" {
            return Err(DecreeError::Other(format!(
                "{DECREE_DIR}/{} exists and no plugin installed it; export it (`decree export {name}`) or move it, then install `{name}` again; nothing was installed",
                crate::layout::machine_file_rel(&name)
            )));
        }
        for need in &plugin.needs {
            queue.push((need.clone(), Some(name.clone())));
        }
        plan.push(Installed {
            name,
            status,
            needed_by: needed_by.into_iter().collect(),
            plugin,
            text,
            changes: Vec::new(),
            dropped_edits_in_place: false,
        });
    }
    if !problems.is_empty() {
        return Err(DecreeError::Other(format!(
            "{}\nnothing was installed",
            problems.join("\n")
        )));
    }

    for a in plan.iter_mut() {
        if a.status == "installed" {
            a.dropped_edits_in_place = plugin::edited_since_install(&decree_dir, &a.name)?;
        }
        if a.status == "replaced" {
            let old = plugin::read_one(&decree_dir, &a.name)?;
            let old_plugin = old.parsed.as_ref().ok();
            a.changes = changes(old_plugin, &a.plugin, true);
            a.dropped_edits_in_place = match old_plugin {
                Some(p) => plugin::edited(&decree_dir, p)?,
                None => false,
            };
        }
        if a.status != "installed" {
            let target = decree_dir.join(plugin::plugin_file(&a.name));
            if let Some(dir) = target.parent() {
                std::fs::create_dir_all(dir)
                    .map_err(|e| DecreeError::Other(format!("{}: {e}", dir.display())))?;
            }
            crate::message::write_replace(&target, &a.text)
                .map_err(|(path, e)| DecreeError::Other(format!("{}: {e}", path.display())))?;
        }
        plugin::install(&decree_dir, &a.plugin, &a.text)?;
    }
    // Draw the machines installed, so their graphs stay current. The plugins are installed
    // either way: a project whose own machines do not load yet is told, not refused.
    if let Err(e) = crate::commands::graph::write(project_root) {
        eprintln!("warning: graphs not redrawn ({e}); run `decree check`, then `decree graph`");
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
            print_json(&json!({ "action": "install", "plugins": plugins }))
        }
        Format::Text => {
            for a in &plan {
                let file = format!("{DECREE_DIR}/{}", plugin::plugin_file(&a.name));
                match a.status {
                    "installed" if a.dropped_edits_in_place => println!(
                        "installed {file}, dropping the edits in place to {DECREE_DIR}/{MACHINES_DIR}/{0}/ (machine.yml, scripts/ or cron/), which were not exported",
                        a.name
                    ),
                    "installed" => {}
                    "added" => println!("added {file}"),
                    "needed" => println!("added {file}, needed by {}", a.needed_by.join(", ")),
                    _ => {
                        println!("replaced {file} with this decree's copy:");
                        print_line_changes(&a.changes);
                        if a.dropped_edits_in_place {
                            println!(
                                "  and dropped the edits in place to {DECREE_DIR}/{MACHINES_DIR}/{0}/ (machine.yml, scripts/ or cron/), which were not exported",
                                a.name
                            );
                        }
                    }
                }
            }
            let names: Vec<&str> = plan.iter().map(|a| a.name.as_str()).collect();
            println!("installed {}", names.join(", "));
            Ok(())
        }
    }
}

/// Whether `machines/<name>/machine.yml` exists and no plugin file names it.
fn machine_without_plugin(decree_dir: &Path, name: &str) -> bool {
    decree_dir
        .join(crate::layout::machine_file_rel(name))
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
        assert_eq!(
            changes[0]["path"],
            ".decree/machines/develop/scripts/gate.sh"
        );
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
        let gate = decree_dir.join("machines/develop/scripts/gate.sh");
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
