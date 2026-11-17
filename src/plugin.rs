//! Plugins (docs/reference/plugins.md): one markdown file per machine, `.decree/plugins/<name>.md`,
//! whose frontmatter holds the machine and the files of `scripts/<name>/`. decree installs a
//! plugin into `machines/<name>.yml` and `scripts/<name>/`, exports an installed copy back to
//! its file, and never overwrites an installed copy that was edited in place.
//!
//! What decree records about an install is a copy of the plugin file as it was installed,
//! `.decree/cache/plugins/<name>.md`. A plugin changed since its install when its file differs
//! from that record; its installed copy was edited when it differs from what the record
//! installs. The machine is compared as YAML, so a comment or a reordered key is no edit;
//! every file of `scripts/<name>/` is compared byte for byte.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde_norway::{Mapping, Value};

use crate::commands::check::md_files;
use crate::error::DecreeError;
use crate::layout::CACHE_DIR;
use crate::machine::{self, is_ident, Invoke, MACHINES_DIR};
use crate::message::{write_replace, Message};
use crate::runtime::resolve::{resolve_script, SCRIPTS_DIR};

/// Directory holding plugin files, relative to `.decree/`.
pub const PLUGINS_DIR: &str = "plugins";

/// Directory under `.decree/cache/` holding the record of each install.
const RECORDS_DIR: &str = "plugins";

/// The keys a plugin's frontmatter may have.
const KEYS: [&str; 4] = ["name", "machine", "scripts", "needs"];

/// A plugin file, parsed and checked against the plugin schema.
#[derive(Debug, Clone, PartialEq)]
pub struct Plugin {
    pub name: String,
    /// The machine as `machines/<name>.yml` holds it, without `name`.
    pub machine: Mapping,
    /// Each file of `scripts/<name>/`, by its path there.
    pub scripts: BTreeMap<String, String>,
    /// The other plugins this one relies on, as written.
    pub needs: Vec<String>,
    /// The markdown below the frontmatter.
    pub body: String,
}

/// Why a plugin file fails the plugin schema (P1): the file line, when known, and what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub line: Option<usize>,
    pub message: String,
}

impl Problem {
    fn at(line: usize, message: impl Into<String>) -> Problem {
        Problem {
            line: Some(line),
            message: message.into(),
        }
    }
}

impl Plugin {
    /// Parse a plugin file's text and check it against the plugin schema. `stem`, when given,
    /// is the file's stem, which `name` must equal.
    pub fn parse(text: &str, stem: Option<&str>) -> Result<Plugin, Vec<Problem>> {
        let mut message = Message::parse(text).map_err(|(line, m)| vec![Problem::at(line, m)])?;
        // `Message::parse` reads the frontmatter without its last line end, which a block
        // scalar at its end (`|`) keeps: read it again with it, so a script's text is exact.
        if let Some(Ok(Value::Mapping(map))) =
            frontmatter_yaml(text).map(|yaml| serde_norway::from_str::<Value>(&yaml))
        {
            message.frontmatter = map;
        }
        let line = |key: &str| message.line_of(key);
        let mut problems = Vec::new();
        for key in message.frontmatter.keys() {
            match key.as_str() {
                Some(k) if KEYS.contains(&k) => {}
                _ => {
                    let shown = key.as_str().map_or_else(
                        || serde_norway::to_string(key).unwrap_or_default(),
                        String::from,
                    );
                    let shown = shown.trim();
                    problems.push(Problem::at(
                        key.as_str().map_or(1, line),
                        format!(
                            "unknown key `{shown}`: a plugin has `name`, `machine`, `scripts` and `needs`"
                        ),
                    ));
                }
            }
        }

        let name = match message.frontmatter.get("name") {
            None => {
                problems.push(Problem::at(1, "no `name`: a plugin names its machine"));
                None
            }
            Some(Value::String(name)) if is_ident(name) => {
                if let Some(stem) = stem.filter(|stem| *stem != name) {
                    problems.push(Problem::at(
                        line("name"),
                        format!(
                            "`name` is `{name}`, but the file is `{stem}.md`: they must be equal"
                        ),
                    ));
                }
                Some(name.clone())
            }
            Some(Value::String(name)) => {
                problems.push(Problem::at(
                    line("name"),
                    format!("`name` `{name}` does not match ^[a-z][a-z0-9_]*$"),
                ));
                None
            }
            Some(_) => {
                problems.push(Problem::at(line("name"), "`name` must be a string"));
                None
            }
        };

        let machine = match message.frontmatter.get("machine") {
            None => {
                problems.push(Problem::at(1, "no `machine`: a plugin holds one machine"));
                Mapping::new()
            }
            Some(Value::Mapping(m)) => {
                if m.contains_key("name") {
                    problems.push(Problem::at(
                        line("machine"),
                        "`machine` has `name`: the plugin's `name` names the machine",
                    ));
                }
                m.clone()
            }
            Some(_) => {
                problems.push(Problem::at(line("machine"), "`machine` must be a mapping"));
                Mapping::new()
            }
        };

        let mut scripts = BTreeMap::new();
        match message.frontmatter.get("scripts") {
            None => {}
            Some(Value::Mapping(map)) => {
                for (path, text) in map {
                    let Some(path) = path.as_str() else {
                        problems.push(Problem::at(
                            line("scripts"),
                            "a `scripts` path must be a string",
                        ));
                        continue;
                    };
                    if !is_plain_path(path) {
                        problems.push(Problem::at(
                            line("scripts"),
                            format!(
                                "script path `{path}` is not a plain relative path: no `..`, `.`, empty parts or leading `/`"
                            ),
                        ));
                        continue;
                    }
                    match text.as_str() {
                        Some(text) => {
                            scripts.insert(path.to_string(), text.to_string());
                        }
                        None => problems.push(Problem::at(
                            line("scripts"),
                            format!("script `{path}` must be a string, the file's text"),
                        )),
                    }
                }
                for path in scripts.keys() {
                    let prefix = format!("{path}/");
                    if scripts.keys().any(|other| other.starts_with(&prefix)) {
                        problems.push(Problem::at(
                            line("scripts"),
                            format!("script path `{path}` is both a file and a folder"),
                        ));
                    }
                }
            }
            Some(_) => problems.push(Problem::at(
                line("scripts"),
                "`scripts` must be a mapping of paths to their text",
            )),
        }

        let mut needs = Vec::new();
        match message.frontmatter.get("needs") {
            None => {}
            Some(Value::Sequence(list)) => {
                for need in list {
                    match need.as_str() {
                        Some(n) if is_ident(n) => {
                            if needs.iter().any(|x| x == n) {
                                problems.push(Problem::at(
                                    line("needs"),
                                    format!("`needs` lists `{n}` twice"),
                                ));
                            } else {
                                needs.push(n.to_string());
                            }
                        }
                        _ => problems.push(Problem::at(
                            line("needs"),
                            "each `needs` entry is a plugin name matching ^[a-z][a-z0-9_]*$",
                        )),
                    }
                }
            }
            Some(_) => problems.push(Problem::at(
                line("needs"),
                "`needs` must be a list of plugin names",
            )),
        }

        match name {
            Some(name) if problems.is_empty() => Ok(Plugin {
                name,
                machine,
                scripts,
                needs,
                body: message.body,
            }),
            _ => Err(problems),
        }
    }

