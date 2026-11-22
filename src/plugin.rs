//! Plugins (docs/reference/plugins.md): one markdown file, `.decree/plugins/<name>.md`, whose
//! frontmatter holds a machine's folder, `machines/<name>/`: its `machine.yml`, its scripts,
//! its cron files, and seeds for its `state/` and `order/`. decree installs a plugin into
//! `machine.yml`, `scripts/` and `cron/` (its installed copy), writes its state and order seeds
//! once, exports an installed copy back to its file, and never overwrites an installed copy
//! that was edited in place.
//!
//! What decree records about an install is a copy of the plugin file as it was installed,
//! `.decree/cache/plugins/<name>.md`. A plugin changed since its install when its file differs
//! from that record; its installed copy was edited when it differs from what the record
//! installs. The machine is compared as YAML, so a comment or a reordered key is no edit;
//! every file of `scripts/` and `cron/` is compared byte for byte.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde_norway::{Mapping, Value};

use crate::commands::check::md_files;
use crate::error::DecreeError;
use crate::layout::{self, CACHE_DIR, CRON_DIR, ORDER_DIR, STATE_DIR};
use crate::machine::{self, is_ident};
use crate::message::{write_replace, Message};
use crate::runtime::resolve::SCRIPTS_DIR;

/// Directory holding plugin files, relative to `.decree/`.
pub const PLUGINS_DIR: &str = "plugins";

/// Directory under `.decree/cache/` holding the record of each install.
const RECORDS_DIR: &str = "plugins";

/// The keys a plugin's frontmatter may have, besides any `x-` key.
const KEYS: [&str; 7] = [
    "name", "machine", "scripts", "cron", "state", "order", "needs",
];

