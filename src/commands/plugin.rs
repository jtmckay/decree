//! `decree install` and `decree list|export` (docs/reference/plugins.md): share a
//! machine as one file, `.decree/plugins/<name>.md`, committed; what it installs into
//! `.decree/machines/<name>/` is ignored by git. `decree install` with no argument installs every
//! plugin as its file says (the step after a checkout or an upgrade, which also makes every
//! official plugin this decree's copy); `decree install <name>` installs that plugin, seeding
//! its file from the official copy built into decree when the project has none;
//! `decree install <file>`, or `-` for stdin, copies it into `plugins/` and installs it.
//! decree fetches only with `decree install --update`, from a plugin's `source`, and writes
//! what it fetched only after a person says `y`. `list` shows the plugins, and `export`
//! writes a plugin file from the installed copy.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, IsTerminal, Read, Write};
use std::path::Path;

use serde_json::{json, Value as Json};

use crate::cli::Format;
use crate::commands::print_json;
use crate::error::DecreeError;
use crate::layout::DECREE_DIR;
use crate::official;
use crate::plugin::{self, Planned, Plugin, OFFICIAL_PREFIX, PLUGINS_DIR};

/// Whether `name` is an official plugin's: it has the prefix (docs/reference/plugins.md,
/// Official plugins).
pub fn is_official(name: &str) -> bool {
    name.starts_with(OFFICIAL_PREFIX)
}

/// How to change what official plugin `name` does: never by editing it.
pub fn overlay_advice(name: &str) -> String {
    format!(
        "official plugins are never edited in a project: `decree install` restores this decree's copy (archiving the change); put the change in a plugin of the project's own that extends it (`name: <your name>`, `extends: {name}`), holding only the files it changes"
    )
}

fn io_other(path: &Path, e: std::io::Error) -> DecreeError {
    DecreeError::Other(format!("{}: {e}", path.display()))
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), DecreeError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| io_other(dir, e))?;
    }
    crate::message::write_replace(path, bytes).map_err(|(path, e)| io_other(&path, e))
}

