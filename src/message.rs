use crate::config::{self, AppConfig};
use crate::error::DecreeError;
use chrono::Local;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use walkdir::WalkDir;

// Known frontmatter field names (everything else is "custom").
const KNOWN_FIELDS: &[&str] = &["id", "chain", "seq", "routine", "migration"];

/// A parsed message ID with the form `<chain>-<seq>`.
///
/// Chain format: `D<NNNN>-HHmm-<name>`
/// Full ID: `D<NNNN>-HHmm-<name>-<seq>`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageId {
    pub chain: String,
    pub seq: u32,
}

impl MessageId {
    /// Construct a new MessageId.
    pub fn new(chain: &str, seq: u32) -> Self {
        Self {
            chain: chain.to_string(),
            seq,
        }
    }

    /// Full ID string: `<chain>-<seq>`.
    pub fn full_id(&self) -> String {
        format!("{}-{}", self.chain, self.seq)
    }

    /// Parse a full message ID string like `D0001-1432-01-add-auth-0`.
    ///
    /// The last `-<number>` segment is the sequence; everything before is the chain.
    pub fn parse(s: &str) -> Result<Self, DecreeError> {
        let Some(last_dash) = s.rfind('-') else {
            return Err(DecreeError::Other(format!("invalid message ID: {s}")));
        };
        let chain = &s[..last_dash];
        let seq_str = &s[last_dash + 1..];
        let seq: u32 = seq_str
            .parse()
            .map_err(|_| DecreeError::Other(format!("invalid sequence in message ID: {s}")))?;
        if chain.is_empty() {
            return Err(DecreeError::Other(format!("empty chain in message ID: {s}")));
        }
        Ok(Self {
            chain: chain.to_string(),
            seq,
        })
    }

    /// Directory name for this message in `.decree/runs/`.
    pub fn run_dir_name(&self) -> String {
        self.full_id()
    }
}

impl std::fmt::Display for MessageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.full_id())
    }
}

// =================================================================
// Day counter and chain utilities
// =================================================================

/// Resolve the next day counter by scanning existing run directories.
///
/// Logic:
/// - Find the highest existing day counter in `.decree/runs/`
/// - Compare the current HHmm to the last entry's HHmm
/// - If current >= last, reuse the same day counter
/// - If current < last (clock wrapped midnight), increment
/// - First run starts at D0001
pub fn next_day_counter(project_root: &Path, current_hhmm: &str) -> Result<String, DecreeError> {
    let runs_dir = project_root
        .join(config::DECREE_DIR)
        .join(config::RUNS_DIR);

    if !runs_dir.exists() {
        return Ok("D0001".to_string());
    }

    let mut entries: Vec<String> = std::fs::read_dir(&runs_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| name.starts_with('D'))
        .collect();

    if entries.is_empty() {
        return Ok("D0001".to_string());
    }

    entries.sort();

    // Safe: we checked `is_empty()` above.
    let last_entry = &entries[entries.len() - 1];

    let last_day = extract_day_counter(last_entry).unwrap_or("D0001");
    let last_day_num: u32 = last_day[1..].parse().unwrap_or(1);

    let last_hhmm = extract_hhmm(last_entry).unwrap_or("0000");

    if current_hhmm >= last_hhmm {
        Ok(format!("D{:04}", last_day_num))
    } else {
        Ok(format!("D{:04}", last_day_num + 1))
    }
}

/// Extract `D<NNNN>` from a run directory name like `D0001-1432-name-0`.
fn extract_day_counter(name: &str) -> Option<&str> {
    if name.len() >= 5 && name.starts_with('D') {
        Some(&name[..5])
    } else {
        None
    }
}

/// Extract `HHmm` from a run directory name like `D0001-1432-name-0`.
fn extract_hhmm(name: &str) -> Option<&str> {
    if name.len() >= 10 && name.as_bytes()[5] == b'-' {
        Some(&name[6..10])
    } else {
        None
    }
}

/// Build a chain ID: `D<NNNN>-HHmm-<name>`.
pub fn build_chain_id(day_counter: &str, hhmm: &str, name: &str) -> String {
    format!("{}-{}-{}", day_counter, hhmm, name)
}

/// List all run directories, sorted by name (chronological).
pub fn list_runs(project_root: &Path) -> Result<Vec<String>, DecreeError> {
    let runs_dir = project_root
        .join(config::DECREE_DIR)
        .join(config::RUNS_DIR);

    if !runs_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries: Vec<String> = std::fs::read_dir(&runs_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();

    entries.sort();
    Ok(entries)
}

/// Find runs matching a prefix or full ID.
pub fn find_matching_runs(project_root: &Path, query: &str) -> Result<Vec<String>, DecreeError> {
    let runs = list_runs(project_root)?;
    let matches: Vec<String> = runs
        .into_iter()
        .filter(|name| name.starts_with(query) || name == query)
        .collect();
    Ok(matches)
}

// =================================================================
// YAML frontmatter parsing
// =================================================================

/// Parse YAML frontmatter from markdown content.
///
/// Returns `(fields_map, body)`. If no frontmatter is found, returns an
/// empty map and the entire content as body. The body is returned with
/// its original whitespace preserved.
pub fn parse_frontmatter(
    content: &str,
) -> Result<(BTreeMap<String, serde_yaml::Value>, String), DecreeError> {
    if !content.starts_with("---\n") {
        return Ok((BTreeMap::new(), content.to_string()));
    }

    let after_open = &content[4..]; // skip "---\n"

    // Find closing "---" delimiter
    let (yaml_str, body) = if let Some(pos) = after_open.find("\n---\n") {
        (&after_open[..pos], &after_open[pos + 5..]) // skip "\n---\n"
    } else if after_open.ends_with("\n---") {
        (&after_open[..after_open.len() - 4], "")
    } else if after_open.starts_with("---\n") {
        // Empty frontmatter: ---\n---\n...
        ("", &after_open[4..])
    } else if after_open == "---" {
        ("", "")
    } else {
        // No closing delimiter
        return Ok((BTreeMap::new(), content.to_string()));
    };

    let map: BTreeMap<String, serde_yaml::Value> = if yaml_str.trim().is_empty() {
        BTreeMap::new()
    } else {
        serde_yaml::from_str(yaml_str)?
    };

    Ok((map, body.to_string()))
}

// =================================================================
// Migration types and functions
// =================================================================

/// A parsed migration file from `.decree/migrations/`.
#[derive(Debug, Clone)]
pub struct MigrationFile {
    pub filename: String,
    pub routine: Option<String>,
    pub body: String,
    pub custom_fields: BTreeMap<String, serde_yaml::Value>,
}

/// List all `*.md` files in `.decree/migrations/`, sorted alphabetically.
pub fn list_migration_files(project_root: &Path) -> Result<Vec<String>, DecreeError> {
    let dir = project_root
        .join(config::DECREE_DIR)
        .join(config::MIGRATIONS_DIR);

    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut files: Vec<String> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file() && e.path().extension().is_some_and(|ext| ext == "md"))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();

    files.sort();
    Ok(files)
}