/// A plugin file, parsed and checked against the plugin schema.
#[derive(Debug, Clone, PartialEq)]
pub struct Plugin {
    pub name: String,
    /// The machine as `machines/<name>.yml` holds it, without `name`; `None` for a plugin of
    /// scripts or cron files only.
    pub machine: Option<Mapping>,
    /// Each file of `scripts/<name>/`, by its path there.
    pub scripts: BTreeMap<String, String>,
    /// Each file of `cron/<name>/`, by its file name there.
    pub cron: BTreeMap<String, String>,
    /// Each seed of `state/`, by its path there: written once, never overwritten.
    pub state: BTreeMap<String, String>,
    /// Each seed of `order/`, by its file name there: written once, never overwritten.
    pub order: BTreeMap<String, String>,
    /// The other plugins this one relies on, as written.
    pub needs: Vec<String>,
    /// Every `x-` key, as written: decree keeps them and reads none.
    pub extensions: Mapping,
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
                Some(k) if KEYS.contains(&k) || k.starts_with("x-") => {}
                _ => {
                    let shown = key.as_str().map_or_else(
                        || serde_norway::to_string(key).unwrap_or_default(),
                        String::from,
                    );
                    let shown = shown.trim();
                    problems.push(Problem::at(
                        key.as_str().map_or(1, line),
                        format!(
                            "unknown key `{shown}`: a plugin has `name`, `machine`, `scripts`, `cron`, `state`, `order`, `needs` and `x-` keys"
                        ),
                    ));
                }
            }
        }

        let name = match message.frontmatter.get("name") {
            None => {
                problems.push(Problem::at(1, "no `name`: every plugin has one"));
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
            None => None,
            Some(Value::Mapping(m)) => {
                if m.contains_key("name") {
                    problems.push(Problem::at(
                        line("machine"),
                        "`machine` has `name`: the plugin's `name` names the machine",
                    ));
                }
                Some(m.clone())
            }
            Some(_) => {
                problems.push(Problem::at(line("machine"), "`machine` must be a mapping"));
                None
            }
        };

        let mut files = |key: &str, what: &str, folders: bool, md: bool| {
            file_map(
                message.frontmatter.get(key),
                key,
                what,
                folders,
                md,
                line(key),
                &mut problems,
            )
        };
        let scripts = files("scripts", "script", true, false);
        let cron = files("cron", "cron file", false, true);
        let state = files("state", "state file", true, false);
        let order = files("order", "order file", false, true);

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

        let extensions: Mapping = message
            .frontmatter
            .iter()
            .filter(|(k, _)| k.as_str().is_some_and(|k| k.starts_with("x-")))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        match name {
            Some(name) if problems.is_empty() => Ok(Plugin {
                name,
                machine,
                scripts,
                cron,
                state,
                order,
                needs,
                extensions,
                body: message.body,
            }),
            _ => Err(problems),
        }
    }

    /// The plugin file, as export writes it: `name`, `needs`, `machine`, `scripts`, `cron`,
    /// `state` and `order` (each unless empty), the `x-` keys as written, then the body.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut message = Message::new(self.body.clone());
        message.set("name", self.name.as_str());
        if !self.needs.is_empty() {
            let needs: Vec<Value> = self.needs.iter().map(|n| n.as_str().into()).collect();
            message.set("needs", Value::Sequence(needs));
        }
        if let Some(machine) = &self.machine {
            message.set("machine", Value::Mapping(machine.clone()));
        }
        for (key, files) in [
            ("scripts", &self.scripts),
            ("cron", &self.cron),
            ("state", &self.state),
            ("order", &self.order),
        ] {
            if !files.is_empty() {
                let mut map = Mapping::new();
                for (path, text) in files {
                    map.insert(path.as_str().into(), text.as_str().into());
                }
                message.set(key, Value::Mapping(map));
            }
        }
        for (key, value) in &self.extensions {
            if let Some(key) = key.as_str() {
                message.set(key, value.clone());
            }
        }
        message.to_bytes()
    }

    /// The machine with `name` first, as `machines/<name>.yml` holds it.
    pub fn machine_value(&self) -> Option<Value> {
        let machine = self.machine.as_ref()?;
        let mut out = Mapping::new();
        out.insert("name".into(), self.name.as_str().into());
        for (key, value) in machine {
            out.insert(key.clone(), value.clone());
        }
        Some(Value::Mapping(out))
    }

    /// The text install writes to `machines/<name>.yml`.
    pub fn machine_text(&self) -> Option<String> {
        // A `Value` built from parsed YAML always serializes.
        self.machine_value()
            .map(|v| serde_norway::to_string(&v).unwrap_or_default())
    }

    /// The machine's `description`, or an empty string.
    pub fn description(&self) -> &str {
        self.machine
            .as_ref()
            .and_then(|m| m.get("description"))
            .and_then(Value::as_str)
            .unwrap_or_else(|| self.body_summary())
    }

    /// A library's description: the first line of its body that is not a heading.
    fn body_summary(&self) -> &str {
        self.body
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .unwrap_or("")
    }

    /// Every file the plugin writes, relative to `.decree/`, with its text: the machine, the
    /// scripts, the cron files, and the state and order seeds, each in path order.
    pub fn files(&self) -> Vec<(String, String)> {
        let name = &self.name;
        let mut out: Vec<(String, String)> = self
            .machine_text()
            .map(|text| (machine_file(name), text))
            .into_iter()
            .collect();
        for (dir, files) in [
            (
                format!("{}/{SCRIPTS_DIR}", layout::machine_dir_rel(name)),
                &self.scripts,
            ),
            (
                format!("{}/{CRON_DIR}", layout::machine_dir_rel(name)),
                &self.cron,
            ),
            (
                format!("{}/{STATE_DIR}", layout::machine_dir_rel(name)),
                &self.state,
            ),
            (
                format!("{}/{ORDER_DIR}", layout::machine_dir_rel(name)),
                &self.order,
            ),
        ] {
            out.extend(
                files
                    .iter()
                    .map(|(path, text)| (format!("{dir}/{path}"), text.clone())),
            );
        }
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

/// Read a frontmatter map of `<path>: <its text>` (`scripts`, `cron`, `state`, `order`). Each path is plain (`is_plain_path`); with `folders` it may have `/`, else it is
/// one file name; with `md` it ends in `.md` and does not start with `.`, which decree skips
/// in `cron/` and `order/`. A path that is both a file and a folder is a
/// problem.
fn file_map(
    value: Option<&Value>,
    key: &str,
    what: &str,
    folders: bool,
    md: bool,
    line: usize,
    problems: &mut Vec<Problem>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let map = match value {
        None => return out,
        Some(Value::Mapping(map)) => map,
        Some(_) => {
            problems.push(Problem::at(
                line,
                format!("`{key}` must be a mapping of paths to their text"),
            ));
            return out;
        }
    };
    for (path, text) in map {
        let Some(path) = path.as_str() else {
            problems.push(Problem::at(
                line,
                format!("a `{key}` path must be a string"),
            ));
            continue;
        };
        if !is_plain_path(path) {
            problems.push(Problem::at(
                line,
                format!(
                    "{what} path `{path}` is not a plain relative path: no `..`, `.`, empty parts or leading `/`"
                ),
            ));
            continue;
        }
        if !folders && path.contains('/') {
            problems.push(Problem::at(
                line,
                format!("{what} `{path}` must be a file name, with no `/`"),
            ));
            continue;
        }
        if md && (!path.ends_with(".md") || path.starts_with('.')) {
            problems.push(Problem::at(
                line,
                format!("{what} `{path}` must end in `.md` and not start with `.`"),
            ));
            continue;
        }
        match text.as_str() {
            Some(text) => {
                out.insert(path.to_string(), text.to_string());
            }
            None => problems.push(Problem::at(
                line,
                format!("{what} `{path}` must be a string, the file's text"),
            )),
        }
    }
    for path in out.keys() {
        let prefix = format!("{path}/");
        if out.keys().any(|other| other.starts_with(&prefix)) {
            problems.push(Problem::at(
                line,
                format!("{what} path `{path}` is both a file and a folder"),
            ));
        }
    }
    out
}

/// `machines/<name>.yml`, relative to `.decree/`.
fn machine_file(name: &str) -> String {
    layout::machine_file_rel(name)
}

/// A path relative to `scripts/<name>/` with nothing that leaves it: not empty, no leading
/// `/`, no empty, `.` or `..` part, no `\` and no NUL.
pub fn is_plain_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', '\0'])
        && path.split('/').all(|part| !matches!(part, "" | "." | ".."))
}

