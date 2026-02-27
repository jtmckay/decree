use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::config::Config;
use crate::error::DecreeError;
use crate::routine::{self, RoutineInfo};

/// A parsed message ID: `<chain>-<seq>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageId {
    pub chain: String,
    pub seq: u32,
}

impl std::fmt::Display for MessageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.chain, self.seq)
    }
}

impl MessageId {
    pub fn new(chain: &str, seq: u32) -> Self {
        Self {
            chain: chain.to_string(),
            seq,
        }
    }

    /// Parse a full message ID like `2025022514320000-2`.
    pub fn parse(s: &str) -> Option<Self> {
        let (chain, seq_str) = s.rsplit_once('-')?;
        let seq: u32 = seq_str.parse().ok()?;
        if chain.len() < 14 {
            return None;
        }
        Some(Self {
            chain: chain.to_string(),
            seq,
        })
    }

    /// Generate a new chain ID from the current timestamp.
    /// Format: YYYYMMDDHHmmss + 2-digit counter.
    pub fn new_chain(counter: u8) -> String {
        let now = chrono::Local::now();
        format!("{}{:02}", now.format("%Y%m%d%H%M%S"), counter)
    }

    /// Directory name for this message under `.decree/runs/`.
    pub fn dir_name(&self) -> String {
        self.to_string()
    }
}

/// Resolve an ID prefix to matching message directories in `.decree/runs/`.
///
/// Accepts:
/// - Full message ID: `2025022514320000-2`
/// - Chain ID: `2025022514320000` (matches all messages in chain)
/// - Unique prefix of either
pub fn resolve_id(runs_dir: &Path, prefix: &str) -> Result<Vec<String>, DecreeError> {
    if !runs_dir.is_dir() {
        return Err(DecreeError::MessageNotFound(prefix.to_string()));
    }

    let mut matches: Vec<String> = Vec::new();

    let entries = std::fs::read_dir(runs_dir)?;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !entry.file_type()?.is_dir() {
            continue;
        }
        // Match full ID or chain prefix
        if name.starts_with(prefix) {
            matches.push(name);
        }
    }

    matches.sort();

    if matches.is_empty() {
        return Err(DecreeError::MessageNotFound(prefix.to_string()));
    }

    Ok(matches)
}

/// Get the most recent message directory (by name, which sorts chronologically).
pub fn most_recent(runs_dir: &Path) -> Result<String, DecreeError> {
    if !runs_dir.is_dir() {
        return Err(DecreeError::MessageNotFound("(no runs)".to_string()));
    }

    let mut dirs: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(runs_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            dirs.push(entry.file_name().to_string_lossy().to_string());
        }
    }

    dirs.sort();
    dirs.last()
        .cloned()
        .ok_or_else(|| DecreeError::MessageNotFound("(no runs)".to_string()))
}

// ---------------------------------------------------------------------------
// Inbox message format & normalization
// ---------------------------------------------------------------------------

/// Message type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageType {
    Spec,
    Task,
}

impl MessageType {
    pub fn as_str(&self) -> &'static str {
        match self {
            MessageType::Spec => "spec",
            MessageType::Task => "task",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "spec" => Some(MessageType::Spec),
            "task" => Some(MessageType::Task),
            _ => None,
        }
    }
}

/// A fully-parsed inbox message with all fields resolved.
#[derive(Debug, Clone)]
pub struct InboxMessage {
    pub id: String,
    pub chain: String,
    pub seq: u32,
    pub msg_type: MessageType,
    pub input_file: Option<String>,
    pub routine: String,
    pub body: String,
    /// Custom frontmatter fields beyond the standard set.
    pub custom_fields: BTreeMap<String, serde_yaml::Value>,
}