/// Read the processed migration tracker (`.decree/processed.md`).
/// Creates the file if missing.
pub fn read_processed(project_root: &Path) -> Result<HashSet<String>, DecreeError> {
    let path = project_root
        .join(config::DECREE_DIR)
        .join(config::PROCESSED_FILE);

    if !path.exists() {
        std::fs::write(&path, "")?;
        return Ok(HashSet::new());
    }

    let content = std::fs::read_to_string(&path)?;
    let set: HashSet<String> = content
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    Ok(set)
}

/// Return unprocessed migration filenames in alphabetical order.
pub fn unprocessed_migrations(project_root: &Path) -> Result<Vec<String>, DecreeError> {
    let all = list_migration_files(project_root)?;
    let processed = read_processed(project_root)?;
    Ok(all.into_iter().filter(|f| !processed.contains(f)).collect())
}

/// Append a filename to `.decree/processed.md`.
pub fn mark_processed(project_root: &Path, filename: &str) -> Result<(), DecreeError> {
    use std::io::Write;
    let path = project_root
        .join(config::DECREE_DIR)
        .join(config::PROCESSED_FILE);

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;

    writeln!(file, "{}", filename)?;
    Ok(())
}

/// Parse a migration file's content into a `MigrationFile`.
pub fn parse_migration(filename: &str, content: &str) -> Result<MigrationFile, DecreeError> {
    let (fields, body) = parse_frontmatter(content)?;

    let routine = fields.get("routine").and_then(value_as_string);

    let known: &[&str] = &["routine"];
    let custom_fields: BTreeMap<String, serde_yaml::Value> = fields
        .into_iter()
        .filter(|(k, _)| !known.contains(&k.as_str()))
        .collect();

    Ok(MigrationFile {
        filename: filename.to_string(),
        routine,
        body,
        custom_fields,
    })
}

// =================================================================
// Inbox message
// =================================================================

/// A parsed inbox message from `.decree/inbox/`.
///
/// Fields are `Option` before normalization. After `normalize()`, `id`,
/// `chain`, `seq`, and `routine` are guaranteed `Some`.
#[derive(Debug, Clone)]
pub struct InboxMessage {
    pub id: Option<String>,
    pub chain: Option<String>,
    pub seq: Option<u32>,
    pub routine: Option<String>,
    pub migration: Option<String>,
    pub body: String,
    pub custom_fields: BTreeMap<String, serde_yaml::Value>,
    pub filename: String,
}

impl InboxMessage {
    /// Parse an inbox message from its filename and raw content.
    pub fn parse(filename: &str, content: &str) -> Result<Self, DecreeError> {
        let (fields, body) = parse_frontmatter(content)?;

        let id = fields.get("id").and_then(value_as_string);
        let chain = fields.get("chain").and_then(value_as_string);
        let seq = fields.get("seq").and_then(value_as_u32);
        let routine = fields.get("routine").and_then(value_as_string);
        let migration = fields.get("migration").and_then(value_as_string);

        let custom_fields: BTreeMap<String, serde_yaml::Value> = fields
            .into_iter()
            .filter(|(k, _)| !KNOWN_FIELDS.contains(&k.as_str()))
            .collect();

        Ok(Self {
            id,
            chain,
            seq,
            routine,
            migration,
            body,
            custom_fields,
            filename: filename.to_string(),
        })
    }

    /// Read and parse an inbox message from the filesystem.
    pub fn from_file(project_root: &Path, filename: &str) -> Result<Self, DecreeError> {
        let path = project_root
            .join(config::DECREE_DIR)
            .join(config::INBOX_DIR)
            .join(filename);

        let content = std::fs::read_to_string(&path)?;
        Self::parse(filename, &content)
    }

    /// Whether all required fields are present (no normalization needed).
    pub fn is_complete(&self) -> bool {
        self.id.is_some()
            && self.chain.is_some()
            && self.seq.is_some()
            && self.routine.is_some()
    }

    /// Normalize the message, filling in missing fields.
    ///
    /// Returns `true` if the message was modified and should be rewritten.
    ///
    /// `ai_router` is an optional callback for AI-based routine selection.
    /// It receives the populated router prompt and should return the routine name.
    pub fn normalize(
        &mut self,
        project_root: &Path,
        config: &AppConfig,
        ai_router: Option<&dyn Fn(&str) -> Result<String, DecreeError>>,
    ) -> Result<bool, DecreeError> {
        if self.is_complete() {
            return Ok(false);
        }

        // 1. Derive chain and seq from filename if missing
        if self.chain.is_none() || self.seq.is_none() {
            if let Some((chain, seq)) = chain_seq_from_filename(&self.filename) {
                if self.chain.is_none() {
                    self.chain = Some(chain);
                }
                if self.seq.is_none() {
                    self.seq = Some(seq);
                }
            }
        }

        // 2. Generate new chain if still missing
        if self.chain.is_none() {
            let now = Local::now();
            let hhmm = now.format("%H%M").to_string();
            let day = next_day_counter(project_root, &hhmm)?;
            let name = self
                .migration
                .as_deref()
                .map(|m| m.trim_end_matches(".md"))
                .unwrap_or("message");
            self.chain = Some(build_chain_id(&day, &hhmm, name));
        }

        // 3. Default seq to 0 if still missing
        if self.seq.is_none() {
            self.seq = Some(0);
        }

        // 4. Recompute id from chain + seq
        let chain = self
            .chain
            .as_ref()
            .ok_or_else(|| DecreeError::Other("chain not set during normalization".into()))?;
        let seq = self
            .seq
            .ok_or_else(|| DecreeError::Other("seq not set during normalization".into()))?;
        self.id = Some(format!("{}-{}", chain, seq));

        // 5. Routine selection
        if self.routine.is_none() {
            self.routine = Some(select_routine(project_root, config, &self.body, ai_router)?);
        }

        Ok(true)
    }