/// What is installed for one name: the machine file's bytes and every file of
/// `scripts/<name>/` and `cron/<name>/`.
#[derive(Debug, Default)]
struct Copy {
    machine: Option<Vec<u8>>,
    /// `None` when `scripts/<name>` does not exist.
    scripts: Option<BTreeMap<String, Vec<u8>>>,
    /// `scripts/<name>` is a file, not a folder.
    scripts_is_file: bool,
    /// `None` when `cron/<name>` does not exist.
    cron: Option<BTreeMap<String, Vec<u8>>>,
    /// `cron/<name>` is a file, not a folder.
    cron_is_file: bool,
}

impl Copy {
    fn read(decree_dir: &Path, name: &str) -> Result<Copy, DecreeError> {
        let machine = match std::fs::read(decree_dir.join(machine_file(name))) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(io_error(&decree_dir.join(machine_file(name)), e)),
        };
        let mut copy = Copy {
            machine,
            ..Copy::default()
        };
        let dir = scripts_dir(decree_dir, name);
        if dir.is_dir() {
            copy.scripts = Some(read_tree(&dir)?);
        } else if dir.exists() {
            copy.scripts_is_file = true;
        }
        let dir = cron_dir(decree_dir, name);
        if dir.is_dir() {
            copy.cron = Some(read_tree(&dir)?);
        } else if dir.exists() {
            copy.cron_is_file = true;
        }
        Ok(copy)
    }

    /// Nothing is installed.
    fn is_absent(&self) -> bool {
        self.machine.is_none()
            && self.scripts.is_none()
            && !self.scripts_is_file
            && self.cron.is_none()
            && !self.cron_is_file
    }

    /// Whether this is exactly what `plugin` installs.
    fn matches(&self, plugin: &Plugin) -> bool {
        self.machine_matches(plugin)
            && !self.scripts_is_file
            && same_files(self.scripts.as_ref(), &plugin.scripts)
            && !self.cron_is_file
            && same_files(self.cron.as_ref(), &plugin.cron)
    }

    fn machine_matches(&self, plugin: &Plugin) -> bool {
        match (&self.machine, plugin.machine_value()) {
            (None, None) => true,
            (Some(bytes), Some(want)) => {
                serde_norway::from_slice::<Value>(bytes).is_ok_and(|have| have == want)
            }
            _ => false,
        }
    }
}

/// Whether the files of a folder (`None`: no folder) are exactly `want`, byte for byte.
fn same_files(have: Option<&BTreeMap<String, Vec<u8>>>, want: &BTreeMap<String, String>) -> bool {
    let empty = BTreeMap::new();
    let have = have.unwrap_or(&empty);
    have.len() == want.len()
        && want
            .iter()
            .all(|(path, text)| have.get(path).map(Vec::as_slice) == Some(text.as_bytes()))
}

/// `.decree/cron/<name>/`.
fn cron_dir(decree_dir: &Path, name: &str) -> PathBuf {
    layout::machine_dir(decree_dir, name).join(CRON_DIR)
}