/// `decree export <name>`: write `plugins/<name>.md` from the installed copy. An official
/// plugin is never exported: a project changes one with a plugin that extends it.
pub fn export(project_root: &Path, name: &str, format: Format) -> Result<(), DecreeError> {
    if is_official(name) {
        return Err(DecreeError::Other(format!(
            "`{name}` is an official plugin: {}; nothing was written",
            overlay_advice(name)
        )));
    }
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

/// An official plugin `closure` seeds: its name, and the plugins that need it.
type Seeded = (String, Vec<String>);

/// The plugins `roots` rely on that `plugins/` lacks (each `extends` base and each plugin
/// that meets a need), seeded from decree's official copies with what they rely on in turn:
/// (name, the plugins that need it). `Err` lists each that is neither in `plugins/` nor
/// official, with what needs it.
fn closure(
    roots: &[Plugin],
    present: &dyn Fn(&str) -> bool,
) -> Result<Vec<Seeded>, Vec<(String, String)>> {
    fn wants(p: &Plugin) -> Vec<String> {
        let mut out: Vec<String> = p.extends.iter().cloned().collect();
        for need in &p.needs {
            out.push(p.need_map.get(need).unwrap_or(need).clone());
        }
        out
    }
    let root_names: BTreeSet<&str> = roots.iter().map(|p| p.name.as_str()).collect();
    let mut seed: Vec<(String, Vec<String>)> = Vec::new();
    let mut missing: Vec<(String, String)> = Vec::new();
    let mut queue: Vec<(String, String)> = roots
        .iter()
        .flat_map(|p| wants(p).into_iter().map(|n| (n, p.name.clone())))
        .collect();
    let mut i = 0;
    while i < queue.len() {
        let (n, by) = queue[i].clone();
        i += 1;
        if root_names.contains(n.as_str()) || present(&n) {
            continue;
        }
        if let Some(entry) = seed.iter_mut().find(|(s, _)| *s == n) {
            if !entry.1.contains(&by) {
                entry.1.push(by);
            }
            continue;
        }
        match official::get(&n).map(|o| o.plugin()) {
            Some(Ok(p)) => {
                queue.extend(wants(&p).into_iter().map(|w| (w, n.clone())));
                seed.push((n, vec![by]));
            }
            _ => {
                if !missing.iter().any(|(m, _)| *m == n) {
                    missing.push((n, by));
                }
            }
        }
    }
    if missing.is_empty() {
        Ok(seed)
    } else {
        missing.sort();
        Err(missing)
    }
}

/// The message for what `closure` found missing.
fn missing_message(missing: &[(String, String)]) -> String {
    let lines: Vec<String> = missing
        .iter()
        .map(|(n, by)| {
            format!(
                "plugin `{by}` needs `{n}`, which is not in {DECREE_DIR}/{PLUGINS_DIR}/ and not an official plugin"
            )
        })
        .collect();
    format!(
        "{}: install it first (`decree install <file>`); nothing was installed",
        lines.join("; ")
    )
}

/// The plan of `plugins/` with `writes` (name, bytes) in place of, or beside, its files:
/// what installing would do, before anything is written.
fn simulate(decree_dir: &Path, writes: &[(String, Vec<u8>)]) -> Result<Vec<Planned>, DecreeError> {
    let mut found = plugin::read_all(decree_dir)?;
    for (name, bytes) in writes {
        let parsed = match std::str::from_utf8(bytes) {
            Ok(t) => Plugin::parse(t, Some(name)),
            Err(_) => Err(vec![]),
        };
        let f = plugin::Found {
            name: name.clone(),
            text: bytes.clone(),
            parsed,
        };
        match found.iter_mut().find(|x| x.name == *name) {
            Some(x) => *x = f,
            None => found.push(f),
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(plugin::plan_found(found))
}

/// The errors that keep `names` from installing in `planned`: P1 (the schema, or the chain
/// of `extends`) and a base not in `plugins/` (P2).
fn plan_errors(planned: &[Planned], names: &BTreeSet<String>) -> Vec<String> {
    let mut out = Vec::new();
    for p in planned.iter().filter(|p| names.contains(&p.name)) {
        let file = plugin::plugin_file(&p.name);
        match (&p.parsed, &p.target) {
            (Err(ps), _) => out.extend(ps.iter().map(|q| match q.line {
                Some(line) => format!("{file}: line {line}: {} (P1)", q.message),
                None => format!("{file}: {} (P1)", q.message),
            })),
            (Ok(_), None) => out.push(format!(
                "{file}: `extends` names {}, not in {PLUGINS_DIR}/ (P2)",
                p.missing
                    .iter()
                    .map(|m| format!("`{m}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            _ => {}
        }
    }
    out
}

/// `names`, with every plugin that extends one of them (installed again on the new base)
/// and every base of those (installed with or without its cron files, as they are extended).
fn affected(planned: &[Planned], names: &BTreeSet<String>) -> BTreeSet<String> {
    let mut out = names.clone();
    for n in names {
        out.extend(plugin::extenders(planned, n));
    }
    let mut bases = BTreeSet::new();
    for n in &out {
        let mut base = planned
            .iter()
            .find(|p| p.name == *n)
            .and_then(|p| p.parsed.as_ref().ok())
            .and_then(|p| p.extends.clone());
        while let Some(b) = base {
            if !bases.insert(b.clone()) {
                break;
            }
            base = planned
                .iter()
                .find(|p| p.name == b)
                .and_then(|p| p.parsed.as_ref().ok())
                .and_then(|p| p.extends.clone());
        }
    }
    out.extend(bases);
    out
}

/// Install each plugin of `plan` that `names` holds, as `plan` resolves it, archiving what it
/// would lose and, from `replaced`, the plugin file it replaces. Returns each name with its
/// archive, in plan order.
fn install_set(
    decree_dir: &Path,
    plan: &[Planned],
    names: &BTreeSet<String>,
    replaced: &BTreeMap<String, Vec<u8>>,
    cron: bool,
) -> Result<Vec<(String, Option<String>)>, DecreeError> {
    let mut out = Vec::new();
    for p in plan.iter().filter(|p| names.contains(&p.name)) {
        let Some(target) = &p.target else { continue };
        let archived = plugin::install_archiving(
            decree_dir,
            target,
            &p.text,
            replaced.get(&p.name).map(Vec::as_slice),
            cron,
        )?;
        out.push((p.name.clone(), archived));
    }
    Ok(out)
}

/// Draw the machines installed, so their graphs stay current. The plugins are installed
/// either way: a project whose own machines do not load yet is told, not refused.
fn redraw(decree_dir: &Path) {
    let Some(project_root) = decree_dir.parent() else {
        return;
    };
    if let Err(e) = crate::commands::graph::write(project_root) {
        eprintln!("warning: graphs not redrawn ({e}); run `decree check`, then `decree graph`");
    }
}

/// `decree install <file|->`: check the plugin, copy it to `plugins/<name>.md` and install
/// it, seeding each official plugin it extends or needs that `plugins/` lacks. A plugin file
/// of that name is replaced, and it and an installed copy that installing would lose are
/// moved to `archive/` first. Refuses, writing nothing, a plugin that fails the schema (a
/// `decree_` name decree does not ship is one), an official plugin that is not this decree's
/// copy (P3), or one that relies on a plugin neither in `plugins/` nor official.
fn import(
    decree_dir: &Path,
    shown: &str,
    bytes: Vec<u8>,
    cron: bool,
    format: Format,
) -> Result<(), DecreeError> {
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| DecreeError::Other(format!("{shown}: not valid UTF-8")))?;
    let p1 = |problems: Vec<plugin::Problem>| {
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
    };
    let plugin = Plugin::parse(text, None).map_err(p1)?;
    let name = plugin.name.clone();
    if let Some(o) = official::get(&name) {
        if o.text.as_bytes() != bytes.as_slice() {
            return Err(DecreeError::Other(format!(
                "{shown}: `{name}` is an official plugin, and this is not this decree's copy of it (P3): {}; nothing was installed",
                overlay_advice(&name)
            )));
        }
    }
    let present = |n: &str| decree_dir.join(plugin::plugin_file(n)).is_file();
    let seed = closure(std::slice::from_ref(&plugin), &present)
        .map_err(|m| DecreeError::Other(missing_message(&m)))?;

    let target = decree_dir.join(plugin::plugin_file(&name));
    let old = if target.is_file() {
        let found = plugin::read_one(decree_dir, &name)?;
        Some((found.parsed.ok(), found.text))
    } else {
        None
    };
    let mut writes = vec![(name.clone(), bytes.clone())];
    for (n, _) in &seed {
        if let Some(o) = official::get(n) {
            writes.push((n.clone(), o.text.as_bytes().to_vec()));
        }
    }
    let plan = simulate(decree_dir, &writes)?;
    let mut names: BTreeSet<String> = writes.iter().map(|(n, _)| n.clone()).collect();
    let errors = plan_errors(&plan, &names);
    if !errors.is_empty() {
        return Err(DecreeError::Other(format!(
            "{}\nnothing was installed",
            errors.join("\n")
        )));
    }
    names = affected(&plan, &names);

    let changes = old
        .as_ref()
        .map(|(old, _)| changes(old.as_ref(), &plugin, false));
    let summary = summary(&plugin);
    let file = format!("{DECREE_DIR}/{}", plugin::plugin_file(&name));
    if format == Format::Text {
        println!("plugin `{name}`: {}", described(&plan, &name, &plugin));
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
    let mut replaced = BTreeMap::new();
    if let Some((_, text)) = &old {
        replaced.insert(name.clone(), text.clone());
    }
    let installed = install_set(decree_dir, &plan, &names, &replaced, cron)?;
    for (n, b) in &writes {
        write_file(&decree_dir.join(plugin::plugin_file(n)), b)?;
    }
    redraw(decree_dir);
    if format == Format::Text {
        for (n, by) in &seed {
            println!(
                "added {DECREE_DIR}/{}, needed by {}",
                plugin::plugin_file(n),
                by.join(", ")
            );
        }
    }
    let archived = installed
        .iter()
        .find(|(n, _)| *n == name)
        .and_then(|(_, a)| a.clone());

    match format {
        Format::Json => {
            let mut doc = json!({
                "action": "import",
                "plugin": name,
                "file": file,
                "description": described(&plan, &name, &plugin),
                "replaced": changes.is_some(),
                "changes": changes.unwrap_or_default(),
                "added": seed.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>(),
                "archived": archived.as_ref().map(|d| format!("{DECREE_DIR}/{d}")),
            });
            merge(&mut doc, summary);
            print_json(&doc)
        }
        Format::Text => {
            for (n, dir) in &installed {
                if let Some(dir) = dir {
                    println!("{}", archived_line(n, dir));
                }
            }
            println!("installed {file}");
            Ok(())
        }
    }
}

/// The one-line description of plugin `name`: its resolved machine's, so a plugin that
/// extends another without a machine of its own shows its base's.
fn described(plan: &[Planned], name: &str, plugin: &Plugin) -> String {
    plan.iter()
        .find(|p| p.name == name)
        .and_then(|p| p.target.as_ref())
        .map_or_else(
            || plugin.description().trim().to_string(),
            |t| t.plugin.description().trim().to_string(),
        )
}

/// `decree install [<name>...|<file>|-]` (docs/reference/plugins.md, Installing). No argument:
/// every plugin is installed as its file says (`plugin::sync`), every official plugin's file
/// and installed copy made this decree's first; stdin is never read, so a script or a loop can
/// run it safely. A name is a plugin, installed from `plugins/<name>.md`, or from decree's
/// official copy when `plugins/` has none. Anything else is a plugin file, or `-` for stdin,
/// imported alone. Nothing an install replaces is lost: what it would lose is moved to
/// `.decree/archive/` first.
///
/// An install that succeeds, or one with no argument that installed what it could, also
/// writes `.decree/schema/`, quietly (it is generated, like a graph), so the step after a clone, a pull or an upgrade leaves the
/// project's schemas this decree's.
pub fn install(
    project_root: &Path,
    sources: &[String],
    cron: bool,
    format: Format,
) -> Result<(), DecreeError> {
    let result = install_plugins(project_root, sources, cron, format);
    // With no argument, plugins that installed stay installed when another fails its schema,
    // so the schemas are written then too; a refused install by name or file writes nothing.
    if result.is_ok() || sources.is_empty() {
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
    cron: bool,
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
        return install_all(&decree_dir, cron, format);
    }
    if sources.iter().all(|s| crate::machine::is_ident(s)) {
        return install_named(project_root, sources, cron, format);
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
    import(&decree_dir, &shown, bytes, cron, format)
}

/// What `decree install` restored of an official plugin, with the lines that change, as the
/// JSON document gives it.
fn restored_json(decree_dir: &Path, r: &plugin::Restored) -> Result<Json, DecreeError> {
    let new = plugin::target(decree_dir, &r.name)?.map(|t| t.plugin);
    let new = match (r.what, new) {
        (plugin::RestoredWhat::Copy, Some(new)) => Some(new),
        _ => official::get(&r.name).and_then(|o| o.plugin().ok()),
    };
    let changes = match (r.what, &new) {
        (plugin::RestoredWhat::File, _) => file_changes(&r.name, &r.old_text),
        (_, Some(new)) => changes(r.old.as_ref(), new, true),
        _ => Vec::new(),
    };
    Ok(json!({
        "plugin": r.name,
        "what": match r.what {
            plugin::RestoredWhat::File => "file",
            plugin::RestoredWhat::Copy => "installed copy",
        },
        "file": format!("{DECREE_DIR}/{}", plugin::plugin_file(&r.name)),
        "changes": changes,
        "archived": format!("{DECREE_DIR}/{}", r.archive),
    }))
}

/// `decree install` with no argument: `plugin::sync`, taking the cron files of plugins
/// already installed only with `cron`.
fn install_all(decree_dir: &Path, cron: bool, format: Format) -> Result<(), DecreeError> {
    let synced = plugin::sync(decree_dir, cron)?;
    let restored: Vec<Json> = synced
        .restored
        .iter()
        .map(|r| restored_json(decree_dir, r))
        .collect::<Result<_, _>>()?;
    match format {
        Format::Json => print_json(&json!({
            "action": "sync",
            "installed": synced.installed,
            "removed": synced.removed,
            "archived": synced
                .archived
                .iter()
                .map(|(name, dir)| json!({ "plugin": name, "archive": format!("{DECREE_DIR}/{dir}") }))
                .collect::<Vec<_>>(),
            "restored": restored,
        }))?,
        Format::Text => {
            for r in &restored {
                let file = r["file"].as_str().unwrap_or("");
                if r["what"] == "file" {
                    println!("replaced {file} with this decree's copy:");
                } else {
                    println!(
                        "restored the installed copy of {file} to this decree's (official plugins are not edited):"
                    );
                }
                print_line_changes(r["changes"].as_array().map_or(&[][..], Vec::as_slice));
                let name = r["plugin"].as_str().unwrap_or("");
                let archived = r["archived"].as_str().unwrap_or("");
                let rel = archived
                    .strip_prefix(&format!("{DECREE_DIR}/"))
                    .unwrap_or(archived);
                println!("{}", archived_line(name, rel));
            }
            let restored_copies: BTreeSet<&str> = synced
                .restored
                .iter()
                .filter(|r| r.what == plugin::RestoredWhat::Copy)
                .map(|r| r.name.as_str())
                .collect();
            for (name, dir) in &synced.archived {
                if !restored_copies.contains(name.as_str()) {
                    println!("{}", archived_line(name, dir));
                }
            }
            for name in &synced.removed {
                println!(
                    "removed the installed copy of {DECREE_DIR}/{}, which was deleted",
                    plugin::plugin_file(name)
                );
            }
            for name in &synced.installed {
                println!("installed {DECREE_DIR}/{}", plugin::plugin_file(name));
            }
            if synced.installed.is_empty()
                && synced.removed.is_empty()
                && synced.invalid.is_empty()
                && synced.missing_needs.is_empty()
                && synced.missing_bases.is_empty()
            {
                println!("no plugins in {DECREE_DIR}/{PLUGINS_DIR}/: nothing to install");
            }
        }
    }
    for note in synced.notices() {
        eprintln!("{note}");
    }
    if !synced.invalid.is_empty() {
        return Err(DecreeError::Other(format!(
            "{} plugin(s) not installed; see above",
            synced.invalid.len()
        )));
    }
    Ok(())
}

/// The line an install prints for what it archived to `path`, relative to `.decree/`: a
/// plugin file, which `decree install <file>` restores, or a folder of the copy as it was.
fn archived_line(name: &str, path: &str) -> String {
    if path.ends_with(".md") {
        format!(
            "archived what installing `{name}` replaced as {DECREE_DIR}/{path}; `decree install {DECREE_DIR}/{path}` restores it"
        )
    } else {
        format!(
            "archived the installed copy of `{name}` as it was, to {DECREE_DIR}/{path}/ (it cannot be a plugin file: its machine.yml is not a YAML mapping, or a file is not text)"
        )
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

/// `decree export` with no name: export every plugin whose installed copy differs from
/// its plugin file (edited in place, or by a run), so the edits reach `plugins/`. `commit`
/// runs it before it commits. A plugin that fails the schema, or is not installed, is skipped,
/// and so is a conflict (its file changed since it was installed): exporting would write the
/// old installed version over the new file, so it is named and left for a person to merge.
/// An official plugin is never exported: one whose installed copy was edited is named, with
/// the plugin to write instead. With `dry_run`, it names them and writes nothing: `keep`
/// holds the queue on them.
pub fn export_edited(
    project_root: &Path,
    dry_run: bool,
    format: Format,
) -> Result<(), DecreeError> {
    let decree_dir = project_root.join(DECREE_DIR);
    let mut exported = Vec::new();
    let mut conflicts = Vec::new();
    let mut official_edited = Vec::new();
    for p in plugin::plan(&decree_dir)? {
        let Some(target) = &p.target else { continue };
        if !plugin::edited(&decree_dir, target)? {
            continue;
        }
        if is_official(&p.name) {
            official_edited.push(p.name);
            continue;
        }
        if plugin::conflicted(&decree_dir, &p.name, &target.record)? {
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
            "official": official_edited,
            "dry_run": dry_run,
        })),
        Format::Text => {
            for name in &official_edited {
                println!(
                    "not exported: the installed copy of official plugin `{name}` was edited; {}",
                    overlay_advice(name)
                );
            }
            for name in &conflicts {
                println!(
                    "not exported: {} changed since `{name}` was installed, and its installed copy was edited; merge by hand (`decree export {name}` writes the copy; `decree install {name}` takes the file, moving the copy to {DECREE_DIR}/{ARCHIVE}/{name}/)",
                    plugin::plugin_file(name),
                    ARCHIVE = plugin::ARCHIVE_DIR
                );
            }
            if exported.is_empty() && conflicts.is_empty() && official_edited.is_empty() {
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
    for p in plugin::plan(decree_dir)? {
        let official = is_official(&p.name);
        let differs = status.official_edited.iter().any(|(n, _, _)| *n == p.name);
        let (description, state, extends) = match &p.parsed {
            Err(_) => (String::new(), "invalid", None),
            Ok(parsed) => {
                let state = if p.target.is_none() {
                    "missing base"
                } else if status.archived.iter().any(|(n, _)| *n == p.name) {
                    "edited"
                } else if status.installed.contains(&p.name) {
                    "not installed"
                } else {
                    "installed"
                };
                let description = p.target.as_ref().map_or_else(
                    || parsed.description().trim().to_string(),
                    |t| t.plugin.description().trim().to_string(),
                );
                (description, state, parsed.extends.clone())
            }
        };
        plugins.push(json!({
            "name": p.name,
            "description": description,
            "official": official,
            "differs": differs,
            "extends": extends,
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

/// `decree list`: the plugins in `plugins/`, each marked `official` when its name has the
/// `decree_` prefix (and `differs` when its file or installed copy is not this decree's) or
/// `own`, `extends <base>` for one that extends another, and its install state when it is
/// not `installed`; the machines no plugin installed; then the official plugins the project
/// does not have.
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
            if let Some(base) = p["extends"].as_str() {
                marks.push(format!("extends {base}"));
            }
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
        notes.push("`differs`: an official plugin whose file or installed copy is not this decree's; `decree install` restores it, archiving the change. Official plugins are never edited: a plugin of your own that extends one holds your changes.");
    }
    if marks.contains(&"not installed") {
        notes.push("`not installed`: new or changed since it was installed; run `decree install`.");
    }
    if marks.contains(&"edited") {
        notes.push("`edited`: the installed copy was edited in place or written by hand; `decree export <name>` keeps the edit (an official plugin's goes in a plugin of your own that extends it), `decree install` archives it as a plugin file in .decree/archive/.");
    }
    if marks.contains(&"missing base") {
        notes.push(
            "`missing base`: it extends a plugin not in .decree/plugins/; `decree check` names it.",
        );
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

/// `decree install <name>...`: install each named plugin from `plugins/<name>.md`, moving an
/// installed copy edited in place, or a machine no plugin installed, to `archive/` first. A
/// name `plugins/` lacks is seeded from the official plugin of that name (only a `decree_`
/// name is one), and so is each official plugin a seeded or installed one extends or needs
/// that `plugins/` lacks. An official plugin's file that is not this decree's copy is replaced
/// with it, printing what changes and moving the old file to `archive/`. Each plugin that
/// extends one installed is installed again on it. A name that is neither, or a plugin file
/// that fails, fails before anything is written.
fn install_named(
    project_root: &Path,
    names: &[String],
    cron: bool,
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

    // (name, status, needed_by): `installed` from its file, `added` from the official copy,
    // `replaced` (an official file that was not this decree's copy), `needed`.
    let mut plan_rows: Vec<(String, &'static str, Vec<String>)> = Vec::new();
    let mut roots = Vec::new();
    let mut writes: Vec<(String, Vec<u8>)> = Vec::new();
    let mut replaced: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut problems = Vec::new();
    for name in names {
        if plan_rows.iter().any(|(n, _, _)| n == name) {
            continue;
        }
        let o = official::get(name);
        if present(name) {
            let found = plugin::read_one(&decree_dir, name)?;
            match (o, found.parsed) {
                (Some(o), _) if found.text != o.text.as_bytes() => {
                    replaced.insert(name.clone(), found.text);
                    writes.push((name.clone(), o.text.as_bytes().to_vec()));
                    roots.push(o.plugin().map_err(DecreeError::Other)?);
                    plan_rows.push((name.clone(), "replaced", Vec::new()));
                }
                (_, Ok(p)) => {
                    roots.push(p);
                    plan_rows.push((name.clone(), "installed", Vec::new()));
                }
                (_, Err(ps)) => {
                    let file = plugin::plugin_file(name);
                    problems.extend(ps.into_iter().map(|p| match p.line {
                        Some(line) => format!("{file}: line {line}: {} (P1)", p.message),
                        None => format!("{file}: {} (P1)", p.message),
                    }));
                }
            }
        } else if let Some(o) = o {
            writes.push((name.clone(), o.text.as_bytes().to_vec()));
            roots.push(o.plugin().map_err(DecreeError::Other)?);
            plan_rows.push((name.clone(), "added", Vec::new()));
        }
    }
    if !problems.is_empty() {
        return Err(DecreeError::Other(format!(
            "{}\nnothing was installed",
            problems.join("\n")
        )));
    }
    let seed = closure(&roots, &present).map_err(|m| DecreeError::Other(missing_message(&m)))?;
    for (n, by) in seed {
        if let Some(o) = official::get(&n) {
            writes.push((n.clone(), o.text.as_bytes().to_vec()));
        }
        plan_rows.push((n, "needed", by));
    }
    let plan = simulate(&decree_dir, &writes)?;
    let wanted: BTreeSet<String> = plan_rows.iter().map(|(n, _, _)| n.clone()).collect();
    let errors = plan_errors(&plan, &wanted);
    if !errors.is_empty() {
        return Err(DecreeError::Other(format!(
            "{}\nnothing was installed",
            errors.join("\n")
        )));
    }
    let set = affected(&plan, &wanted);
    let installed = install_set(&decree_dir, &plan, &set, &replaced, cron)?;
    for (n, b) in &writes {
        write_file(&decree_dir.join(plugin::plugin_file(n)), b)?;
    }
    redraw(&decree_dir);

    let archive_of = |n: &str| -> Option<String> {
        installed
            .iter()
            .find(|(m, _)| m == n)
            .and_then(|(_, a)| a.clone())
    };
    let change_rows: BTreeMap<String, Vec<Json>> = plan_rows
        .iter()
        .filter(|(_, status, _)| *status == "replaced")
        .map(|(n, _, _)| (n.clone(), file_changes(n, &replaced[n])))
        .collect();
    let also: Vec<&String> = installed
        .iter()
        .map(|(n, _)| n)
        .filter(|n| !wanted.contains(*n))
        .collect();
    match format {
        Format::Json => {
            let mut plugins: Vec<Json> = plan_rows
                .iter()
                .map(|(n, status, by)| {
                    json!({
                        "plugin": n,
                        "file": format!("{DECREE_DIR}/{}", plugin::plugin_file(n)),
                        "status": status,
                        "needed_by": by,
                        "changes": change_rows.get(n).cloned().unwrap_or_default(),
                        "archived": archive_of(n).map(|d| format!("{DECREE_DIR}/{d}")),
                    })
                })
                .collect();
            for n in &also {
                plugins.push(json!({
                    "plugin": n,
                    "file": format!("{DECREE_DIR}/{}", plugin::plugin_file(n)),
                    "status": "reinstalled",
                    "needed_by": [],
                    "changes": [],
                    "archived": archive_of(n).map(|d| format!("{DECREE_DIR}/{d}")),
                }));
            }
            print_json(&json!({ "action": "install", "plugins": plugins }))
        }
        Format::Text => {
            for (n, status, by) in &plan_rows {
                let file = format!("{DECREE_DIR}/{}", plugin::plugin_file(n));
                match *status {
                    "added" => println!("added {file}"),
                    "needed" => println!("added {file}, needed by {}", by.join(", ")),
                    "replaced" => {
                        println!("replaced {file} with this decree's copy:");
                        print_line_changes(&change_rows[n]);
                    }
                    _ => {}
                }
                if let Some(dir) = archive_of(n) {
                    println!("{}", archived_line(n, &dir));
                }
            }
            for n in &also {
                if let Some(dir) = archive_of(n) {
                    println!("{}", archived_line(n, &dir));
                }
            }
            let names: Vec<&str> = installed.iter().map(|(n, _)| n.as_str()).collect();
            println!("installed {}", names.join(", "));
            Ok(())
        }
    }
}

/// The program `decree install --update` fetches with: `DECREE_CURL`, else the system `curl`.
const CURL_VAR: &str = "DECREE_CURL";

/// What `--write-out` prints to stderr after a fetch: the URL the body came from, after
/// any redirect.
const EFFECTIVE_URL: &str = "decree-effective-url: ";

/// Fetch `url` with curl: HTTPS only, redirects only to HTTPS (and, checked after, only on
/// the same host), with a timeout. Returns the body, or curl's error.
fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let program = std::env::var_os(CURL_VAR)
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "curl".into());
    let out = std::process::Command::new(&program)
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--max-redirs",
            "5",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--connect-timeout",
            "15",
            "--max-time",
            "60",
            "--write-out",
            &format!("%{{stderr}}{EFFECTIVE_URL}%{{url_effective}}\n"),
            "--",
            url,
        ])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {}: {e}", program.to_string_lossy()))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    let effective = stderr
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix(EFFECTIVE_URL))
        .map(str::trim)
        .filter(|u| !u.is_empty());
    let error: Vec<&str> = stderr
        .lines()
        .filter(|l| !l.starts_with(EFFECTIVE_URL) && !l.trim().is_empty())
        .collect();
    if !out.status.success() {
        return Err(if error.is_empty() {
            format!("curl exited with {}", out.status)
        } else {
            error.join("; ")
        });
    }
    if let Some(effective) = effective {
        if !plugin::is_https_url(effective) || host(effective) != host(url) {
            return Err(format!(
                "redirected to {effective}: a plugin is fetched only from HTTPS on its source's host"
            ));
        }
    }
    Ok(out.stdout)
}

/// The host of an `https://` URL.
fn host(url: &str) -> &str {
    let rest = url.strip_prefix("https://").unwrap_or(url);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    authority.rsplit('@').next().unwrap_or(authority)
}

/// Every line of `a` and `b`, in order: `  ` common, `- ` only in `a`, `+ ` only in `b`.
fn diff_full(a: &str, b: &str) -> Vec<String> {
    let a: Vec<&str> = a.lines().collect();
    let b: Vec<&str> = b.lines().collect();
    let (n, m) = (a.len(), b.len());
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
            out.push(format!("  {}", a[i]));
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

/// `decree install --update [<name>...]` (docs/reference/plugins.md, `source`): fetch each
/// named plugin, or with no name each in `plugins/` with a `source`, from its source; refuse
/// one that fails the schema or whose `name` or `source` is not the file's; print the whole
/// diff against the committed file and ask before writing it. Only `y` on a terminal takes
/// it: the old file is archived, the new one written and installed, with every plugin that
/// extends it. Without a terminal it writes nothing. Exit 1 if a fetch failed or an update
/// was refused.
pub fn update(project_root: &Path, names: &[String], format: Format) -> Result<(), DecreeError> {
    let decree_dir = project_root.join(DECREE_DIR);
    let found = plugin::read_all(&decree_dir)?;
    let sourced = |f: &plugin::Found| -> Option<String> {
        f.parsed.as_ref().ok().and_then(|p| p.source.clone())
    };
    let mut todo: Vec<(String, String, Vec<u8>)> = Vec::new();
    if names.is_empty() {
        for f in &found {
            if let Some(url) = sourced(f) {
                todo.push((f.name.clone(), url, f.text.clone()));
            }
        }
    } else {
        let mut bad = Vec::new();
        for n in names {
            match found.iter().find(|f| f.name == *n) {
                Some(f) => match sourced(f) {
                    Some(url) => todo.push((n.clone(), url, f.text.clone())),
                    None => bad.push(format!(
                        "{DECREE_DIR}/{} has no `source`: nothing updates it",
                        plugin::plugin_file(n)
                    )),
                },
                None => bad.push(format!("no plugin `{n}` in {DECREE_DIR}/{PLUGINS_DIR}/")),
            }
        }
        if !bad.is_empty() {
            return Err(DecreeError::Other(format!(
                "{}; nothing was updated",
                bad.join("; ")
            )));
        }
    }
    let terminal = std::io::stdin().is_terminal();
    let mut results = Vec::new();
    let mut failed = 0;
    for (name, url, old) in todo {
        let file = format!("{DECREE_DIR}/{}", plugin::plugin_file(&name));
        let mut result = json!({ "plugin": name, "file": file, "source": url });
        let refuse = |why: String| -> Json { json!(why) };
        let fetched = match fetch(&url) {
            Ok(bytes) => bytes,
            Err(e) => {
                failed += 1;
                result["status"] = "failed".into();
                result["error"] = refuse(format!("fetching {url}: {e}"));
                if format == Format::Text {
                    eprintln!("{file}: fetching {url} failed: {e}; left as it is");
                }
                results.push(result);
                continue;
            }
        };
        let problem = match std::str::from_utf8(&fetched) {
            Err(_) => Some("it is not valid UTF-8".to_string()),
            Ok(text) => match Plugin::parse(text, None) {
                Err(ps) => Some(format!(
                    "it fails the plugin schema: {}",
                    ps.iter()
                        .map(|p| match p.line {
                            Some(l) => format!("line {l}: {}", p.message),
                            None => p.message.clone(),
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                )),
                Ok(p) if p.name != name => {
                    Some(format!("its `name` is `{}`, not `{name}`", p.name))
                }
                Ok(p) if p.source.as_deref() != Some(url.as_str()) => Some(format!(
                    "its `source` is {}, not `{url}`",
                    p.source
                        .as_deref()
                        .map_or_else(|| "missing".to_string(), |s| format!("`{s}`"))
                )),
                Ok(_) => None,
            },
        };
        if let Some(problem) = problem {
            failed += 1;
            result["status"] = "refused".into();
            result["error"] = refuse(problem.clone());
            if format == Format::Text {
                eprintln!("{file}: refused the copy at {url}: {problem}; nothing was written");
            }
            results.push(result);
            continue;
        }
        if fetched == old {
            result["status"] = "up to date".into();
            if format == Format::Text {
                println!("{file}: up to date");
            }
            results.push(result);
            continue;
        }
        let diff = diff_full(
            &String::from_utf8_lossy(&old),
            &String::from_utf8_lossy(&fetched),
        );
        // The diff is always shown on the terminal a person answers on.
        println!("--- {file}");
        println!("+++ {url}");
        for line in &diff {
            println!("{line}");
        }
        if !terminal {
            result["status"] = "not updated".into();
            println!("not updated: run decree install --update {name} in a terminal to review it");
            results.push(result);
            continue;
        }
        print!("Take this update to {file}? [y/N] ");
        std::io::stdout()
            .flush()
            .map_err(|e| DecreeError::Other(format!("stdout: {e}")))?;
        let mut answer = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut answer)
            .map_err(|e| DecreeError::Other(format!("cannot read stdin: {e}")))?;
        if !matches!(answer.trim(), "y" | "Y" | "yes" | "Yes") {
            result["status"] = "not updated".into();
            println!("not updated: {file} is unchanged");
            results.push(result);
            continue;
        }
        let writes = vec![(name.clone(), fetched.clone())];
        let plan = simulate(&decree_dir, &writes)?;
        let one: BTreeSet<String> = [name.clone()].into();
        let errors = plan_errors(&plan, &one);
        if !errors.is_empty() {
            failed += 1;
            result["status"] = "refused".into();
            result["error"] = refuse(errors.join("; "));
            eprintln!("{}; nothing was written", errors.join("\n"));
            results.push(result);
            continue;
        }
        let set = affected(&plan, &one);
        let mut replaced = BTreeMap::new();
        replaced.insert(name.clone(), old.clone());
        let installed = install_set(&decree_dir, &plan, &set, &replaced, false)?;
        write_file(&decree_dir.join(plugin::plugin_file(&name)), &fetched)?;
        redraw(&decree_dir);
        for (n, dir) in &installed {
            if let Some(dir) = dir {
                println!("{}", archived_line(n, dir));
            }
        }
        let names: Vec<&str> = installed.iter().map(|(n, _)| n.as_str()).collect();
        println!("updated {file}; installed {}", names.join(", "));
        result["status"] = "updated".into();
        results.push(result);
    }
    if format == Format::Json {
        print_json(&json!({ "action": "update", "plugins": results }))?;
    } else if results.is_empty() {
        println!("no plugin in {DECREE_DIR}/{PLUGINS_DIR}/ has a `source`: nothing to update");
    }
    if failed > 0 {
        return Err(DecreeError::Other(format!(
            "{failed} plugin(s) not updated: a fetch failed or the copy was refused; see above"
        )));
    }
    Ok(())
}

/// What replacing official plugin `name`'s file, `old`, with this decree's copy changes: the
/// file, with the lines it drops and adds (its body and frontmatter alike).
fn file_changes(name: &str, old: &[u8]) -> Vec<Json> {
    let new = official::get(name).map_or("", |o| o.text);
    let old = String::from_utf8_lossy(old);
    if old == new {
        return Vec::new();
    }
    vec![json!({
        "path": format!("{DECREE_DIR}/{}", plugin::plugin_file(name)),
        "change": "changed",
        "lines": diff_lines(&old, new),
    })]
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
    let mut old_needs = old.map(|o| o.needs.clone()).unwrap_or_default();
    let mut new_needs = new.needs.clone();
    old_needs.sort();
    new_needs.sort();
    // Only a need added or dropped is a change; their order is not.
    if old_needs != new_needs {
        let mut entry = json!({ "path": "needs", "change": "changed" });
        if lines {
            entry["lines"] = json!([
                format!("- {}", old_needs.join(", ")),
                format!("+ {}", new_needs.join(", "))
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
        let text = official::get("decree_develop").unwrap().text;
        let new = Plugin::parse(text, None).unwrap();
        let mut old = new.clone();
        old.scripts
            .insert("gate.sh".into(), "#!/usr/bin/env bash\ncargo test\n".into());
        old.needs.clear();
        let changes = changes(Some(&old), &new, true);
        assert_eq!(changes.len(), 2, "{changes:?}");
        assert_eq!(
            changes[0]["path"],
            ".decree/machines/decree_develop/scripts/gate.sh"
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
        // A plugin of the project's own: an official one is never exported.
        let text = "---\nname: mine\nmachine:\n  description: Mine.\n  initial: gate\n  states:\n    gate:\n      invoke: gate\n      transitions:\n        done: done\n    done:\n      final: true\nscripts:\n  gate.sh: |\n    #!/usr/bin/env bash\n    true\n---\n# mine\n";
        let file = plugins.join("mine.md");
        std::fs::write(&file, text).unwrap();
        plugin::sync(&decree_dir, false).unwrap();
        let gate = decree_dir.join("machines/mine/scripts/gate.sh");
        std::fs::write(&gate, "#!/usr/bin/env bash\ncargo test\n").unwrap();
        // A pull changes the plugin file under the edit.
        let pulled = format!("{text}\nA teammate's note.\n");
        std::fs::write(&file, &pulled).unwrap();
        assert_eq!(plugin::status(&decree_dir).unwrap().archived.len(), 1);
        export_edited(root, false, Format::Json).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), pulled);
        assert!(std::fs::read_to_string(&gate)
            .unwrap()
            .contains("cargo test"));
        // Without the pull, the edit is exported.
        std::fs::write(&file, text).unwrap();
        export_edited(root, false, Format::Json).unwrap();
        assert!(std::fs::read_to_string(&file)
            .unwrap()
            .contains("cargo test"));
    }

    #[test]
    fn export_edited_names_an_edited_official_plugin_and_writes_nothing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let decree_dir = root.join(DECREE_DIR);
        let plugins = decree_dir.join(plugin::PLUGINS_DIR);
        std::fs::create_dir_all(&plugins).unwrap();
        let text = official::get("decree_prune").unwrap().text;
        std::fs::write(plugins.join("decree_prune.md"), text).unwrap();
        plugin::sync(&decree_dir, false).unwrap();
        let script = decree_dir.join("machines/decree_prune/scripts/prune.sh");
        assert!(script.is_file());
        std::fs::write(&script, "#!/usr/bin/env bash\necho edited\n").unwrap();
        export_edited(root, false, Format::Json).unwrap();
        assert_eq!(
            std::fs::read_to_string(plugins.join("decree_prune.md")).unwrap(),
            official::get("decree_prune").unwrap().text
        );
        assert!(export(root, "decree_prune", Format::Json)
            .unwrap_err()
            .to_string()
            .contains("extends: decree_prune"));
    }

    #[test]
    fn host_and_full_diff() {
        assert_eq!(host("https://example.com/x.md"), "example.com");
        assert_eq!(host("https://u@h.io:8443/a?b"), "h.io:8443");
        assert_eq!(diff_full("a\nb\n", "a\nc\n"), ["  a", "- b", "+ c"]);
    }
}