    /// The plugin file, as export writes it: `name`, `needs` (unless empty), `machine` and
    /// `scripts` (unless empty), then the body.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut message = Message::new(self.body.clone());
        message.set("name", self.name.as_str());
        if !self.needs.is_empty() {
            let needs: Vec<Value> = self.needs.iter().map(|n| n.as_str().into()).collect();
            message.set("needs", Value::Sequence(needs));
        }
        message.set("machine", Value::Mapping(self.machine.clone()));
        if !self.scripts.is_empty() {
            let mut scripts = Mapping::new();
            for (path, text) in &self.scripts {
                scripts.insert(path.as_str().into(), text.as_str().into());
            }
            message.set("scripts", Value::Mapping(scripts));
        }
        message.to_bytes()
    }

    /// The machine with `name` first, as `machines/<name>.yml` holds it.
    pub fn machine_value(&self) -> Value {
        let mut out = Mapping::new();
        out.insert("name".into(), self.name.as_str().into());
        for (key, value) in &self.machine {
            out.insert(key.clone(), value.clone());
        }
        Value::Mapping(out)
    }

    /// The text install writes to `machines/<name>.yml`.
    pub fn machine_text(&self) -> String {
        // A `Value` built from parsed YAML always serializes.
        serde_norway::to_string(&self.machine_value()).unwrap_or_default()
    }

    /// The machine's `description`, or an empty string.
    pub fn description(&self) -> &str {
        self.machine
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    /// Every file install writes, relative to `.decree/`, with its text: the machine, then the
    /// scripts in path order.
    pub fn files(&self) -> Vec<(String, String)> {
        let mut out = vec![(machine_file(&self.name), self.machine_text())];
        out.extend(
            self.scripts
                .iter()
                .map(|(path, text)| (format!("{SCRIPTS_DIR}/{}/{path}", self.name), text.clone())),
        );
        out
    }
}

/// The frontmatter of `text` with every line end, as `\n`: the lines between the opening and
/// the closing `---` (docs/reference/messages.md, Parsing and writing). `None` without both.
fn frontmatter_yaml(text: &str) -> Option<String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let is_fence = |l: &str| l.trim_end() == "---";
    let mut lines = text.split_inclusive('\n');
    if !lines.next().is_some_and(is_fence) {
        return None;
    }
    let mut yaml = String::new();
    for line in lines {
        if is_fence(line) {
            return Some(yaml);
        }
        yaml.push_str(line.trim_end_matches(['\r', '\n']));
        yaml.push('\n');
    }
    None
}

/// `machines/<name>.yml`, relative to `.decree/`.
fn machine_file(name: &str) -> String {
    format!("{MACHINES_DIR}/{name}.yml")
}