    /// Serialize the message to markdown with YAML frontmatter.
    pub fn serialize(&self) -> String {
        let mut map = serde_yaml::Mapping::new();
        let str_key = |k: &str| serde_yaml::Value::String(k.into());
        let str_val = |v: &str| serde_yaml::Value::String(v.into());

        if let Some(ref v) = self.id {
            map.insert(str_key("id"), str_val(v));
        }
        if let Some(ref v) = self.chain {
            map.insert(str_key("chain"), str_val(v));
        }
        if let Some(seq) = self.seq {
            let seq_val = serde_yaml::to_value(seq)
                .unwrap_or_else(|_| serde_yaml::Value::String(seq.to_string()));
            map.insert(str_key("seq"), seq_val);
        }
        if let Some(ref v) = self.routine {
            map.insert(str_key("routine"), str_val(v));
        }
        if let Some(ref v) = self.migration {
            map.insert(str_key("migration"), str_val(v));
        }

        for (k, v) in &self.custom_fields {
            map.insert(str_key(k), v.clone());
        }

        let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(map))
            .unwrap_or_default();

        if self.body.is_empty() {
            format!("---\n{}---\n", yaml)
        } else {
            format!("---\n{}---\n{}", yaml, self.body)
        }
    }

    /// Write the message to `.decree/inbox/`.
    pub fn write_to_inbox(&self, project_root: &Path) -> Result<(), DecreeError> {
        let path = project_root
            .join(config::DECREE_DIR)
            .join(config::INBOX_DIR)
            .join(&self.filename);

        std::fs::write(&path, self.serialize())?;
        Ok(())
    }
}

/// List all `*.md` files in `.decree/inbox/`, sorted alphabetically.
pub fn list_inbox_messages(project_root: &Path) -> Result<Vec<String>, DecreeError> {
    let dir = project_root
        .join(config::DECREE_DIR)
        .join(config::INBOX_DIR);

    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut files: Vec<String> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file() && e.path().extension().is_some_and(|ext| ext == "md"))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();

    files.sort();
    Ok(files)
}

/// Extract chain and seq from an inbox message filename.
///
/// Expects format `<chain>-<seq>.md` (e.g., `D0001-1432-01-add-auth-0.md`).
fn chain_seq_from_filename(filename: &str) -> Option<(String, u32)> {
    let stem = filename.strip_suffix(".md")?;
    let last_dash = stem.rfind('-')?;
    let chain = &stem[..last_dash];
    let seq_str = &stem[last_dash + 1..];
    let seq: u32 = seq_str.parse().ok()?;
    if chain.is_empty() {
        return None;
    }
    Some((chain.to_string(), seq))
}

// =================================================================
// Routine listing and router
// =================================================================

/// Information about an available routine.
#[derive(Debug, Clone)]
pub struct RoutineInfo {
    /// Routine name (relative path without extension, e.g., "develop").
    pub name: String,
    /// Description extracted from comment header.
    pub description: String,
}

/// List available routines, respecting the config registry.
///
/// - With a `routines` section: only enabled project-local routines are listed.
/// - Without a `routines` section (legacy): all filesystem routines are listed.
/// - With `routine_source`: enabled shared routines are also included.
pub fn list_routines(project_root: &Path, config: &AppConfig) -> Result<Vec<RoutineInfo>, DecreeError> {
    let routines_dir = project_root
        .join(config::DECREE_DIR)
        .join(config::ROUTINES_DIR);

    let mut routines = Vec::new();

    if let Some(ref registry) = config.routines {
        // Strict mode: only enabled routines from registry
        for (name, entry) in registry {
            if !entry.is_active() {
                continue;
            }
            if let Ok(path) = crate::routine::find_routine_script(&routines_dir, name) {
                let content = std::fs::read_to_string(&path)?;
                let description = extract_routine_description(&content);
                routines.push(RoutineInfo {
                    name: name.clone(),
                    description,
                });
            }
        }
    } else {
        // Legacy mode: all filesystem routines
        routines = scan_routines_dir(&routines_dir)?;
    }

    // Add enabled shared routines
    if let Some(shared_dir) = config.resolved_routine_source() {
        if let Some(ref shared_registry) = config.shared_routines {
            for (name, entry) in shared_registry {
                if !entry.is_active() {
                    continue;
                }
                // Skip if already listed from project-local (precedence)
                if routines.iter().any(|r| r.name == *name) {
                    continue;
                }
                if let Ok(path) = crate::routine::find_routine_script(&shared_dir, name) {
                    let content = std::fs::read_to_string(&path)?;
                    let description = extract_routine_description(&content);
                    routines.push(RoutineInfo {
                        name: name.clone(),
                        description,
                    });
                }
            }
        }
    }

    routines.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(routines)
}

/// Scan a routines directory for all script files (legacy mode).
fn scan_routines_dir(routines_dir: &Path) -> Result<Vec<RoutineInfo>, DecreeError> {
    if !routines_dir.exists() {
        return Ok(Vec::new());
    }

    let mut routines = Vec::new();

    for entry in WalkDir::new(routines_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        let rel = path
            .strip_prefix(routines_dir)
            .map_err(|e| DecreeError::Other(e.to_string()))?;

        let name = rel.with_extension("").to_string_lossy().to_string();
        if name.is_empty() {
            continue;
        }

        let content = std::fs::read_to_string(path)?;
        let description = extract_routine_description(&content);

        routines.push(RoutineInfo { name, description });
    }

    Ok(routines)
}

/// Extract a description from a routine script's comment header.
///
/// Expected format:
/// ```text
/// #!/usr/bin/env bash
/// # Title
/// #
/// # Description line 1
/// # Description line 2
/// ```
pub fn extract_routine_description(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();

    // Skip shebang if present
    let start = if lines.first().is_some_and(|l| l.starts_with("#!")) {
        1
    } else {
        0
    };

    // Skip title line (# Title) and blank comment (#)
    let desc_start = start + 2;

    if desc_start >= lines.len() {
        return String::new();
    }

    let mut desc_lines = Vec::new();
    for line in &lines[desc_start..] {
        if let Some(text) = line.strip_prefix("# ") {
            desc_lines.push(text);
        } else {
            break;
        }
    }

    desc_lines.join(" ")
}