/// `.decree/scripts/<name>/`.
fn scripts_dir(decree_dir: &Path, name: &str) -> PathBuf {
    layout::machine_dir(decree_dir, name).join(SCRIPTS_DIR)
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

/// Every file under `dir`, by its `/`-separated path there, with its bytes. A symlink to
/// a folder is not followed and not listed, so nothing under its target is ever removed;
/// a symlink to a file is listed with its target's bytes.
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
            let kind = entry.file_type().map_err(|e| io_error(&full, e))?;
            if kind.is_dir() {
                stack.push((full, format!("{rel}/")));
            } else if kind.is_symlink() && full.is_dir() {
                continue;
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
    let path = decree_dir.join(machine_file(&plugin.name));
    if !copy.machine_matches(plugin) {
        match plugin.machine_text() {
            Some(machine) => {
                create_dir(path.parent())?;
                write_replace(&path, machine.as_bytes()).map_err(|(path, e)| io_error(&path, e))?;
            }
            None => std::fs::remove_file(&path).map_err(|e| io_error(&path, e))?,
        }
    }
    let folders = [
        (
            scripts_dir(decree_dir, &plugin.name),
            copy.scripts_is_file,
            copy.scripts,
            &plugin.scripts,
            true,
        ),
        (
            cron_dir(decree_dir, &plugin.name),
            copy.cron_is_file,
            copy.cron,
            &plugin.cron,
            false,
        ),
    ];
    for (dir, is_file, have, want, scripts) in folders {
        if is_file {
            return Err(DecreeError::Other(format!(
                "{}: is a file, but plugin `{}` installs a folder of that name; move it",
                dir.display(),
                plugin.name
            )));
        }
        write_folder(&dir, have.unwrap_or_default(), want, scripts)?;
    }
    seed(decree_dir, plugin)?;
    record(decree_dir, &plugin.name, text)
}

/// Make folder `dir`, holding `have`, hold exactly `want`: remove the files it does not
/// ship, and write each that differs. With `scripts`, a file is executable when it starts
/// with `#!`, else not. An empty `want` removes the folder.
fn write_folder(
    dir: &Path,
    have: BTreeMap<String, Vec<u8>>,
    want: &BTreeMap<String, String>,
    scripts: bool,
) -> Result<(), DecreeError> {
    if want.is_empty() {
        if dir.is_dir() {
            std::fs::remove_dir_all(dir).map_err(|e| io_error(dir, e))?;
        }
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|e| io_error(dir, e))?;
    for path in have.keys().filter(|p| !want.contains_key(*p)) {
        let full = dir.join(path);
        std::fs::remove_file(&full).map_err(|e| io_error(&full, e))?;
    }
    remove_empty_dirs(dir)?;
    for (path, text) in want {
        let full = dir.join(path);
        let same = have.get(path).map(Vec::as_slice) == Some(text.as_bytes());
        if !same {
            create_dir(full.parent())?;
            write_replace(&full, text.as_bytes()).map_err(|(path, e)| io_error(&path, e))?;
        }
        if scripts {
            let mode = if text.starts_with("#!") { 0o755 } else { 0o644 };
            std::fs::set_permissions(&full, std::fs::Permissions::from_mode(mode))
                .map_err(|e| io_error(&full, e))?;
        }
    }
    Ok(())
}

/// Write `plugin`'s state and order seeds that are not there yet, into `state/` and
/// `order/`. Neither is ever overwritten: they are the project's, which an install keeps.
fn seed(decree_dir: &Path, plugin: &Plugin) -> Result<(), DecreeError> {
    let dir = layout::machine_dir(decree_dir, &plugin.name);
    for (sub, files) in [(STATE_DIR, &plugin.state), (ORDER_DIR, &plugin.order)] {
        for (file, text) in files {
            let path = dir.join(sub).join(file);
            if !path.exists() {
                create_dir(path.parent())?;
                write_replace(&path, text.as_bytes()).map_err(|(path, e)| io_error(&path, e))?;
            }
        }
    }
    Ok(())
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

/// Remove plugin `name`'s installed copy and its record. Its `state/`, `order/` and `graph.md`
/// stay: they are the project's. The folder goes when nothing is left in it.
fn uninstall(decree_dir: &Path, name: &str) -> Result<(), DecreeError> {
    let machine = decree_dir.join(machine_file(name));
    if machine.exists() {
        std::fs::remove_file(&machine).map_err(|e| io_error(&machine, e))?;
    }
    for dir in [scripts_dir(decree_dir, name), cron_dir(decree_dir, name)] {
        if dir.is_dir() {
            std::fs::remove_dir_all(&dir).map_err(|e| io_error(&dir, e))?;
        }
    }
    let dir = layout::machine_dir(decree_dir, name);
    // An empty `state/` or `order/` holds nothing of the project's.
    if dir.is_dir() {
        remove_empty_dirs(&dir)?;
    }
    if dir.is_dir()
        && std::fs::read_dir(&dir)
            .map_err(|e| io_error(&dir, e))?
            .next()
            .is_none()
    {
        std::fs::remove_dir(&dir).map_err(|e| io_error(&dir, e))?;
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

/// What `sync` did and found, or what `status` found `sync` would do.
#[derive(Debug, Default)]
pub struct Synced {
    /// Whether this is `status`: nothing was written, and `installed`, `removed` and `kept`
    /// are what `decree install` would do.
    pub checked: bool,
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
/// installed copy that was edited (docs/reference/plugins.md, Installing): what
/// `decree install` with no name does. A project without `plugins/` and without
/// records is left as it is.
pub fn sync(decree_dir: &Path) -> Result<Synced, DecreeError> {
    reconcile(decree_dir, true)
}

/// What `sync` would do, writing nothing: `check`, `process` and `daemon` report it, and
/// never install.
pub fn status(decree_dir: &Path) -> Result<Synced, DecreeError> {
    reconcile(decree_dir, false)
}

fn reconcile(decree_dir: &Path, write: bool) -> Result<Synced, DecreeError> {
    let mut out = Synced {
        checked: !write,
        ..Synced::default()
    };
    let mut recorded = records(decree_dir)?;
    let found = read_all(decree_dir)?;
    let names: BTreeSet<String> = found.iter().map(|f| f.name.clone()).collect();
    for f in found {
        let plugin = match f.parsed {
            Ok(plugin) => plugin,
            Err(problems) => {
                // The file is there: its installed copy and record stay as they were.
                recorded.remove(&f.name);
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
            if write {
                install(decree_dir, &plugin, &f.text)?;
            }
            out.installed.push(plugin.name);
            continue;
        }
        if record.as_deref() == Some(f.text.as_slice()) {
            // Unchanged: an edit stays and runs.
            continue;
        }
        if copy.matches(&plugin) {
            if write {
                record_only(decree_dir, &plugin, &f.text)?;
            }
            continue;
        }
        let unedited = record
            .as_deref()
            .and_then(|old| std::str::from_utf8(old).ok())
            .and_then(|old| Plugin::parse(old, None).ok())
            .is_some_and(|old| copy.matches(&old));
        if unedited {
            if write {
                install(decree_dir, &plugin, &f.text)?;
            }
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
        if !write {
            if copy.is_absent() {
                continue;
            } else if unedited {
                out.removed.push(name);
            } else {
                out.kept.push(name);
            }
        } else if copy.is_absent() {
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

/// Record a plugin whose installed copy already matches it, and write what it seeds.
fn record_only(decree_dir: &Path, plugin: &Plugin, text: &[u8]) -> Result<(), DecreeError> {
    seed(decree_dir, plugin)?;
    record(decree_dir, &plugin.name, text)
}

impl Synced {
    /// One line per conflict, invalid plugin, and plugin whose edited copy was kept, for
    /// `process`, `daemon` and `decree install` to print; for `status`, also one per
    /// plugin `decree install` would install or remove.
    pub fn notices(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.checked {
            for name in &self.installed {
                out.push(format!(
                    "{}: not installed, or changed since it was installed: run `decree install`",
                    plugin_file(name)
                ));
            }
            for name in &self.removed {
                out.push(format!(
                    "{}: deleted, but its installed copy is still there: run `decree install` to remove it",
                    plugin_file(name)
                ));
            }
        }
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
                "{}: deleted, but its installed copy was edited, so {} stays as a machine no plugin installed",
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
        "plugin `{name}` changed, but its installed copy ({}, or a file in {dir}/{SCRIPTS_DIR}/ or {dir}/{CRON_DIR}/) was edited in place, so it was not installed and the edited copy keeps running. To keep the edit, run `decree export {name}` and merge with git; to take the plugin's version, run `decree install {name}`",
        machine_file(name),
        dir = layout::machine_dir_rel(name)
    )
}

/// The plugin of the installed copy `name` (docs/reference/plugins.md, Export): the machine
/// (if any), every file of `scripts/<name>/` and `cron/<name>/`, every flat `scripts/` file
/// the machine's states run, as a file of `scripts/<name>/`, and `needs`, every other machine
/// it invokes, emits to or routes through. An existing plugin file keeps its body, its
/// `needs`, its state and order seeds and its `x-` keys; a new one gets `# <name>` and
/// the machine's description.
pub fn export(decree_dir: &Path, name: &str) -> Result<Plugin, DecreeError> {
    if !is_ident(name) {
        return Err(DecreeError::Other(format!(
            "plugin name `{name}` does not match ^[a-z][a-z0-9_]*$"
        )));
    }
    let path = decree_dir.join(machine_file(name));
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(io_error(&path, e)),
    };
    let sdir = scripts_dir(decree_dir, name);
    let cdir = cron_dir(decree_dir, name);
    if text.is_none() && !sdir.is_dir() && !cdir.is_dir() {
        return Err(DecreeError::Other(format!(
            "nothing installed as `{name}`: none of {dir}/{}, {dir}/{}/{SCRIPTS_DIR}/ and {dir}/{}/{CRON_DIR}/ exists",
            machine_file(name),
            layout::machine_dir_rel(name),
            layout::machine_dir_rel(name),
            dir = crate::layout::DECREE_DIR,
        )));
    }
    let loaded = text
        .as_deref()
        .map(|text| {
            machine::load_machine_text(name, text)
                .map_err(|e| DecreeError::Other(format!("{e}; fix it first (`decree check`)")))
        })
        .transpose()?;
    let machine = match text.as_deref().map(serde_norway::from_str::<Value>) {
        None => None,
        Some(Ok(Value::Mapping(mut m))) => {
            m.shift_remove("name");
            Some(m)
        }
        Some(_) => {
            return Err(DecreeError::Other(format!(
                "{}: not a YAML mapping",
                machine_file(name)
            )))
        }
    };

    let text_files = |dir: &Path, shown: &str| -> Result<BTreeMap<String, String>, DecreeError> {
        let mut out = BTreeMap::new();
        if dir.is_dir() {
            for (path, bytes) in read_tree(dir)? {
                let text = String::from_utf8(bytes).map_err(|_| {
                    DecreeError::Other(format!(
                        "{}/{shown}/{path}: not UTF-8 text; a plugin holds text files only",
                        layout::machine_dir_rel(name)
                    ))
                })?;
                out.insert(path, text);
            }
        }
        Ok(out)
    };
    let scripts = text_files(&sdir, SCRIPTS_DIR)?;
    let cron = text_files(&cdir, CRON_DIR)?;
    let mut needs = BTreeSet::new();
    if let Some(loaded) = &loaded {
        for (_, child) in loaded.invoked_machines() {
            needs.insert(child.to_string());
        }
        for node in &loaded.nodes {
            needs.extend(node.emits.iter().cloned());
        }
    }

    let existing = read_one(decree_dir, name).ok();
    let old = existing.as_ref().and_then(|f| f.parsed.as_ref().ok());
    // An existing plugin file keeps its `needs`: a plugin may rely on another in a way the
    // machine does not show (develop's precheck runs infra's run.sh).
    if let Some(old) = old {
        needs.extend(old.needs.iter().cloned());
    }
    needs.remove(name);
    let (state, order, extensions) = match old {
        Some(old) => (old.state.clone(), old.order.clone(), old.extensions.clone()),
        None => Default::default(),
    };
    let body = existing
        .as_ref()
        .and_then(|f| std::str::from_utf8(&f.text).ok())
        .and_then(|t| Message::parse(t).ok())
        .map(|m| m.body);
    let body = body.unwrap_or_else(|| {
        let description = loaded.as_ref().map_or("", |l| l.description().trim());
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
        cron,
        state,
        order,
        needs: needs.into_iter().collect(),
        extensions,
        body,
    })
}

/// Whether `plugin`'s installed copy exists and differs from what the plugin installs: it was
/// edited in place, or the plugin changed while it was (a conflict).
pub fn edited(decree_dir: &Path, plugin: &Plugin) -> Result<bool, DecreeError> {
    let copy = Copy::read(decree_dir, &plugin.name)?;
    Ok(!copy.is_absent() && !copy.matches(plugin))
}

/// Whether plugin `name`'s installed copy was edited in place since it was installed: it
/// differs from what the plugin file it was installed from installs. False when nothing is
/// installed or no record says what it was installed from.
pub fn edited_since_install(decree_dir: &Path, name: &str) -> Result<bool, DecreeError> {
    let path = record_path(decree_dir, name);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(io_error(&path, e)),
    };
    match Plugin::parse(&text, Some(name)) {
        Ok(recorded) => edited(decree_dir, &recorded),
        Err(_) => Ok(false),
    }
}

/// Whether plugin file `text` is not the one plugin `name` was installed from: changed
/// since (a `git pull`), or never installed over a copy that was already there. An edited
/// copy of such a plugin is a conflict for a person to merge; exporting it would write the
/// old installed version over the new file.
pub fn conflicted(decree_dir: &Path, name: &str, text: &[u8]) -> Result<bool, DecreeError> {
    let path = record_path(decree_dir, name);
    match std::fs::read(&path) {
        Ok(recorded) => Ok(recorded != text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(io_error(&path, e)),
    }
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
            .unwrap()
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
            problems("---\nname: hello\nmachine: { name: hello }\nwhen: x\nx-exist: y\n---\n"),
            [
                "Some(4): unknown key `when`: a plugin has `name`, `machine`, `scripts`, `cron`, `state`, `order`, `needs` and `x-` keys",
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
                "Some(1): no `name`: every plugin has one",
                "Some(2): `machine` must be a mapping",
            ]
        );
        assert_eq!(
            problems("---\nname: hello\nmachine: {}\nscripts: { a: x, a/b: y }\n---\n"),
            ["Some(4): script path `a` is both a file and a folder"]
        );
        assert_eq!(
            problems("---\nname: hello\ncron: x\norder: { a/b.md: x, c.txt: y }\n---\n"),
            [
                "Some(3): `cron` must be a mapping of paths to their text",
                "Some(4): order file `a/b.md` must be a file name, with no `/`",
                "Some(4): order file `c.txt` must end in `.md` and not start with `.`",
            ]
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
        assert!(get(d, "machines/hello/machine.yml")
            .unwrap()
            .starts_with("name: hello\n"));
        let mode = std::fs::metadata(d.join("machines/hello/scripts/greet.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
        let mode = std::fs::metadata(d.join("machines/hello/scripts/kinds/data.txt"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o644);

        // An edit stays while the plugin is unchanged.
        put(
            d,
            "machines/hello/scripts/greet.sh",
            "#!/bin/sh\necho edited\n",
        );
        let s = sync(d).unwrap();
        assert!(s.installed.is_empty() && s.conflicts.is_empty());
        assert_eq!(
            get(d, "machines/hello/scripts/greet.sh").unwrap(),
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
            get(d, "machines/hello/scripts/greet.sh").unwrap(),
            "#!/bin/sh\necho edited\n"
        );

        // Install takes the plugin's version.
        let found = read_one(d, "hello").unwrap();
        install(d, &found.parsed.unwrap(), &found.text).unwrap();
        assert!(get(d, "machines/hello/scripts/greet.sh")
            .unwrap()
            .contains("echo bye"));
        assert!(sync(d).unwrap().conflicts.is_empty());

        // An unedited copy follows a changed plugin.
        put(
            d,
            "plugins/hello.md",
            &PLUGIN.replace("echo hi", "echo again"),
        );
        put(d, "machines/hello/scripts/added.txt", "x");
        let s = sync(d).unwrap();
        assert_eq!(s.conflicts, ["hello"], "an added file is an edit");
        std::fs::remove_file(d.join("machines/hello/scripts/added.txt")).unwrap();
        let s = sync(d).unwrap();
        assert_eq!(s.installed, ["hello"]);
        assert!(get(d, "machines/hello/scripts/greet.sh")
            .unwrap()
            .contains("echo again"));

        // A deleted plugin's copy is removed, a hand-written machine stays.
        put(d, "machines/extra.yml", "name: extra\n");
        std::fs::remove_file(d.join("plugins/hello.md")).unwrap();
        let s = sync(d).unwrap();
        assert_eq!(s.removed, ["hello"]);
        assert!(!d.join("machines/hello/machine.yml").exists());
        assert!(!d.join("machines/hello/scripts").exists());
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
            "machines/hello/machine.yml",
            &format!("# by hand\n{}", plugin.machine_text().unwrap()),
        );
        for (path, text) in &plugin.scripts {
            put(d, &format!("machines/hello/scripts/{path}"), text);
        }
        put(d, "plugins/hello.md", PLUGIN);
        let s = sync(d).unwrap();
        assert!(s.installed.is_empty() && s.conflicts.is_empty());
        assert_eq!(
            std::fs::read(record_path(d, "hello")).unwrap(),
            PLUGIN.as_bytes()
        );
        assert!(get(d, "machines/hello/machine.yml")
            .unwrap()
            .starts_with("# by hand"));

        std::fs::remove_file(record_path(d, "hello")).unwrap();
        put(d, "machines/hello/scripts/greet.sh", "different");
        assert_eq!(sync(d).unwrap().conflicts, ["hello"]);
    }

    #[test]
    fn a_deleted_plugin_with_an_edited_copy_keeps_it() {
        let tmp = project();
        let d = tmp.path();
        put(d, "plugins/hello.md", PLUGIN);
        sync(d).unwrap();
        put(d, "machines/hello/scripts/greet.sh", "edited");
        std::fs::remove_file(d.join("plugins/hello.md")).unwrap();
        let s = sync(d).unwrap();
        assert_eq!(s.kept, ["hello"]);
        assert_eq!(get(d, "machines/hello/scripts/greet.sh").unwrap(), "edited");
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
        assert!(!d.join("machines/hello/machine.yml").exists());
        assert!(s.notices()[0].starts_with("plugins/hello.md: line 4: script path `../x.sh`"));
    }

    #[test]
    fn an_installed_plugin_whose_file_turns_invalid_stays_installed() {
        let tmp = project();
        let d = tmp.path();
        put(d, "plugins/hello.md", PLUGIN);
        sync(d).unwrap();
        put(
            d,
            "plugins/hello.md",
            &PLUGIN.replace("name: hello", "name: hello\ncron: x"),
        );
        let s = sync(d).unwrap();
        assert_eq!(s.invalid.len(), 1);
        assert!(s.removed.is_empty() && s.kept.is_empty());
        assert!(d.join("machines/hello/machine.yml").exists());
        assert!(d.join("machines/hello/scripts/greet.sh").exists());
        assert_eq!(
            std::fs::read(record_path(d, "hello")).unwrap(),
            PLUGIN.as_bytes()
        );
        // Fixed again, it is unchanged: no conflict, nothing to install.
        put(d, "plugins/hello.md", PLUGIN);
        let s = sync(d).unwrap();
        assert!(s.installed.is_empty() && s.conflicts.is_empty());
    }

    #[test]
    fn a_symlinked_folder_in_scripts_is_not_followed() {
        let tmp = project();
        let d = tmp.path();
        put(d, "shared/keep.txt", "mine");
        put(d, "plugins/hello.md", PLUGIN);
        sync(d).unwrap();
        std::os::unix::fs::symlink("../../../shared", d.join("machines/hello/scripts/shared"))
            .unwrap();
        let found = read_one(d, "hello").unwrap();
        install(d, &found.parsed.unwrap(), &found.text).unwrap();
        assert_eq!(get(d, "shared/keep.txt").unwrap(), "mine");
        assert!(d.join("machines/hello/scripts/shared").is_symlink());
    }

    const KIT: &str = "\
---
name: kit
cron:
  daily.md: |
    ---
    cron: 0 9 * * *
    machine: hello
    ---
    Daily.
order:
  A01-kit.md: |
    ---
    machine: hello
    ---
    Set up.
state:
  seeds/list.txt: |
    a
scripts:
  lib.sh: |
    #!/usr/bin/env bash
    kit() { :; }
x-exist-services:
- web
---
# kit
";

    #[test]
    fn a_plugin_without_a_machine_installs_cron_state_and_order() {
        let tmp = project();
        let d = tmp.path();
        put(d, "plugins/kit.md", KIT);
        let s = sync(d).unwrap();
        assert_eq!(s.installed, ["kit"]);
        assert!(!d.join("machines/kit/machine.yml").exists());
        assert!(get(d, "machines/kit/scripts/lib.sh")
            .unwrap()
            .starts_with("#!"));
        assert!(get(d, "machines/kit/cron/daily.md")
            .unwrap()
            .contains("cron: 0 9 * * *"));
        assert!(get(d, "machines/kit/order/A01-kit.md")
            .unwrap()
            .contains("Set up."));
        assert_eq!(get(d, "machines/kit/state/seeds/list.txt").unwrap(), "a\n");
        assert_eq!(
            crate::cron::file_names(d).unwrap(),
            [("kit".to_string(), "daily.md".to_string())]
        );

        // The state and order seeds are never overwritten; the cron file is.
        put(d, "machines/kit/state/seeds/list.txt", "mine\n");
        put(d, "machines/kit/order/A01-kit.md", "edited\n");
        put(
            d,
            "plugins/kit.md",
            &KIT.replace("a\n", "b\n")
                .replace("Set up.", "Again.")
                .replace("Daily.", "Each day."),
        );
        assert_eq!(sync(d).unwrap().installed, ["kit"]);
        assert_eq!(
            get(d, "machines/kit/state/seeds/list.txt").unwrap(),
            "mine\n"
        );
        assert_eq!(get(d, "machines/kit/order/A01-kit.md").unwrap(), "edited\n");
        assert!(get(d, "machines/kit/cron/daily.md")
            .unwrap()
            .contains("Each day."));

        // An edited cron file is an edit: a plugin change over it is a conflict.
        put(
            d,
            "machines/kit/cron/daily.md",
            "---\ncron: '@reboot'\nmachine: hello\n---\n",
        );
        put(d, "plugins/kit.md", KIT);
        assert_eq!(sync(d).unwrap().conflicts, ["kit"]);

        // Export keeps the x- keys and the state and order seeds, and takes the edited cron file.
        let out = export(d, "kit").unwrap();
        assert!(out.machine.is_none());
        assert!(out.cron["daily.md"].contains("@reboot"));
        assert_eq!(out.order, Plugin::parse(KIT, None).unwrap().order);
        assert_eq!(out.state["seeds/list.txt"], "a\n");
        let text = String::from_utf8(out.to_bytes()).unwrap();
        assert!(text.contains("x-exist-services:\n- web\n"), "{text}");
        assert_eq!(Plugin::parse(&text, Some("kit")).unwrap(), out);
    }

    #[test]
    fn removing_a_plugin_keeps_its_state_and_order() {
        let tmp = project();
        let d = tmp.path();
        put(d, "plugins/kit.md", KIT);
        sync(d).unwrap();
        std::fs::remove_file(d.join("plugins/kit.md")).unwrap();
        assert_eq!(sync(d).unwrap().removed, ["kit"]);
        assert!(!d.join("machines/kit/cron").exists() && !d.join("machines/kit/scripts").exists());
        assert!(d.join("machines/kit/order/A01-kit.md").exists());
        assert!(d.join("machines/kit/state/seeds/list.txt").exists());

        // A seed already there is not written again.
        put(d, "machines/kit/order/A01-kit.md", "mine\n");
        put(d, "plugins/kit.md", KIT);
        sync(d).unwrap();
        assert_eq!(get(d, "machines/kit/order/A01-kit.md").unwrap(), "mine\n");
    }

    #[test]
    fn no_plugins_touches_nothing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let d = tmp.path();
        put(d, "machines/a/machine.yml", "name: a\n");
        let s = sync(d).unwrap();
        assert!(s.installed.is_empty() && s.removed.is_empty());
        assert!(!d.join(CACHE_DIR).exists());
    }
}