/// A path relative to `scripts/<name>/` with nothing that leaves it: not empty, no leading
/// `/`, no empty, `.` or `..` part, no `\` and no NUL.
pub fn is_plain_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', '\0'])
        && path.split('/').all(|part| !matches!(part, "" | "." | ".."))
}

/// What is installed for one name: the machine file's bytes and every file of
/// `scripts/<name>/`.
#[derive(Debug, Default)]
struct Copy {
    machine: Option<Vec<u8>>,
    /// `None` when `scripts/<name>` does not exist.
    scripts: Option<BTreeMap<String, Vec<u8>>>,
    /// `scripts/<name>` is a file, not a folder.
    scripts_is_file: bool,
}

impl Copy {
    fn read(decree_dir: &Path, name: &str) -> Result<Copy, DecreeError> {
        let machine = match std::fs::read(decree_dir.join(machine_file(name))) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(io_error(&decree_dir.join(machine_file(name)), e)),
        };
        let dir = scripts_dir(decree_dir, name);
        let mut copy = Copy {
            machine,
            ..Copy::default()
        };
        if dir.is_dir() {
            copy.scripts = Some(read_tree(&dir)?);
        } else if dir.exists() {
            copy.scripts_is_file = true;
        }
        Ok(copy)
    }

    /// Nothing is installed.
    fn is_absent(&self) -> bool {
        self.machine.is_none() && self.scripts.is_none() && !self.scripts_is_file
    }

    /// Whether this is exactly what `plugin` installs.
    fn matches(&self, plugin: &Plugin) -> bool {
        self.machine_matches(plugin) && !self.scripts_is_file && self.scripts_match(plugin)
    }

    fn machine_matches(&self, plugin: &Plugin) -> bool {
        self.machine
            .as_deref()
            .and_then(|bytes| serde_norway::from_slice::<Value>(bytes).ok())
            .is_some_and(|value| value == plugin.machine_value())
    }

    fn scripts_match(&self, plugin: &Plugin) -> bool {
        let empty = BTreeMap::new();
        let have = self.scripts.as_ref().unwrap_or(&empty);
        have.len() == plugin.scripts.len()
            && plugin
                .scripts
                .iter()
                .all(|(path, text)| have.get(path).map(Vec::as_slice) == Some(text.as_bytes()))
    }
}

/// `.decree/scripts/<name>/`.
fn scripts_dir(decree_dir: &Path, name: &str) -> PathBuf {
    decree_dir.join(SCRIPTS_DIR).join(name)
}

/// `.decree/cache/plugins/<name>.md`, the record of an install.
fn record_path(decree_dir: &Path, name: &str) -> PathBuf {
    decree_dir
        .join(CACHE_DIR)
        .join(RECORDS_DIR)
        .join(format!("{name}.md"))
}

fn io_error(path: &Path, e: io::Error) -> DecreeError {
    DecreeError::Other(format!("{}: {e}", path.display()))
}