/// Build the router prompt for AI-based routine selection.
///
/// Reads `.decree/router.md` and populates `{routines}` and `{message}`.
pub fn build_router_prompt(
    project_root: &Path,
    routines: &[RoutineInfo],
    message_body: &str,
) -> Result<String, DecreeError> {
    let router_path = project_root
        .join(config::DECREE_DIR)
        .join(config::ROUTER_FILE);

    let template = std::fs::read_to_string(&router_path)?;

    let routines_text: String = routines
        .iter()
        .map(|r| {
            if r.description.is_empty() {
                format!("- **{}**", r.name)
            } else {
                format!("- **{}**: {}", r.name, r.description)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = template
        .replace("{routines}", &routines_text)
        .replace("{message}", message_body);

    Ok(prompt)
}

/// Select a routine for a message.
///
/// Fallback chain: AI router → config `default_routine` → `"develop"`.
fn select_routine(
    project_root: &Path,
    config: &AppConfig,
    message_body: &str,
    ai_router: Option<&dyn Fn(&str) -> Result<String, DecreeError>>,
) -> Result<String, DecreeError> {
    // Try AI router if provided
    if let Some(router_fn) = ai_router {
        let routines = list_routines(project_root, config)?;
        if !routines.is_empty() {
            if let Ok(prompt) = build_router_prompt(project_root, &routines, message_body) {
                if let Ok(selected) = router_fn(&prompt) {
                    let trimmed = selected.trim().to_string();
                    if routines.iter().any(|r| r.name == trimmed) {
                        return Ok(trimmed);
                    }
                }
            }
        }
    }

    // Fallback: config default
    if !config.default_routine.is_empty() {
        return Ok(config.default_routine.clone());
    }

    // Ultimate fallback
    Ok("develop".to_string())
}

// =================================================================
// Helpers
// =================================================================

fn value_as_string(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn value_as_u32(v: &serde_yaml::Value) -> Option<u32> {
    match v {
        serde_yaml::Value::Number(n) => n.as_u64().map(|n| n as u32),
        serde_yaml::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

// =================================================================
// Tests
// =================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // --- MessageId tests (existing) ---

    #[test]
    fn test_parse_message_id() {
        let id = MessageId::parse("D0001-1432-01-add-auth-0").unwrap();
        assert_eq!(id.chain, "D0001-1432-01-add-auth");
        assert_eq!(id.seq, 0);
        assert_eq!(id.full_id(), "D0001-1432-01-add-auth-0");
    }

    #[test]
    fn test_parse_followup() {
        let id = MessageId::parse("D0001-1432-01-add-auth-1").unwrap();
        assert_eq!(id.seq, 1);
    }

    #[test]
    fn test_parse_invalid() {
        assert!(MessageId::parse("invalid").is_err());
        assert!(MessageId::parse("-0").is_err());
    }

    #[test]
    fn test_display() {
        let id = MessageId::new("D0001-1432-develop", 0);
        assert_eq!(format!("{id}"), "D0001-1432-develop-0");
    }

    #[test]
    fn test_extract_day_counter() {
        assert_eq!(extract_day_counter("D0001-1432-test-0"), Some("D0001"));
        assert_eq!(extract_day_counter("D0042-0900-foo-1"), Some("D0042"));
    }

    #[test]
    fn test_extract_hhmm() {
        assert_eq!(extract_hhmm("D0001-1432-test-0"), Some("1432"));
        assert_eq!(extract_hhmm("D0001-0900-foo-0"), Some("0900"));
    }

    #[test]
    fn test_build_chain_id() {
        assert_eq!(
            build_chain_id("D0001", "1432", "01-add-auth"),
            "D0001-1432-01-add-auth"
        );
    }

    // --- Frontmatter parsing tests ---

    #[test]
    fn test_parse_frontmatter_none() {
        let (map, body) = parse_frontmatter("Just some text.\n").unwrap();
        assert!(map.is_empty());
        assert_eq!(body, "Just some text.\n");
    }

    #[test]
    fn test_parse_frontmatter_empty() {
        let (map, body) = parse_frontmatter("---\n---\n").unwrap();
        assert!(map.is_empty());
        assert_eq!(body, "");
    }

    #[test]
    fn test_parse_frontmatter_with_fields() {
        let content = "---\nroutine: develop\n---\nHello world.\n";
        let (map, body) = parse_frontmatter(content).unwrap();
        assert_eq!(
            map.get("routine"),
            Some(&serde_yaml::Value::String("develop".into()))
        );
        assert_eq!(body, "Hello world.\n");
    }

    #[test]
    fn test_parse_frontmatter_full_message() {
        let content = "---\n\
            id: D0001-1432-01-add-auth-0\n\
            chain: D0001-1432-01-add-auth\n\
            seq: 0\n\
            routine: develop\n\
            migration: 01-add-auth.md\n\
            ---\n\
            # Add Auth\n\
            \n\
            Add authentication.\n";

        let (map, body) = parse_frontmatter(content).unwrap();
        assert_eq!(map.len(), 5);
        assert_eq!(
            map.get("id"),
            Some(&serde_yaml::Value::String(
                "D0001-1432-01-add-auth-0".into()
            ))
        );
        assert!(body.starts_with("# Add Auth"));
    }

    #[test]
    fn test_parse_frontmatter_no_trailing_newline() {
        let content = "---\nroutine: develop\n---\nHello";
        let (map, body) = parse_frontmatter(content).unwrap();
        assert_eq!(
            map.get("routine"),
            Some(&serde_yaml::Value::String("develop".into()))
        );
        assert_eq!(body, "Hello");
    }

    #[test]
    fn test_parse_frontmatter_empty_body() {
        let content = "---\nroutine: develop\n---\n";
        let (map, body) = parse_frontmatter(content).unwrap();
        assert_eq!(
            map.get("routine"),
            Some(&serde_yaml::Value::String("develop".into()))
        );
        assert_eq!(body, "");
    }

    #[test]
    fn test_parse_frontmatter_no_closing() {
        let content = "---\nroutine: develop\nNo closing delimiter.\n";
        let (map, body) = parse_frontmatter(content).unwrap();
        // Treated as no frontmatter
        assert!(map.is_empty());
        assert_eq!(body, content);
    }

    #[test]
    fn test_parse_frontmatter_preserves_body_whitespace() {
        let content = "---\nroutine: develop\n---\n\nHello\n\nWorld\n";
        let (_, body) = parse_frontmatter(content).unwrap();
        assert_eq!(body, "\nHello\n\nWorld\n");
    }

    #[test]
    fn test_parse_frontmatter_custom_fields() {
        let content = "---\nroutine: develop\npriority: high\n---\nBody.\n";
        let (map, _) = parse_frontmatter(content).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(
            map.get("priority"),
            Some(&serde_yaml::Value::String("high".into()))
        );
    }

    // --- Migration tests ---

    fn setup_decree_dir(dir: &TempDir) {
        let decree = dir.path().join(".decree");
        std::fs::create_dir_all(decree.join("migrations")).unwrap();
        std::fs::create_dir_all(decree.join("inbox")).unwrap();
        std::fs::create_dir_all(decree.join("routines")).unwrap();
        std::fs::create_dir_all(decree.join("runs")).unwrap();
        std::fs::write(decree.join("processed.md"), "").unwrap();
        std::fs::write(
            decree.join("config.yml"),
            "commands:\n  ai_router: test\n  ai_interactive: test\n",
        )
        .unwrap();
    }

    #[test]
    fn test_list_migration_files_empty() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let files = list_migration_files(dir.path()).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_list_migration_files_sorted() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let mig_dir = dir.path().join(".decree/migrations");
        std::fs::write(mig_dir.join("03-api.md"), "").unwrap();
        std::fs::write(mig_dir.join("01-auth.md"), "").unwrap();
        std::fs::write(mig_dir.join("02-db.md"), "").unwrap();
        // Non-md file should be excluded
        std::fs::write(mig_dir.join("notes.txt"), "").unwrap();

        let files = list_migration_files(dir.path()).unwrap();
        assert_eq!(files, vec!["01-auth.md", "02-db.md", "03-api.md"]);
    }

    #[test]
    fn test_list_migration_files_no_dir() {
        let dir = TempDir::new().unwrap();
        // No .decree at all
        let files = list_migration_files(dir.path()).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_read_processed_empty() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let processed = read_processed(dir.path()).unwrap();
        assert!(processed.is_empty());
    }

    #[test]
    fn test_read_processed_with_entries() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        std::fs::write(
            dir.path().join(".decree/processed.md"),
            "01-auth.md\n02-db.md\n",
        )
        .unwrap();

        let processed = read_processed(dir.path()).unwrap();
        assert_eq!(processed.len(), 2);
        assert!(processed.contains("01-auth.md"));
        assert!(processed.contains("02-db.md"));
    }

    #[test]
    fn test_read_processed_creates_if_missing() {
        let dir = TempDir::new().unwrap();
        let decree = dir.path().join(".decree");
        std::fs::create_dir_all(&decree).unwrap();
        // No processed.md file
        assert!(!decree.join("processed.md").exists());

        let processed = read_processed(dir.path()).unwrap();
        assert!(processed.is_empty());
        assert!(decree.join("processed.md").exists());
    }

    #[test]
    fn test_unprocessed_migrations() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let mig_dir = dir.path().join(".decree/migrations");
        std::fs::write(mig_dir.join("01-auth.md"), "").unwrap();
        std::fs::write(mig_dir.join("02-db.md"), "").unwrap();
        std::fs::write(mig_dir.join("03-api.md"), "").unwrap();
        std::fs::write(
            dir.path().join(".decree/processed.md"),
            "01-auth.md\n",
        )
        .unwrap();

        let unprocessed = unprocessed_migrations(dir.path()).unwrap();
        assert_eq!(unprocessed, vec!["02-db.md", "03-api.md"]);
    }

    #[test]
    fn test_unprocessed_migrations_all_processed() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let mig_dir = dir.path().join(".decree/migrations");
        std::fs::write(mig_dir.join("01-auth.md"), "").unwrap();
        std::fs::write(
            dir.path().join(".decree/processed.md"),
            "01-auth.md\n",
        )
        .unwrap();

        let unprocessed = unprocessed_migrations(dir.path()).unwrap();
        assert!(unprocessed.is_empty());
    }

    #[test]
    fn test_mark_processed() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        mark_processed(dir.path(), "01-auth.md").unwrap();
        mark_processed(dir.path(), "02-db.md").unwrap();

        let processed = read_processed(dir.path()).unwrap();
        assert_eq!(processed.len(), 2);
        assert!(processed.contains("01-auth.md"));
        assert!(processed.contains("02-db.md"));
    }

    #[test]
    fn test_parse_migration_with_frontmatter() {
        let content = "---\nroutine: rust-develop\n---\nAdd auth.\n";
        let mig = parse_migration("01-auth.md", content).unwrap();
        assert_eq!(mig.filename, "01-auth.md");
        assert_eq!(mig.routine, Some("rust-develop".to_string()));
        assert_eq!(mig.body, "Add auth.\n");
    }

    #[test]
    fn test_parse_migration_without_frontmatter() {
        let content = "Add auth.\n";
        let mig = parse_migration("01-auth.md", content).unwrap();
        assert_eq!(mig.routine, None);
        assert_eq!(mig.body, "Add auth.\n");
    }

    #[test]
    fn test_parse_migration_empty_body() {
        let content = "---\nroutine: develop\n---\n";
        let mig = parse_migration("01-empty.md", content).unwrap();
        assert_eq!(mig.routine, Some("develop".to_string()));
        assert_eq!(mig.body, "");
    }

    // --- InboxMessage tests ---

    #[test]
    fn test_inbox_parse_full_message() {
        let content = "---\n\
            id: D0001-1432-01-add-auth-0\n\
            chain: D0001-1432-01-add-auth\n\
            seq: 0\n\
            routine: develop\n\
            migration: 01-add-auth.md\n\
            ---\n\
            Add auth.\n";

        let msg = InboxMessage::parse("D0001-1432-01-add-auth-0.md", content).unwrap();
        assert_eq!(msg.id.as_deref(), Some("D0001-1432-01-add-auth-0"));
        assert_eq!(msg.chain.as_deref(), Some("D0001-1432-01-add-auth"));
        assert_eq!(msg.seq, Some(0));
        assert_eq!(msg.routine.as_deref(), Some("develop"));
        assert_eq!(msg.migration.as_deref(), Some("01-add-auth.md"));
        assert!(msg.is_complete());
    }

    #[test]
    fn test_inbox_parse_bare_message() {
        let content = "Fix type errors in src/auth.rs.\n";
        let msg = InboxMessage::parse("fix-errors.md", content).unwrap();
        assert!(msg.id.is_none());
        assert!(msg.chain.is_none());
        assert!(msg.seq.is_none());
        assert!(msg.routine.is_none());
        assert_eq!(msg.body, "Fix type errors in src/auth.rs.\n");
        assert!(!msg.is_complete());
    }

    #[test]
    fn test_inbox_parse_partial_frontmatter() {
        let content = "---\nroutine: rust-develop\n---\nFix errors.\n";
        let msg = InboxMessage::parse("D0001-1432-fix-0.md", content).unwrap();
        assert!(msg.id.is_none());
        assert!(msg.chain.is_none());
        assert!(msg.seq.is_none());
        assert_eq!(msg.routine.as_deref(), Some("rust-develop"));
        assert!(!msg.is_complete());
    }

    #[test]
    fn test_inbox_parse_custom_fields() {
        let content = "---\nroutine: develop\npriority: high\ntags: urgent\n---\nBody.\n";
        let msg = InboxMessage::parse("test.md", content).unwrap();
        assert_eq!(msg.routine.as_deref(), Some("develop"));
        assert_eq!(msg.custom_fields.len(), 2);
        assert_eq!(
            msg.custom_fields.get("priority"),
            Some(&serde_yaml::Value::String("high".into()))
        );
        assert_eq!(
            msg.custom_fields.get("tags"),
            Some(&serde_yaml::Value::String("urgent".into()))
        );
    }

    #[test]
    fn test_inbox_parse_empty_body_valid() {
        let content = "---\nroutine: develop\n---\n";
        let msg = InboxMessage::parse("test.md", content).unwrap();
        assert_eq!(msg.body, "");
        assert_eq!(msg.routine.as_deref(), Some("develop"));
    }

    #[test]
    fn test_inbox_parse_entirely_empty() {
        let msg = InboxMessage::parse("test.md", "").unwrap();
        assert!(msg.id.is_none());
        assert_eq!(msg.body, "");
    }

    // --- chain_seq_from_filename tests ---

    #[test]
    fn test_chain_seq_from_filename_valid() {
        let (chain, seq) = chain_seq_from_filename("D0001-1432-01-add-auth-0.md").unwrap();
        assert_eq!(chain, "D0001-1432-01-add-auth");
        assert_eq!(seq, 0);
    }

    #[test]
    fn test_chain_seq_from_filename_followup() {
        let (chain, seq) = chain_seq_from_filename("D0001-1432-01-add-auth-3.md").unwrap();
        assert_eq!(chain, "D0001-1432-01-add-auth");
        assert_eq!(seq, 3);
    }

    #[test]
    fn test_chain_seq_from_filename_no_md() {
        assert!(chain_seq_from_filename("D0001-1432-test-0.txt").is_none());
    }

    #[test]
    fn test_chain_seq_from_filename_no_seq() {
        assert!(chain_seq_from_filename("no-seq.md").is_none());
    }

    #[test]
    fn test_chain_seq_from_filename_empty_chain() {
        assert!(chain_seq_from_filename("-0.md").is_none());
    }

    // --- Normalization tests ---

    #[test]
    fn test_normalize_complete_message_not_modified() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig::default();

        let mut msg = InboxMessage {
            id: Some("D0001-1432-test-0".into()),
            chain: Some("D0001-1432-test".into()),
            seq: Some(0),
            routine: Some("develop".into()),
            migration: None,
            body: "Test.".into(),
            custom_fields: BTreeMap::new(),
            filename: "D0001-1432-test-0.md".into(),
        };

        let modified = msg.normalize(dir.path(), &config, None).unwrap();
        assert!(!modified, "complete message should not be modified");
    }

    #[test]
    fn test_normalize_from_filename() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig::default();

        let mut msg = InboxMessage::parse(
            "D0001-1432-01-add-auth-0.md",
            "Add auth.\n",
        )
        .unwrap();

        let modified = msg.normalize(dir.path(), &config, None).unwrap();
        assert!(modified);
        assert_eq!(msg.chain.as_deref(), Some("D0001-1432-01-add-auth"));
        assert_eq!(msg.seq, Some(0));
        assert_eq!(msg.id.as_deref(), Some("D0001-1432-01-add-auth-0"));
        // Routine falls back to config default
        assert_eq!(msg.routine.as_deref(), Some("develop"));
    }

    #[test]
    fn test_normalize_preserves_frontmatter_over_filename() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig::default();

        let content = "---\nchain: my-chain\nseq: 5\n---\nBody.\n";
        let mut msg = InboxMessage::parse("D0001-1432-01-add-auth-0.md", content).unwrap();

        msg.normalize(dir.path(), &config, None).unwrap();
        // Frontmatter values should take priority over filename
        assert_eq!(msg.chain.as_deref(), Some("my-chain"));
        assert_eq!(msg.seq, Some(5));
        assert_eq!(msg.id.as_deref(), Some("my-chain-5"));
    }

    #[test]
    fn test_normalize_generates_chain_when_missing() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig::default();

        // Filename without chain-seq pattern
        let mut msg = InboxMessage::parse("random-name.md", "Body.\n").unwrap();

        let modified = msg.normalize(dir.path(), &config, None).unwrap();
        assert!(modified);
        assert!(msg.chain.is_some());
        assert!(msg.chain.as_ref().unwrap().starts_with("D0001-"));
        assert_eq!(msg.seq, Some(0));
        assert!(msg.id.is_some());
    }

    #[test]
    fn test_normalize_routine_fallback_config_default() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig {
            default_routine: "rust-develop".to_string(),
            ..AppConfig::default()
        };

        let mut msg =
            InboxMessage::parse("D0001-1432-test-0.md", "Body.\n").unwrap();
        msg.normalize(dir.path(), &config, None).unwrap();
        assert_eq!(msg.routine.as_deref(), Some("rust-develop"));
    }

    #[test]
    fn test_normalize_routine_ultimate_fallback() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig {
            default_routine: String::new(),
            ..AppConfig::default()
        };

        let mut msg =
            InboxMessage::parse("D0001-1432-test-0.md", "Body.\n").unwrap();
        msg.normalize(dir.path(), &config, None).unwrap();
        assert_eq!(msg.routine.as_deref(), Some("develop"));
    }

    #[test]
    fn test_normalize_routine_ai_selection() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig::default();

        // Write a routine and router.md
        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\n# Develop\n#\n# General purpose.\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".decree/routines/rust-develop.sh"),
            "#!/usr/bin/env bash\n# Rust Develop\n#\n# Rust specific.\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".decree/router.md"),
            "Select routine.\n\n{routines}\n\n{message}\n",
        )
        .unwrap();

        // AI selects "rust-develop"
        let ai_fn = |_prompt: &str| -> Result<String, DecreeError> {
            Ok("rust-develop".to_string())
        };

        let mut msg =
            InboxMessage::parse("D0001-1432-test-0.md", "Add Rust auth.\n").unwrap();
        msg.normalize(dir.path(), &config, Some(&ai_fn)).unwrap();
        assert_eq!(msg.routine.as_deref(), Some("rust-develop"));
    }

    #[test]
    fn test_normalize_routine_ai_invalid_falls_back() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig::default();

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\n# Develop\n#\n# General.\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".decree/router.md"),
            "{routines}\n{message}\n",
        )
        .unwrap();

        // AI returns a non-existent routine
        let ai_fn = |_prompt: &str| -> Result<String, DecreeError> {
            Ok("nonexistent-routine".to_string())
        };

        let mut msg =
            InboxMessage::parse("D0001-1432-test-0.md", "Body.\n").unwrap();
        msg.normalize(dir.path(), &config, Some(&ai_fn)).unwrap();
        // Should fall back to config default
        assert_eq!(msg.routine.as_deref(), Some("develop"));
    }

    #[test]
    fn test_normalize_custom_fields_preserved() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig::default();

        let content = "---\npriority: high\ntags: urgent\n---\nBody.\n";
        let mut msg = InboxMessage::parse("D0001-1432-test-0.md", content).unwrap();

        msg.normalize(dir.path(), &config, None).unwrap();
        assert_eq!(msg.custom_fields.len(), 2);
        assert_eq!(
            msg.custom_fields.get("priority"),
            Some(&serde_yaml::Value::String("high".into()))
        );
    }

    #[test]
    fn test_normalize_migration_field_preserved() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let config = AppConfig::default();

        let content = "---\nmigration: 01-auth.md\n---\nBody.\n";
        let mut msg = InboxMessage::parse("D0001-1432-01-auth-0.md", content).unwrap();

        msg.normalize(dir.path(), &config, None).unwrap();
        assert_eq!(msg.migration.as_deref(), Some("01-auth.md"));
    }

    // --- Serialization tests ---

    #[test]
    fn test_serialize_full_message() {
        let msg = InboxMessage {
            id: Some("D0001-1432-test-0".into()),
            chain: Some("D0001-1432-test".into()),
            seq: Some(0),
            routine: Some("develop".into()),
            migration: Some("01-test.md".into()),
            body: "Hello.\n".into(),
            custom_fields: BTreeMap::new(),
            filename: "D0001-1432-test-0.md".into(),
        };

        let output = msg.serialize();
        assert!(output.starts_with("---\n"));
        assert!(output.contains("id: D0001-1432-test-0"));
        assert!(output.contains("chain: D0001-1432-test"));
        assert!(output.contains("seq: 0"));
        assert!(output.contains("routine: develop"));
        assert!(output.contains("migration: 01-test.md"));
        assert!(output.ends_with("Hello.\n"));
    }

    #[test]
    fn test_serialize_with_custom_fields() {
        let mut custom = BTreeMap::new();
        custom.insert(
            "priority".to_string(),
            serde_yaml::Value::String("high".into()),
        );

        let msg = InboxMessage {
            id: Some("D0001-1432-test-0".into()),
            chain: Some("D0001-1432-test".into()),
            seq: Some(0),
            routine: Some("develop".into()),
            migration: None,
            body: "Body.\n".into(),
            custom_fields: custom,
            filename: "D0001-1432-test-0.md".into(),
        };

        let output = msg.serialize();
        assert!(output.contains("priority: high"));
    }

    #[test]
    fn test_serialize_empty_body() {
        let msg = InboxMessage {
            id: Some("D0001-1432-test-0".into()),
            chain: Some("D0001-1432-test".into()),
            seq: Some(0),
            routine: Some("develop".into()),
            migration: None,
            body: String::new(),
            custom_fields: BTreeMap::new(),
            filename: "D0001-1432-test-0.md".into(),
        };

        let output = msg.serialize();
        assert!(output.starts_with("---\n"));
        assert!(output.ends_with("---\n"));
    }

    #[test]
    fn test_serialize_roundtrip() {
        let original = InboxMessage {
            id: Some("D0001-1432-test-0".into()),
            chain: Some("D0001-1432-test".into()),
            seq: Some(0),
            routine: Some("develop".into()),
            migration: Some("01-test.md".into()),
            body: "Hello world.\n".into(),
            custom_fields: BTreeMap::new(),
            filename: "D0001-1432-test-0.md".into(),
        };

        let serialized = original.serialize();
        let parsed = InboxMessage::parse("D0001-1432-test-0.md", &serialized).unwrap();

        assert_eq!(parsed.id, original.id);
        assert_eq!(parsed.chain, original.chain);
        assert_eq!(parsed.seq, original.seq);
        assert_eq!(parsed.routine, original.routine);
        assert_eq!(parsed.migration, original.migration);
        assert_eq!(parsed.body, original.body);
    }

    // --- write_to_inbox / from_file tests ---

    #[test]
    fn test_write_and_read_inbox() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        let msg = InboxMessage {
            id: Some("D0001-1432-test-0".into()),
            chain: Some("D0001-1432-test".into()),
            seq: Some(0),
            routine: Some("develop".into()),
            migration: None,
            body: "Hello.\n".into(),
            custom_fields: BTreeMap::new(),
            filename: "D0001-1432-test-0.md".into(),
        };

        msg.write_to_inbox(dir.path()).unwrap();

        let read_back = InboxMessage::from_file(dir.path(), "D0001-1432-test-0.md").unwrap();
        assert_eq!(read_back.id.as_deref(), Some("D0001-1432-test-0"));
        assert_eq!(read_back.chain.as_deref(), Some("D0001-1432-test"));
        assert_eq!(read_back.seq, Some(0));
        assert_eq!(read_back.routine.as_deref(), Some("develop"));
        assert_eq!(read_back.body, "Hello.\n");
    }

    // --- list_inbox_messages tests ---

    #[test]
    fn test_list_inbox_messages_empty() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let msgs = list_inbox_messages(dir.path()).unwrap();
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_list_inbox_messages_sorted() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        let inbox = dir.path().join(".decree/inbox");
        std::fs::write(inbox.join("D0001-1432-beta-0.md"), "").unwrap();
        std::fs::write(inbox.join("D0001-1432-alpha-0.md"), "").unwrap();
        // Non-md should be excluded
        std::fs::write(inbox.join("notes.txt"), "").unwrap();

        let msgs = list_inbox_messages(dir.path()).unwrap();
        assert_eq!(
            msgs,
            vec!["D0001-1432-alpha-0.md", "D0001-1432-beta-0.md"]
        );
    }

    // --- Routine listing tests ---

    #[test]
    fn test_extract_routine_description_standard() {
        let content = "#!/usr/bin/env bash\n# Develop\n#\n# General-purpose development.\n";
        let desc = extract_routine_description(content);
        assert_eq!(desc, "General-purpose development.");
    }

    #[test]
    fn test_extract_routine_description_multiline() {
        let content =
            "#!/usr/bin/env bash\n# Develop\n#\n# Line one.\n# Line two.\nset -euo pipefail\n";
        let desc = extract_routine_description(content);
        assert_eq!(desc, "Line one. Line two.");
    }

    #[test]
    fn test_extract_routine_description_no_desc() {
        let content = "#!/usr/bin/env bash\n# Title\n#\nset -euo pipefail\n";
        let desc = extract_routine_description(content);
        assert_eq!(desc, "");
    }

    #[test]
    fn test_extract_routine_description_no_shebang() {
        let content = "# Title\n#\n# Description here.\n";
        let desc = extract_routine_description(content);
        assert_eq!(desc, "Description here.");
    }

    #[test]
    fn test_list_routines() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        let routines_dir = dir.path().join(".decree/routines");
        std::fs::write(
            routines_dir.join("develop.sh"),
            "#!/usr/bin/env bash\n# Develop\n#\n# General purpose.\n",
        )
        .unwrap();
        std::fs::write(
            routines_dir.join("rust-develop.sh"),
            "#!/usr/bin/env bash\n# Rust Develop\n#\n# Rust specific.\n",
        )
        .unwrap();

        let config = AppConfig::default();
        let routines = list_routines(dir.path(), &config).unwrap();
        assert_eq!(routines.len(), 2);
        assert_eq!(routines[0].name, "develop");
        assert_eq!(routines[0].description, "General purpose.");
        assert_eq!(routines[1].name, "rust-develop");
        assert_eq!(routines[1].description, "Rust specific.");
    }

    #[test]
    fn test_list_routines_nested() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        let routines_dir = dir.path().join(".decree/routines");
        std::fs::create_dir_all(routines_dir.join("hooks")).unwrap();
        std::fs::write(
            routines_dir.join("develop.sh"),
            "#!/usr/bin/env bash\n# Develop\n#\n# General.\n",
        )
        .unwrap();
        std::fs::write(
            routines_dir.join("hooks/git-baseline.sh"),
            "#!/usr/bin/env bash\n# Git Baseline\n#\n# Captures baseline.\n",
        )
        .unwrap();

        let config = AppConfig::default();
        let routines = list_routines(dir.path(), &config).unwrap();
        assert_eq!(routines.len(), 2);
        let names: Vec<&str> = routines.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"develop"));
        assert!(names.contains(&"hooks/git-baseline"));
    }

    #[test]
    fn test_list_routines_no_dir() {
        let dir = TempDir::new().unwrap();
        let config = AppConfig::default();
        let routines = list_routines(dir.path(), &config).unwrap();
        assert!(routines.is_empty());
    }

    // --- Router prompt tests ---

    #[test]
    fn test_build_router_prompt() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        std::fs::write(
            dir.path().join(".decree/router.md"),
            "Select routine.\n\n## Routines\n{routines}\n\n## Message\n{message}\n",
        )
        .unwrap();

        let routines = vec![
            RoutineInfo {
                name: "develop".into(),
                description: "General purpose.".into(),
            },
            RoutineInfo {
                name: "rust-develop".into(),
                description: "Rust specific.".into(),
            },
        ];

        let prompt =
            build_router_prompt(dir.path(), &routines, "Add auth.").unwrap();

        assert!(prompt.contains("- **develop**: General purpose."));
        assert!(prompt.contains("- **rust-develop**: Rust specific."));
        assert!(prompt.contains("Add auth."));
    }

    #[test]
    fn test_build_router_prompt_no_description() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        std::fs::write(
            dir.path().join(".decree/router.md"),
            "{routines}\n{message}\n",
        )
        .unwrap();

        let routines = vec![RoutineInfo {
            name: "develop".into(),
            description: String::new(),
        }];

        let prompt =
            build_router_prompt(dir.path(), &routines, "Body.").unwrap();
        assert!(prompt.contains("- **develop**"));
        assert!(!prompt.contains("- **develop**:"));
    }

    // --- value helper tests ---

    #[test]
    fn test_value_as_string() {
        assert_eq!(
            value_as_string(&serde_yaml::Value::String("hello".into())),
            Some("hello".to_string())
        );
        assert_eq!(value_as_string(&serde_yaml::Value::Null), None);
    }

    #[test]
    fn test_value_as_u32() {
        let num = serde_yaml::to_value(42u32).unwrap();
        assert_eq!(value_as_u32(&num), Some(42));

        assert_eq!(
            value_as_u32(&serde_yaml::Value::String("7".into())),
            Some(7)
        );
        assert_eq!(value_as_u32(&serde_yaml::Value::Null), None);
    }
}