/// Raw parsed frontmatter — all fields optional.
#[derive(Debug, Clone, Default)]
pub struct RawFrontmatter {
    pub id: Option<String>,
    pub chain: Option<String>,
    pub seq: Option<u32>,
    pub msg_type: Option<String>,
    pub input_file: Option<String>,
    pub routine: Option<String>,
    pub custom_fields: BTreeMap<String, serde_yaml::Value>,
}

/// Parse a markdown file into raw frontmatter + body.
pub fn parse_message_file(content: &str) -> (RawFrontmatter, String) {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (RawFrontmatter::default(), content.to_string());
    }

    let after_open = &trimmed[3..];
    // Find closing "---" on its own line
    let close = match after_open.find("\n---") {
        Some(pos) => pos,
        None => return (RawFrontmatter::default(), content.to_string()),
    };

    let yaml_block = &after_open[..close];
    // Body starts after the closing "---\n"
    let body_start = 3 + close + 4; // "---" + "\n---"
    let body = if body_start < trimmed.len() {
        let rest = &trimmed[body_start..];
        // Strip one leading newline if present
        rest.strip_prefix('\n').unwrap_or(rest).to_string()
    } else {
        String::new()
    };

    let map: serde_yaml::Value = match serde_yaml::from_str(yaml_block) {
        Ok(v) => v,
        Err(_) => return (RawFrontmatter::default(), content.to_string()),
    };

    let mapping = match map.as_mapping() {
        Some(m) => m,
        None => return (RawFrontmatter::default(), content.to_string()),
    };

    let mut fm = RawFrontmatter::default();
    let known_keys = ["id", "chain", "seq", "type", "input_file", "routine"];

    for (key, value) in mapping {
        let key_str = match key.as_str() {
            Some(k) => k,
            None => continue,
        };

        match key_str {
            "id" => fm.id = value.as_str().map(String::from),
            "chain" => {
                // Chain can be a number or string in YAML
                fm.chain = value
                    .as_str()
                    .map(String::from)
                    .or_else(|| value.as_u64().map(|n| n.to_string()));
            }
            "seq" => fm.seq = value.as_u64().map(|n| n as u32),
            "type" => fm.msg_type = value.as_str().map(String::from),
            "input_file" => fm.input_file = value.as_str().map(String::from),
            "routine" => fm.routine = value.as_str().map(String::from),
            _ => {
                if !known_keys.contains(&key_str) {
                    fm.custom_fields
                        .insert(key_str.to_string(), value.clone());
                }
            }
        }
    }

    (fm, body)
}

/// Try to extract chain and seq from a filename like `<chain>-<seq>.md`.
pub fn chain_seq_from_filename(filename: &str) -> Option<(String, u32)> {
    let stem = filename.strip_suffix(".md")?;
    MessageId::parse(stem).map(|id| (id.chain, id.seq))
}

/// Check if all required fields are present (normalization is a no-op).
fn is_fully_normalized(fm: &RawFrontmatter) -> bool {
    fm.id.is_some()
        && fm.chain.is_some()
        && fm.seq.is_some()
        && fm.msg_type.is_some()
        && fm.routine.is_some()
}

/// Serialize an InboxMessage back to a markdown file with YAML frontmatter.
pub fn serialize_message(msg: &InboxMessage) -> String {
    let mut out = String::from("---\n");
    out.push_str(&format!("id: {}\n", msg.id));
    out.push_str(&format!("chain: {}\n", msg.chain));
    out.push_str(&format!("seq: {}\n", msg.seq));
    out.push_str(&format!("type: {}\n", msg.msg_type.as_str()));
    if let Some(ref input_file) = msg.input_file {
        out.push_str(&format!("input_file: {}\n", input_file));
    }
    out.push_str(&format!("routine: {}\n", msg.routine));
    for (key, value) in &msg.custom_fields {
        let val_str = match value {
            serde_yaml::Value::String(s) => s.clone(),
            other => serde_yaml::to_string(other).unwrap_or_default().trim().to_string(),
        };
        out.push_str(&format!("{}: {}\n", key, val_str));
    }
    out.push_str("---\n");
    if !msg.body.is_empty() {
        out.push_str(&msg.body);
    }
    out
}