/// Every file under `dir`, by its `/`-separated path there, with its bytes.
fn read_tree(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>, DecreeError> {
    let mut out = BTreeMap::new();
    let mut stack = vec![(dir.to_path_buf(), String::new())];
    while let Some((path, prefix)) = stack.pop() {
        let entries = std::fs::read_dir(&path).map_err(|e| io_error(&path, e))?;
        for entry in entries {
            let entry = entry.map_err(|e| io_error(&path, e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = format!("{prefix}{name}");
            let full = entry.path();
            if full.is_dir() {
                stack.push((full, format!("{rel}/")));
            } else {
                let bytes = std::fs::read(&full).map_err(|e| io_error(&full, e))?;
                out.insert(rel, bytes);
            }
        }
    }
    Ok(out)
}

/// Make the installed copy exactly what `plugin` installs, and record `text`, the plugin
/// file, as installed. The machine file is written only when it differs as YAML; in
/// `scripts/<name>/`, files the plugin does not ship are removed, and each file is written
/// when its bytes differ. A file is executable when it starts with `#!`.
pub fn install(decree_dir: &Path, plugin: &Plugin, text: &[u8]) -> Result<(), DecreeError> {
    let copy = Copy::read(decree_dir, &plugin.name)?;
    if !copy.machine_matches(plugin) {
        let path = decree_dir.join(machine_file(&plugin.name));
        create_dir(path.parent())?;
        write_replace(&path, plugin.machine_text().as_bytes())
            .map_err(|(path, e)| io_error(&path, e))?;
    }
    let dir = scripts_dir(decree_dir, &plugin.name);
    if copy.scripts_is_file {
        return Err(DecreeError::Other(format!(
            "{}: is a file, but plugin `{}` installs its scripts in a folder of that name; move it",
            dir.display(),
            plugin.name
        )));
    }
    if plugin.scripts.is_empty() {
        if dir.is_dir() {
            std::fs::remove_dir_all(&dir).map_err(|e| io_error(&dir, e))?;
        }
    } else {
        let have = copy.scripts.unwrap_or_default();
        std::fs::create_dir_all(&dir).map_err(|e| io_error(&dir, e))?;
        for path in have.keys().filter(|p| !plugin.scripts.contains_key(*p)) {
            let full = dir.join(path);
            std::fs::remove_file(&full).map_err(|e| io_error(&full, e))?;
        }
        remove_empty_dirs(&dir)?;
        for (path, text) in &plugin.scripts {
            let full = dir.join(path);
            if have.get(path).map(Vec::as_slice) != Some(text.as_bytes()) {
                create_dir(full.parent())?;
                write_replace(&full, text.as_bytes()).map_err(|(path, e)| io_error(&path, e))?;
            }
            let mode = if text.starts_with("#!") { 0o755 } else { 0o644 };
            std::fs::set_permissions(&full, std::fs::Permissions::from_mode(mode))
                .map_err(|e| io_error(&full, e))?;
        }
    }
    record(decree_dir, &plugin.name, text)
}

fn create_dir(dir: Option<&Path>) -> Result<(), DecreeError> {
    match dir {
        Some(dir) => std::fs::create_dir_all(dir).map_err(|e| io_error(dir, e)),
        None => Ok(()),
    }
}

/// Remove every empty folder under `dir`, deepest first; `dir` itself stays.
fn remove_empty_dirs(dir: &Path) -> Result<(), DecreeError> {
    let entries = std::fs::read_dir(dir).map_err(|e| io_error(dir, e))?;
    for entry in entries {
        let path = entry.map_err(|e| io_error(dir, e))?.path();
        if path.is_dir() && !path.is_symlink() {
            remove_empty_dirs(&path)?;
            let empty = std::fs::read_dir(&path)
                .map_err(|e| io_error(&path, e))?
                .next()
                .is_none();
            if empty {
                std::fs::remove_dir(&path).map_err(|e| io_error(&path, e))?;
            }
        }
    }
    Ok(())
}

/// Record `text` as plugin `name`'s installed file.
fn record(decree_dir: &Path, name: &str, text: &[u8]) -> Result<(), DecreeError> {
    let path = record_path(decree_dir, name);
    create_dir(path.parent())?;
    write_replace(&path, text).map_err(|(path, e)| io_error(&path, e))
}

/// The recorded plugin files, by name.
fn records(decree_dir: &Path) -> Result<BTreeMap<String, Vec<u8>>, DecreeError> {
    let dir = decree_dir.join(CACHE_DIR).join(RECORDS_DIR);
    let mut out = BTreeMap::new();
    for file in md_files(&dir)? {
        let path = dir.join(&file);
        let bytes = std::fs::read(&path).map_err(|e| io_error(&path, e))?;
        out.insert(stem(&file).to_string(), bytes);
    }
    Ok(out)
}

fn stem(file: &str) -> &str {
    file.strip_suffix(".md").unwrap_or(file)
}

/// Remove plugin `name`'s installed copy and its record.
fn uninstall(decree_dir: &Path, name: &str) -> Result<(), DecreeError> {
    let machine = decree_dir.join(machine_file(name));
    if machine.exists() {
        std::fs::remove_file(&machine).map_err(|e| io_error(&machine, e))?;
    }
    let dir = scripts_dir(decree_dir, name);
    if dir.is_dir() {
        std::fs::remove_dir_all(&dir).map_err(|e| io_error(&dir, e))?;
    }
    forget(decree_dir, name)
}

/// Remove plugin `name`'s record.
fn forget(decree_dir: &Path, name: &str) -> Result<(), DecreeError> {
    let path = record_path(decree_dir, name);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_error(&path, e)),
    }
}

/// One plugin file: its name (the file's stem), its bytes, and the plugin or its problems.
pub struct Found {
    pub name: String,
    pub text: Vec<u8>,
    pub parsed: Result<Plugin, Vec<Problem>>,
}

/// Every plugin file in `plugins/`, by name.
pub fn read_all(decree_dir: &Path) -> Result<Vec<Found>, DecreeError> {
    let dir = decree_dir.join(PLUGINS_DIR);
    let mut out = Vec::new();
    for file in md_files(&dir)? {
        out.push(read_one(decree_dir, stem(&file))?);
    }
    Ok(out)
}

/// `plugins/<name>.md`, read and parsed.
pub fn read_one(decree_dir: &Path, name: &str) -> Result<Found, DecreeError> {
    let path = decree_dir.join(PLUGINS_DIR).join(format!("{name}.md"));
    let text = std::fs::read(&path).map_err(|e| io_error(&path, e))?;
    let parsed = match std::str::from_utf8(&text) {
        Ok(s) => Plugin::parse(s, Some(name)),
        Err(_) => Err(vec![Problem::at(1, "file is not valid UTF-8")]),
    };
    Ok(Found {
        name: name.to_string(),
        text,
        parsed,
    })
}

/// What `sync` did and found.
#[derive(Debug, Default)]
pub struct Synced {
    /// Plugins installed now.
    pub installed: Vec<String>,
    /// Plugins whose file was deleted, whose installed copy was removed.
    pub removed: Vec<String>,
    /// Plugins whose file was deleted but whose installed copy was edited: the copy stays,
    /// as a machine no plugin installed.
    pub kept: Vec<String>,
    /// Plugins that changed while their installed copy was edited: not installed (P3).
    pub conflicts: Vec<String>,
    /// Plugin files that fail the schema (P1), by name: not installed.
    pub invalid: Vec<(String, Vec<Problem>)>,
    /// `needs` that are not in `plugins/` (P2): (plugin, missing).
    pub missing_needs: Vec<(String, Vec<String>)>,
}

/// Install every plugin in `plugins/` that is new or changed since it was installed, and
/// remove the installed copy of each plugin whose file was deleted, never overwriting an
/// installed copy that was edited (docs/reference/plugins.md, Installing). A project without
/// `plugins/` and without records is left as it is.
pub fn sync(decree_dir: &Path) -> Result<Synced, DecreeError> {
    let mut out = Synced::default();
    let mut recorded = records(decree_dir)?;
    let found = read_all(decree_dir)?;
    let names: BTreeSet<String> = found.iter().map(|f| f.name.clone()).collect();
    for f in found {
        let plugin = match f.parsed {
            Ok(plugin) => plugin,
            Err(problems) => {
                out.invalid.push((f.name, problems));
                continue;
            }
        };
        let missing: Vec<String> = plugin
            .needs
            .iter()
            .filter(|n| !names.contains(*n))
            .cloned()
            .collect();
        if !missing.is_empty() {
            out.missing_needs.push((plugin.name.clone(), missing));
        }
        let record = recorded.remove(&plugin.name);
        let copy = Copy::read(decree_dir, &plugin.name)?;
        if copy.is_absent() {
            install(decree_dir, &plugin, &f.text)?;
            out.installed.push(plugin.name);
            continue;
        }
        if record.as_deref() == Some(f.text.as_slice()) {
            // Unchanged: an edit stays and runs.
            continue;
        }
        if copy.matches(&plugin) {
            record_only(decree_dir, &plugin.name, &f.text)?;
            continue;
        }
        let unedited = record
            .as_deref()
            .and_then(|old| std::str::from_utf8(old).ok())
            .and_then(|old| Plugin::parse(old, None).ok())
            .is_some_and(|old| copy.matches(&old));
        if unedited {
            install(decree_dir, &plugin, &f.text)?;
            out.installed.push(plugin.name);
        } else {
            out.conflicts.push(plugin.name);
        }
    }
    // What is left was recorded, but its plugin file is gone.
    for (name, old) in recorded {
        let copy = Copy::read(decree_dir, &name)?;
        let unedited = std::str::from_utf8(&old)
            .ok()
            .and_then(|old| Plugin::parse(old, None).ok())
            .is_some_and(|old| copy.matches(&old));
        if copy.is_absent() {
            forget(decree_dir, &name)?;
        } else if unedited {
            uninstall(decree_dir, &name)?;
            out.removed.push(name);
        } else {
            forget(decree_dir, &name)?;
            out.kept.push(name);
        }
    }
    Ok(out)
}

fn record_only(decree_dir: &Path, name: &str, text: &[u8]) -> Result<(), DecreeError> {
    record(decree_dir, name, text)
}

impl Synced {
    /// One line per conflict, invalid plugin, and plugin whose edited copy was kept, for
    /// `process`, `daemon` and `graph` to print.
    pub fn notices(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (name, problems) in &self.invalid {
            for p in problems {
                out.push(format!("{}: {} (P1)", problem_at(name, p.line), p.message));
            }
        }
        for name in &self.conflicts {
            out.push(format!(
                "{}: {} (P3)",
                plugin_file(name),
                conflict_message(name)
            ));
        }
        for name in &self.kept {
            out.push(format!(
                "{}: deleted, but its installed copy was edited, so {} and {SCRIPTS_DIR}/{name}/ stay as a machine no plugin installed",
                plugin_file(name),
                machine_file(name)
            ));
        }
        out
    }
}

fn problem_at(name: &str, line: Option<usize>) -> String {
    match line {
        Some(line) => format!("{}: line {line}", plugin_file(name)),
        None => plugin_file(name),
    }
}

/// `plugins/<name>.md`, relative to `.decree/`.
pub fn plugin_file(name: &str) -> String {
    format!("{PLUGINS_DIR}/{name}.md")
}

/// What a conflict is and both ways out of it.
pub fn conflict_message(name: &str) -> String {
    format!(
        "plugin `{name}` changed, but its installed copy ({} or {SCRIPTS_DIR}/{name}/) was edited in place, so it was not installed and the edited copy keeps running. To keep the edit, run `decree plugin export {name}` and merge with git; to take the plugin's version, run `decree plugin install {name}`",
        machine_file(name)
    )
}

/// The plugin of the installed copy of machine `name` (docs/reference/plugins.md, Export): the
/// machine, every file of `scripts/<name>/`, and every flat `scripts/` file its states run,
/// as a file of `scripts/<name>/`, and `needs`, every other machine it invokes, emits to or
/// routes through. An existing plugin file keeps its body and its `needs`; a new one gets
/// `# <name>` and the machine's description.
pub fn export(decree_dir: &Path, name: &str) -> Result<Plugin, DecreeError> {
    let path = decree_dir.join(machine_file(name));
    if !is_ident(name) {
        return Err(DecreeError::Other(format!(
            "plugin name `{name}` does not match ^[a-z][a-z0-9_]*$"
        )));
    }
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(DecreeError::Other(format!(
                "no machine `{name}`: {}/{} does not exist",
                crate::layout::DECREE_DIR,
                machine_file(name)
            )))
        }
        Err(e) => return Err(io_error(&path, e)),
    };
    let loaded = machine::load_machine_text(name, &text)
        .map_err(|e| DecreeError::Other(format!("{e}; fix it first (`decree check`)")))?;
    let mut machine = match serde_norway::from_str::<Value>(&text) {
        Ok(Value::Mapping(m)) => m,
        _ => {
            return Err(DecreeError::Other(format!(
                "{}: not a YAML mapping",
                machine_file(name)
            )))
        }
    };
    machine.shift_remove("name");

    let mut scripts = BTreeMap::new();
    let dir = scripts_dir(decree_dir, name);
    if dir.is_dir() {
        for (path, bytes) in read_tree(&dir)? {
            let text = String::from_utf8(bytes).map_err(|_| {
                DecreeError::Other(format!(
                    "{SCRIPTS_DIR}/{name}/{path}: not UTF-8 text; a plugin holds text files only"
                ))
            })?;
            scripts.insert(path, text);
        }
    }
    let flat = decree_dir.join(SCRIPTS_DIR);
    for script in script_names(&loaded) {
        let Ok(found) = resolve_script(decree_dir, name, &script) else {
            continue;
        };
        if found.parent() != Some(flat.as_path()) {
            continue;
        }
        let Some(file) = found.file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        let text = std::fs::read_to_string(&found).map_err(|e| io_error(&found, e))?;
        scripts.insert(file.to_string(), text);
    }

    let mut needs = BTreeSet::new();
    for (_, child) in loaded.invoked_machines() {
        needs.insert(child.to_string());
    }
    for node in &loaded.nodes {
        needs.extend(node.emits.iter().cloned());
    }
    needs.remove(name);

    let existing = read_one(decree_dir, name).ok();
    // An existing plugin file keeps its `needs`: a plugin may rely on another in a way the
    // machine does not show (develop's precheck runs infra's run.sh).
    if let Some(Ok(old)) = existing.as_ref().map(|f| &f.parsed) {
        needs.extend(old.needs.iter().cloned());
    }
    needs.remove(name);
    let body = match existing {
        Some(Found { text, .. }) => std::str::from_utf8(&text)
            .ok()
            .and_then(|t| Message::parse(t).ok())
            .map(|m| m.body),
        None => None,
    };
    let body = body.unwrap_or_else(|| {
        let description = loaded.description().trim();
        if description.is_empty() {
            format!("# {name}\n")
        } else {
            format!("# {name}\n\n{description}\n")
        }
    });
    Ok(Plugin {
        name: name.to_string(),
        machine,
        scripts,
        needs: needs.into_iter().collect(),
        body,
    })
}

/// Whether `plugin`'s installed copy exists and differs from what the plugin installs: it was
/// edited in place, or the plugin changed while it was (a conflict).
pub fn edited(decree_dir: &Path, plugin: &Plugin) -> Result<bool, DecreeError> {
    let copy = Copy::read(decree_dir, &plugin.name)?;
    Ok(!copy.is_absent() && !copy.matches(plugin))
}

/// Write `plugin` to `plugins/<name>.md` and make the installed copy match it: the flat
/// scripts it took are copied into `scripts/<name>/`, where they already resolve first.
pub fn write_and_install(decree_dir: &Path, plugin: &Plugin) -> Result<PathBuf, DecreeError> {
    let path = decree_dir.join(plugin_file(&plugin.name));
    create_dir(path.parent())?;
    let bytes = plugin.to_bytes();
    write_replace(&path, &bytes).map_err(|(path, e)| io_error(&path, e))?;
    install(decree_dir, plugin, &bytes)?;
    Ok(path)
}

/// Every script name a machine runs: root `onentry` and `onexit`, and each state's script
/// invoke, `reply` `notify`, `onentry` and `onexit`, sorted.
fn script_names(m: &machine::LoadedMachine) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for node in &m.nodes {
        out.extend(node.onentry.iter().cloned());
        out.extend(node.onexit.iter().cloned());
        match &node.invoke {
            Some(Invoke::Script(s)) => {
                out.insert(s.name.clone());
            }
            Some(Invoke::Reply(p)) => out.extend(p.notify.iter().cloned()),
            _ => {}
        }
    }
    out
}