/// Router function type for routine selection.
/// Takes a prompt string and returns the router's response.
pub type RouterFn = dyn FnOnce(&str) -> Result<String, DecreeError>;

/// Normalize an inbox message file in place.
///
/// Reads the file, fills missing fields, optionally invokes the router for
/// routine selection, and writes the normalized message back.
///
/// `router_fn`: called when the routine field is missing and must be selected
/// by the router AI. Pass `None` to skip AI routing (fallback chain is used).
///
/// `spec_routine`: the routine from the spec's frontmatter, if this message
/// originated from a spec. Used in the fallback chain.
pub fn normalize_message(
    file_path: &Path,
    config: &Config,
    routines: &[RoutineInfo],
    router_fn: Option<Box<RouterFn>>,
    spec_routine: Option<&str>,
) -> Result<InboxMessage, DecreeError> {
    let content = fs::read_to_string(file_path)?;
    let (fm, body) = parse_message_file(&content);

    // If fully normalized, parse and return without rewriting
    if is_fully_normalized(&fm) {
        let msg_type = MessageType::parse(fm.msg_type.as_deref().unwrap_or("task"))
            .unwrap_or(MessageType::Task);
        return Ok(InboxMessage {
            id: fm.id.unwrap_or_default(),
            chain: fm.chain.unwrap_or_default(),
            seq: fm.seq.unwrap_or(0),
            msg_type,
            input_file: fm.input_file,
            routine: fm.routine.unwrap_or_default(),
            body,
            custom_fields: fm.custom_fields,
        });
    }

    // Derive chain and seq from filename if not in frontmatter
    let filename = file_path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    let from_filename = chain_seq_from_filename(&filename);

    let chain = fm
        .chain
        .or_else(|| from_filename.as_ref().map(|(c, _)| c.clone()))
        .unwrap_or_else(|| MessageId::new_chain(0));

    let seq = fm
        .seq
        .or_else(|| from_filename.map(|(_, s)| s))
        .unwrap_or(0);

    let id = format!("{}-{}", chain, seq);

    // Type inference
    let msg_type = fm
        .msg_type
        .as_deref()
        .and_then(MessageType::parse)
        .unwrap_or_else(|| {
            if fm.input_file.is_some() {
                MessageType::Spec
            } else {
                MessageType::Task
            }
        });

    // Routine selection
    let routine = if let Some(r) = fm.routine {
        r
    } else {
        select_routine(&body, routines, router_fn, spec_routine, config)
    };

    let msg = InboxMessage {
        id,
        chain,
        seq,
        msg_type,
        input_file: fm.input_file,
        routine,
        body: body.clone(),
        custom_fields: fm.custom_fields,
    };

    // Write back normalized message
    let normalized = serialize_message(&msg);
    fs::write(file_path, normalized)?;

    Ok(msg)
}

/// Select a routine using the fallback chain:
/// 1. Router AI (if router_fn provided and routines exist)
/// 2. Spec frontmatter routine
/// 3. Config default_routine
/// 4. "develop"
fn select_routine(
    body: &str,
    routines: &[RoutineInfo],
    router_fn: Option<Box<RouterFn>>,
    spec_routine: Option<&str>,
    config: &Config,
) -> String {
    // Try router AI
    if let Some(router) = router_fn {
        if !routines.is_empty() {
            let prompt = routine::build_router_prompt(routines, body);
            if let Ok(response) = router(&prompt) {
                let name = response.trim().to_string();
                if routine::is_valid_routine(routines, &name) {
                    return name;
                }
            }
        }
    }

    // Fallback chain
    if let Some(r) = spec_routine {
        if !r.is_empty() {
            return r.to_string();
        }
    }

    if !config.default_routine.is_empty() {
        return config.default_routine.clone();
    }

    "develop".to_string()
}