/// The machines in `machines/` that no plugin file names, for `decree check`'s warning.
/// Empty when the project has no `plugins/`.
pub fn unplugged(
    decree_dir: &Path,
    machine_ids: &BTreeSet<String>,
) -> Result<Vec<String>, DecreeError> {
    let dir = decree_dir.join(PLUGINS_DIR);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let plugins: BTreeSet<String> = md_files(&dir)?
        .iter()
        .map(|f| stem(f).to_string())
        .collect();
    Ok(machine_ids
        .iter()
        .filter(|id| !plugins.contains(*id))
        .cloned()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLUGIN: &str = "\
---
name: hello
needs:
- other
machine:
  description: Says hello.
  initial: greet
  states:
    greet:
      invoke: greet
      transitions:
        done: done
    done:
      final: true
    failed:
      final: true
scripts:
  greet.sh: |
    #!/usr/bin/env bash
    echo hi
  kinds/data.txt: no newline
---
# hello

Says hello.
";

    #[test]
    fn parses_and_writes_the_same_bytes() {
        let plugin = Plugin::parse(PLUGIN, Some("hello")).unwrap();
        assert_eq!(plugin.name, "hello");
        assert_eq!(plugin.needs, ["other"]);
        assert_eq!(plugin.scripts["greet.sh"], "#!/usr/bin/env bash\necho hi\n");
        assert_eq!(plugin.scripts["kinds/data.txt"], "no newline");
        assert_eq!(String::from_utf8(plugin.to_bytes()).unwrap(), PLUGIN);
        assert!(plugin
            .machine_text()
            .starts_with("name: hello\ndescription: Says hello.\n"));
    }

    #[test]
    fn text_round_trips_through_yaml() {
        for text in [
            "",
            "a",
            "a\n",
            "a\n\n",
            "  leading\nspace\n",
            "trailing \nspace\n",
            "tab\there\n",
            "#!/bin/sh\necho \"$1\" 'x' : y\n",
            "\n\nstarts blank\n",
            "no newline at end\nreally",
        ] {
            let mut plugin = Plugin::parse(PLUGIN, None).unwrap();
            plugin.scripts.insert("x".into(), text.into());
            let bytes = plugin.to_bytes();
            let back = Plugin::parse(std::str::from_utf8(&bytes).unwrap(), None).unwrap();
            assert_eq!(back.scripts["x"], text, "{text:?}");
            assert_eq!(back.to_bytes(), bytes);
        }
    }

    fn problems(text: &str) -> Vec<String> {
        Plugin::parse(text, Some("hello"))
            .unwrap_err()
            .into_iter()
            .map(|p| format!("{:?}: {}", p.line, p.message))
            .collect()
    }

    #[test]
    fn rejects_what_the_schema_rejects() {
        assert_eq!(
            problems("---\nname: other\nmachine: {}\n---\n"),
            ["Some(2): `name` is `other`, but the file is `hello.md`: they must be equal"]
        );
        assert_eq!(
            problems("---\nname: hello\nmachine: { name: hello }\ncron: x\n---\n"),
            [
                "Some(4): unknown key `cron`: a plugin has `name`, `machine`, `scripts` and `needs`",
                "Some(3): `machine` has `name`: the plugin's `name` names the machine",
            ]
        );
        assert_eq!(
            problems("---\nname: hello\nmachine: {}\nscripts:\n  ../x.sh: hi\n---\n"),
            ["Some(4): script path `../x.sh` is not a plain relative path: no `..`, `.`, empty parts or leading `/`"]
        );
        assert_eq!(
            problems("---\nname: hello\nmachine: {}\nneeds: [a, a, B]\n---\n"),
            [
                "Some(4): `needs` lists `a` twice",
                "Some(4): each `needs` entry is a plugin name matching ^[a-z][a-z0-9_]*$",
            ]
        );
        assert_eq!(
            problems("---\nmachine: []\n---\n"),
            [
                "Some(1): no `name`: a plugin names its machine",
                "Some(2): `machine` must be a mapping",
            ]
        );
        assert_eq!(
            problems("---\nname: hello\nmachine: {}\nscripts: { a: x, a/b: y }\n---\n"),
            ["Some(4): script path `a` is both a file and a folder"]
        );
    }

    #[test]
    fn plain_paths() {
        for ok in ["a", "a.sh", "kinds/command.sh", "...", ".hidden", "a..b/c"] {
            assert!(is_plain_path(ok), "{ok}");
        }
        for bad in [
            "", "/a", "../x.sh", "a/../b", "./a", "a/", "a//b", "a\\b", ".",
        ] {
            assert!(!is_plain_path(bad), "{bad}");
        }
    }

    fn project() -> tempfile::TempDir {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(PLUGINS_DIR)).unwrap();
        tmp
    }

    fn put(dir: &Path, path: &str, text: &str) {
        let full = dir.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, text).unwrap();
    }

    fn get(dir: &Path, path: &str) -> Option<String> {
        std::fs::read_to_string(dir.join(path)).ok()
    }

    #[test]
    fn sync_installs_keeps_edits_and_reports_conflicts() {
        let tmp = project();
        let d = tmp.path();
        put(d, "plugins/hello.md", PLUGIN);
        let s = sync(d).unwrap();
        assert_eq!(s.installed, ["hello"]);
        assert_eq!(
            s.missing_needs,
            [("hello".to_string(), vec!["other".to_string()])]
        );
        assert!(get(d, "machines/hello.yml")
            .unwrap()
            .starts_with("name: hello\n"));
        let mode = std::fs::metadata(d.join("scripts/hello/greet.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
        let mode = std::fs::metadata(d.join("scripts/hello/kinds/data.txt"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o644);

        // An edit stays while the plugin is unchanged.
        put(d, "scripts/hello/greet.sh", "#!/bin/sh\necho edited\n");
        let s = sync(d).unwrap();
        assert!(s.installed.is_empty() && s.conflicts.is_empty());
        assert_eq!(
            get(d, "scripts/hello/greet.sh").unwrap(),
            "#!/bin/sh\necho edited\n"
        );

        // A changed plugin over an edit is a conflict; nothing is touched.
        put(
            d,
            "plugins/hello.md",
            &PLUGIN.replace("echo hi", "echo bye"),
        );
        let s = sync(d).unwrap();
        assert_eq!(s.conflicts, ["hello"]);
        assert_eq!(
            get(d, "scripts/hello/greet.sh").unwrap(),
            "#!/bin/sh\necho edited\n"
        );

        // Install takes the plugin's version.
        let found = read_one(d, "hello").unwrap();
        install(d, &found.parsed.unwrap(), &found.text).unwrap();
        assert!(get(d, "scripts/hello/greet.sh")
            .unwrap()
            .contains("echo bye"));
        assert!(sync(d).unwrap().conflicts.is_empty());

        // An unedited copy follows a changed plugin.
        put(
            d,
            "plugins/hello.md",
            &PLUGIN.replace("echo hi", "echo again"),
        );
        put(d, "scripts/hello/added.txt", "x");
        let s = sync(d).unwrap();
        assert_eq!(s.conflicts, ["hello"], "an added file is an edit");
        std::fs::remove_file(d.join("scripts/hello/added.txt")).unwrap();
        let s = sync(d).unwrap();
        assert_eq!(s.installed, ["hello"]);
        assert!(get(d, "scripts/hello/greet.sh")
            .unwrap()
            .contains("echo again"));

        // A deleted plugin's copy is removed, a hand-written machine stays.
        put(d, "machines/extra.yml", "name: extra\n");
        std::fs::remove_file(d.join("plugins/hello.md")).unwrap();
        let s = sync(d).unwrap();
        assert_eq!(s.removed, ["hello"]);
        assert!(!d.join("machines/hello.yml").exists());
        assert!(!d.join("scripts/hello").exists());
        assert!(d.join("machines/extra.yml").exists());
        assert!(!record_path(d, "hello").exists());
    }

    #[test]
    fn an_existing_copy_is_recorded_when_it_matches_and_a_conflict_when_not() {
        let tmp = project();
        let d = tmp.path();
        let plugin = Plugin::parse(PLUGIN, None).unwrap();
        // Written by hand, with a comment: equal as YAML.
        put(
            d,
            "machines/hello.yml",
            &format!("# by hand\n{}", plugin.machine_text()),
        );
        for (path, text) in &plugin.scripts {
            put(d, &format!("scripts/hello/{path}"), text);
        }
        put(d, "plugins/hello.md", PLUGIN);
        let s = sync(d).unwrap();
        assert!(s.installed.is_empty() && s.conflicts.is_empty());
        assert_eq!(
            std::fs::read(record_path(d, "hello")).unwrap(),
            PLUGIN.as_bytes()
        );
        assert!(get(d, "machines/hello.yml")
            .unwrap()
            .starts_with("# by hand"));

        std::fs::remove_file(record_path(d, "hello")).unwrap();
        put(d, "scripts/hello/greet.sh", "different");
        assert_eq!(sync(d).unwrap().conflicts, ["hello"]);
    }

    #[test]
    fn a_deleted_plugin_with_an_edited_copy_keeps_it() {
        let tmp = project();
        let d = tmp.path();
        put(d, "plugins/hello.md", PLUGIN);
        sync(d).unwrap();
        put(d, "scripts/hello/greet.sh", "edited");
        std::fs::remove_file(d.join("plugins/hello.md")).unwrap();
        let s = sync(d).unwrap();
        assert_eq!(s.kept, ["hello"]);
        assert_eq!(get(d, "scripts/hello/greet.sh").unwrap(), "edited");
        assert!(!record_path(d, "hello").exists());
    }

    #[test]
    fn an_invalid_plugin_is_not_installed() {
        let tmp = project();
        let d = tmp.path();
        put(
            d,
            "plugins/hello.md",
            "---\nname: hello\nmachine: {}\nscripts: { ../x.sh: hi }\n---\n",
        );
        let s = sync(d).unwrap();
        assert_eq!(s.invalid.len(), 1);
        assert!(!d.join("machines/hello.yml").exists());
        assert!(s.notices()[0].starts_with("plugins/hello.md: line 4: script path `../x.sh`"));
    }

    #[test]
    fn no_plugins_touches_nothing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let d = tmp.path();
        put(d, "machines/a.yml", "name: a\n");
        let s = sync(d).unwrap();
        assert!(s.installed.is_empty() && s.removed.is_empty());
        assert!(!d.join(CACHE_DIR).exists());
    }
}
