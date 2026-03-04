use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde_yaml::Value;

use crate::error::{DecreeError, Result};
use crate::migration::split_frontmatter;

/// Message type: spec (has input_file) or task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// A fully normalized inbox message with all required fields present.
#[derive(Debug, Clone)]
pub struct Message {
    pub id: String,
    pub chain: String,
    pub seq: u32,
    pub message_type: MessageType,
    pub input_file: Option<String>,
    pub routine: String,
    pub custom_fields: BTreeMap<String, Value>,
    pub body: String,
    pub path: PathBuf,
}

/// A raw parsed message where all fields are optional.
#[derive(Debug, Clone)]
pub struct RawMessage {
    pub id: Option<String>,
    pub chain: Option<String>,
    pub seq: Option<u32>,
    pub message_type: Option<MessageType>,
    pub input_file: Option<String>,
    pub routine: Option<String>,
    pub custom_fields: BTreeMap<String, Value>,
    pub body: String,
    pub path: PathBuf,
}

/// Known frontmatter field names that are extracted into typed fields.
const KNOWN_FIELDS: &[&str] = &["id", "chain", "seq", "type", "input_file", "routine"];

impl RawMessage {
    /// Parse an inbox message file from disk.
    pub fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        Self::parse(path, &content)
    }

    /// Parse message content with a given path.
    pub fn parse(path: &Path, content: &str) -> Result<Self> {
        let (frontmatter, body) = split_frontmatter(content);

        let (known, custom) = match frontmatter {
            Some(yaml) if !yaml.trim().is_empty() => parse_frontmatter(yaml)?,
            _ => (BTreeMap::new(), BTreeMap::new()),
        };

        Ok(Self {
            id: get_string(&known, "id"),
            chain: get_string(&known, "chain"),
            seq: get_u32(&known, "seq"),
            message_type: get_string(&known, "type").and_then(|s| MessageType::parse(&s)),
            input_file: get_string(&known, "input_file"),
            routine: get_string(&known, "routine"),
            custom_fields: custom,
            body: body.to_string(),
            path: path.to_path_buf(),
        })
    }

    /// Check if this message already has all required fields filled.
    pub fn is_complete(&self) -> bool {
        self.id.is_some()
            && self.chain.is_some()
            && self.seq.is_some()
            && self.message_type.is_some()
            && self.routine.is_some()
    }
}

/// Parse YAML frontmatter into known fields and custom fields.
fn parse_frontmatter(yaml: &str) -> Result<(BTreeMap<String, Value>, BTreeMap<String, Value>)> {
    let mapping: BTreeMap<String, Value> = serde_yaml::from_str(yaml)
        .map_err(|e| DecreeError::Config(format!("invalid frontmatter YAML: {e}")))?;

    let mut known = BTreeMap::new();
    let mut custom = BTreeMap::new();

    for (key, value) in mapping {
        if KNOWN_FIELDS.contains(&key.as_str()) {
            known.insert(key, value);
        } else {
            custom.insert(key, value);
        }
    }

    Ok((known, custom))
}

fn get_string(map: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    map.get(key).and_then(|v| match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

fn get_u32(map: &BTreeMap<String, Value>, key: &str) -> Option<u32> {
    map.get(key).and_then(|v| match v {
        Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
        Value::String(s) => s.parse().ok(),
        _ => None,
    })
}

/// Extract chain and seq from a filename like `<chain>-<seq>.md`.
fn parse_filename(path: &Path) -> (Option<String>, Option<u32>) {
    let stem = match path.file_stem().and_then(|s| s.to_str()) {
        Some(s) => s,
        None => return (None, None),
    };

    // Find the last hyphen — everything before is chain, after is seq
    if let Some(last_hyphen) = stem.rfind('-') {
        let chain_part = &stem[..last_hyphen];
        let seq_part = &stem[last_hyphen + 1..];
        if let Ok(seq) = seq_part.parse::<u32>() {
            if !chain_part.is_empty() {
                return (Some(chain_part.to_string()), Some(seq));
            }
        }
    }

    (None, None)
}

/// Generate a new chain ID based on current timestamp.
pub fn generate_chain_id() -> String {
    Utc::now().format("%Y%m%d%H%M%S%3f").to_string()
}

/// Configuration needed for normalization.
pub struct NormalizeContext {
    pub default_routine: String,
    pub migration_routine: Option<String>,
}

/// Normalize a raw message, filling all missing fields.
///
/// The `routine_selector` callback is invoked only if no routine can be
/// determined from frontmatter, migration, or config. It receives the
/// message body and should return a routine name (AI-assisted selection).
pub fn normalize<F>(
    raw: RawMessage,
    ctx: &NormalizeContext,
    routine_selector: F,
) -> Result<Message>
where
    F: FnOnce(&str) -> Result<Option<String>>,
{
    let (filename_chain, filename_seq) = parse_filename(&raw.path);

    // chain: frontmatter → filename → generate new
    let chain = raw
        .chain
        .or(filename_chain)
        .unwrap_or_else(generate_chain_id);

    // seq: frontmatter → filename → default 0
    let seq = raw.seq.or(filename_seq).unwrap_or(0);

    // id: always recomputed
    let id = format!("{chain}-{seq}");

    // type: frontmatter → spec if input_file set → task
    let message_type = raw.message_type.unwrap_or_else(|| {
        if raw.input_file.is_some() {
            MessageType::Spec
        } else {
            MessageType::Task
        }
    });

    // routine: frontmatter → migration frontmatter → AI selector → config default → "develop"
    let routine = if let Some(r) = raw.routine {
        r
    } else if let Some(r) = ctx.migration_routine.clone() {
        r
    } else {
        // Try AI-assisted selection
        let ai_result = routine_selector(&raw.body)?;
        ai_result.unwrap_or_else(|| {
            if ctx.default_routine.is_empty() {
                "develop".to_string()
            } else {
                ctx.default_routine.clone()
            }
        })
    };

    Ok(Message {
        id,
        chain,
        seq,
        message_type,
        input_file: raw.input_file,
        routine,
        custom_fields: raw.custom_fields,
        body: raw.body,
        path: raw.path,
    })
}

impl Message {
    /// Serialize this message back to a file with full YAML frontmatter.
    pub fn to_string(&self) -> String {
        let mut yaml = BTreeMap::new();

        yaml.insert("id".to_string(), Value::String(self.id.clone()));
        yaml.insert("chain".to_string(), Value::String(self.chain.clone()));
        yaml.insert("seq".to_string(), Value::Number(serde_yaml::Number::from(self.seq)));
        yaml.insert(
            "type".to_string(),
            Value::String(self.message_type.as_str().to_string()),
        );
        if let Some(ref input_file) = self.input_file {
            yaml.insert("input_file".to_string(), Value::String(input_file.clone()));
        }
        yaml.insert("routine".to_string(), Value::String(self.routine.clone()));

        // Merge custom fields
        for (key, value) in &self.custom_fields {
            yaml.insert(key.clone(), value.clone());
        }

        let yaml_str = serde_yaml::to_string(&yaml)
            .unwrap_or_default();

        let mut output = String::new();
        output.push_str("---\n");
        output.push_str(&yaml_str);
        output.push_str("---\n");
        if !self.body.is_empty() {
            output.push_str(&self.body);
            if !self.body.ends_with('\n') {
                output.push('\n');
            }
        }
        output
    }

    /// Write this message to its path (rewrite with full frontmatter).
    pub fn write(&self) -> Result<()> {
        let content = self.to_string();
        fs::write(&self.path, content)?;
        Ok(())
    }

    /// Rename the file to match the canonical `<chain>-<seq>.md` name.
    /// Returns the new path.
    pub fn rename_to_canonical(&mut self, inbox_dir: &Path) -> Result<()> {
        let canonical_name = format!("{}-{}.md", self.chain, self.seq);
        let new_path = inbox_dir.join(&canonical_name);

        if new_path != self.path {
            if self.path.exists() {
                fs::rename(&self.path, &new_path)?;
            }
            self.path = new_path;
        }

        Ok(())
    }
}

/// List pending inbox messages (top-level .md files in inbox dir).
pub fn list_inbox(inbox_dir: &Path) -> Result<Vec<PathBuf>> {
    if !inbox_dir.exists() {
        return Ok(Vec::new());
    }

    let mut files: Vec<PathBuf> = Vec::new();

    for entry in fs::read_dir(inbox_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            files.push(path);
        }
    }

    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_filename_valid() {
        let path = Path::new(".decree/inbox/2025022514320000-0.md");
        let (chain, seq) = parse_filename(path);
        assert_eq!(chain.as_deref(), Some("2025022514320000"));
        assert_eq!(seq, Some(0));
    }

    #[test]
    fn test_parse_filename_higher_seq() {
        let path = Path::new(".decree/inbox/2025022514320000-5.md");
        let (chain, seq) = parse_filename(path);
        assert_eq!(chain.as_deref(), Some("2025022514320000"));
        assert_eq!(seq, Some(5));
    }

    #[test]
    fn test_parse_filename_no_seq() {
        let path = Path::new(".decree/inbox/random-name.md");
        let (chain, seq) = parse_filename(path);
        // "random" as chain, "name" is not a u32, so None
        assert!(chain.is_none());
        assert!(seq.is_none());
    }

    #[test]
    fn test_parse_filename_bare() {
        let path = Path::new(".decree/inbox/something.md");
        let (chain, seq) = parse_filename(path);
        assert!(chain.is_none());
        assert!(seq.is_none());
    }

    #[test]
    fn test_generate_chain_id_format() {
        let id = generate_chain_id();
        // Should be digits only, at least 17 chars (YYYYMMDDHHmmSSmmm)
        assert!(id.len() >= 17);
        assert!(id.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn test_raw_message_parse_full() {
        let content = "\
---
id: 2025022514320000-0
chain: 2025022514320000
seq: 0
type: spec
input_file: migrations/01-add-auth.md
routine: develop
---
Do the thing.";

        let path = Path::new(".decree/inbox/2025022514320000-0.md");
        let raw = RawMessage::parse(path, content).unwrap();

        assert_eq!(raw.id.as_deref(), Some("2025022514320000-0"));
        assert_eq!(raw.chain.as_deref(), Some("2025022514320000"));
        assert_eq!(raw.seq, Some(0));
        assert_eq!(raw.message_type, Some(MessageType::Spec));
        assert_eq!(raw.input_file.as_deref(), Some("migrations/01-add-auth.md"));
        assert_eq!(raw.routine.as_deref(), Some("develop"));
        assert_eq!(raw.body, "Do the thing.");
        assert!(raw.is_complete());
    }

    #[test]
    fn test_raw_message_parse_minimal() {
        let content = "\
---
routine: develop
---
Fix type errors in src/auth.rs.";

        let path = Path::new(".decree/inbox/test.md");
        let raw = RawMessage::parse(path, content).unwrap();

        assert!(raw.id.is_none());
        assert!(raw.chain.is_none());
        assert!(raw.seq.is_none());
        assert!(raw.message_type.is_none());
        assert_eq!(raw.routine.as_deref(), Some("develop"));
        assert_eq!(raw.body, "Fix type errors in src/auth.rs.");
        assert!(!raw.is_complete());
    }

    #[test]
    fn test_raw_message_parse_bare() {
        let content = "Fix type errors in src/auth.rs.";
        let path = Path::new(".decree/inbox/test.md");
        let raw = RawMessage::parse(path, content).unwrap();

        assert!(raw.id.is_none());
        assert!(raw.routine.is_none());
        assert_eq!(raw.body, "Fix type errors in src/auth.rs.");
    }

    #[test]
    fn test_raw_message_parse_empty_body() {
        let content = "\
---
input_file: migrations/01-add-auth.md
routine: develop
---
";
        let path = Path::new(".decree/inbox/test.md");
        let raw = RawMessage::parse(path, content).unwrap();

        assert_eq!(raw.input_file.as_deref(), Some("migrations/01-add-auth.md"));
        assert_eq!(raw.body, "");
    }

    #[test]
    fn test_raw_message_custom_fields() {
        let content = "\
---
routine: develop
my_custom: hello
priority: 5
---
Body.";

        let path = Path::new(".decree/inbox/test.md");
        let raw = RawMessage::parse(path, content).unwrap();

        assert_eq!(raw.routine.as_deref(), Some("develop"));
        assert_eq!(
            raw.custom_fields.get("my_custom"),
            Some(&Value::String("hello".to_string()))
        );
        assert!(raw.custom_fields.contains_key("priority"));
        assert_eq!(raw.custom_fields.len(), 2);
    }

    #[test]
    fn test_normalize_bare_message() {
        let content = "Fix type errors in src/auth.rs.";
        let path = Path::new(".decree/inbox/test.md");
        let raw = RawMessage::parse(path, content).unwrap();

        let ctx = NormalizeContext {
            default_routine: "develop".to_string(),
            migration_routine: None,
        };

        let msg = normalize(raw, &ctx, |_| Ok(None)).unwrap();

        assert!(!msg.chain.is_empty());
        assert_eq!(msg.seq, 0);
        assert_eq!(msg.id, format!("{}-0", msg.chain));
        assert_eq!(msg.message_type, MessageType::Task);
        assert_eq!(msg.routine, "develop");
        assert_eq!(msg.body, "Fix type errors in src/auth.rs.");
    }

    #[test]
    fn test_normalize_uses_filename_chain_seq() {
        let content = "Fix type errors.";
        let path = Path::new(".decree/inbox/2025022514320000-3.md");
        let raw = RawMessage::parse(path, content).unwrap();

        let ctx = NormalizeContext {
            default_routine: "develop".to_string(),
            migration_routine: None,
        };

        let msg = normalize(raw, &ctx, |_| Ok(None)).unwrap();

        assert_eq!(msg.chain, "2025022514320000");
        assert_eq!(msg.seq, 3);
        assert_eq!(msg.id, "2025022514320000-3");
    }

    #[test]
    fn test_normalize_frontmatter_overrides_filename() {
        let content = "\
---
chain: custom_chain
seq: 7
---
Body.";
        let path = Path::new(".decree/inbox/2025022514320000-3.md");
        let raw = RawMessage::parse(path, content).unwrap();

        let ctx = NormalizeContext {
            default_routine: "develop".to_string(),
            migration_routine: None,
        };

        let msg = normalize(raw, &ctx, |_| Ok(None)).unwrap();

        assert_eq!(msg.chain, "custom_chain");
        assert_eq!(msg.seq, 7);
        assert_eq!(msg.id, "custom_chain-7");
    }

    #[test]
    fn test_normalize_type_spec_when_input_file() {
        let content = "\
---
input_file: migrations/01-add-auth.md
---
";
        let path = Path::new(".decree/inbox/test.md");
        let raw = RawMessage::parse(path, content).unwrap();

        let ctx = NormalizeContext {
            default_routine: "develop".to_string(),
            migration_routine: None,
        };

        let msg = normalize(raw, &ctx, |_| Ok(None)).unwrap();

        assert_eq!(msg.message_type, MessageType::Spec);
    }

    #[test]
    fn test_normalize_routine_fallback_migration() {
        let content = "Do something.";
        let path = Path::new(".decree/inbox/test.md");
        let raw = RawMessage::parse(path, content).unwrap();

        let ctx = NormalizeContext {
            default_routine: "develop".to_string(),
            migration_routine: Some("custom-routine".to_string()),
        };

        // AI returns None, so fallback to migration routine
        let msg = normalize(raw, &ctx, |_| Ok(None)).unwrap();
        assert_eq!(msg.routine, "custom-routine");
    }

    #[test]
    fn test_normalize_routine_fallback_config_default() {
        let content = "Do something.";
        let path = Path::new(".decree/inbox/test.md");
        let raw = RawMessage::parse(path, content).unwrap();

        let ctx = NormalizeContext {
            default_routine: "my-default".to_string(),
            migration_routine: None,
        };

        let msg = normalize(raw, &ctx, |_| Ok(None)).unwrap();
        assert_eq!(msg.routine, "my-default");
    }

    #[test]
    fn test_normalize_routine_ai_selection() {
        let content = "Do something.";
        let path = Path::new(".decree/inbox/test.md");
        let raw = RawMessage::parse(path, content).unwrap();

        let ctx = NormalizeContext {
            default_routine: "develop".to_string(),
            migration_routine: None,
        };

        let msg = normalize(raw, &ctx, |_| Ok(Some("ai-picked".to_string()))).unwrap();
        assert_eq!(msg.routine, "ai-picked");
    }

    #[test]
    fn test_normalize_routine_from_frontmatter_skips_ai() {
        let content = "\
---
routine: explicit
---
Body.";
        let path = Path::new(".decree/inbox/test.md");
        let raw = RawMessage::parse(path, content).unwrap();

        let ctx = NormalizeContext {
            default_routine: "develop".to_string(),
            migration_routine: None,
        };

        // The AI selector should NOT be called since routine is in frontmatter
        let msg = normalize(raw, &ctx, |_| {
            panic!("AI selector should not be called");
        })
        .unwrap();
        assert_eq!(msg.routine, "explicit");
    }

    #[test]
    fn test_normalize_complete_message_not_changed() {
        let content = "\
---
id: 2025022514320000-0
chain: 2025022514320000
seq: 0
type: spec
input_file: migrations/01-add-auth.md
routine: develop
---
Do the thing.";

        let path = Path::new(".decree/inbox/2025022514320000-0.md");
        let raw = RawMessage::parse(path, content).unwrap();
        assert!(raw.is_complete());

        let ctx = NormalizeContext {
            default_routine: "develop".to_string(),
            migration_routine: None,
        };

        let msg = normalize(raw, &ctx, |_| Ok(None)).unwrap();
        assert_eq!(msg.id, "2025022514320000-0");
        assert_eq!(msg.chain, "2025022514320000");
        assert_eq!(msg.seq, 0);
        assert_eq!(msg.message_type, MessageType::Spec);
        assert_eq!(msg.routine, "develop");
    }

    #[test]
    fn test_message_serialize() {
        let msg = Message {
            id: "20250225-0".to_string(),
            chain: "20250225".to_string(),
            seq: 0,
            message_type: MessageType::Task,
            input_file: None,
            routine: "develop".to_string(),
            custom_fields: BTreeMap::new(),
            body: "Fix the bug.".to_string(),
            path: PathBuf::from(".decree/inbox/20250225-0.md"),
        };

        let output = msg.to_string();
        assert!(output.starts_with("---\n"));
        assert!(output.contains("id: 20250225-0"));
        assert!(output.contains("chain: '20250225'"));
        assert!(output.contains("seq: 0"));
        assert!(output.contains("type: task"));
        assert!(output.contains("routine: develop"));
        assert!(output.contains("Fix the bug."));
    }

    #[test]
    fn test_message_serialize_with_custom_fields() {
        let mut custom = BTreeMap::new();
        custom.insert("priority".to_string(), Value::Number(serde_yaml::Number::from(5)));
        custom.insert("tag".to_string(), Value::String("urgent".to_string()));

        let msg = Message {
            id: "20250225-0".to_string(),
            chain: "20250225".to_string(),
            seq: 0,
            message_type: MessageType::Task,
            input_file: None,
            routine: "develop".to_string(),
            custom_fields: custom,
            body: "Fix it.".to_string(),
            path: PathBuf::from("test.md"),
        };

        let output = msg.to_string();
        assert!(output.contains("priority: 5"));
        assert!(output.contains("tag: urgent"));
    }

    #[test]
    fn test_message_serialize_with_input_file() {
        let msg = Message {
            id: "20250225-0".to_string(),
            chain: "20250225".to_string(),
            seq: 0,
            message_type: MessageType::Spec,
            input_file: Some("migrations/01-add-auth.md".to_string()),
            routine: "develop".to_string(),
            custom_fields: BTreeMap::new(),
            body: String::new(),
            path: PathBuf::from("test.md"),
        };

        let output = msg.to_string();
        assert!(output.contains("input_file: migrations/01-add-auth.md"));
        assert!(output.contains("type: spec"));
    }

    #[test]
    fn test_message_roundtrip() {
        let mut custom = BTreeMap::new();
        custom.insert("env_var".to_string(), Value::String("value".to_string()));

        let msg = Message {
            id: "2025022514320000-2".to_string(),
            chain: "2025022514320000".to_string(),
            seq: 2,
            message_type: MessageType::Task,
            input_file: None,
            routine: "develop".to_string(),
            custom_fields: custom,
            body: "Original body text.".to_string(),
            path: PathBuf::from(".decree/inbox/2025022514320000-2.md"),
        };

        let serialized = msg.to_string();
        let raw = RawMessage::parse(&msg.path, &serialized).unwrap();

        assert_eq!(raw.id.as_deref(), Some("2025022514320000-2"));
        assert_eq!(raw.chain.as_deref(), Some("2025022514320000"));
        assert_eq!(raw.seq, Some(2));
        assert_eq!(raw.message_type, Some(MessageType::Task));
        assert_eq!(raw.routine.as_deref(), Some("develop"));
        assert!(raw.custom_fields.contains_key("env_var"));
        assert_eq!(raw.body.trim(), "Original body text.");
    }

    #[test]
    fn test_list_inbox() {
        let tmp = std::env::temp_dir().join("decree_test_list_inbox");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        fs::write(tmp.join("b-1.md"), "B").unwrap();
        fs::write(tmp.join("a-0.md"), "A").unwrap();
        fs::write(tmp.join("not-md.txt"), "skip").unwrap();
        fs::create_dir_all(tmp.join("done")).unwrap(); // subdir should be skipped

        let files = list_inbox(&tmp).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files[0].file_name().unwrap().to_str().unwrap() == "a-0.md");
        assert!(files[1].file_name().unwrap().to_str().unwrap() == "b-1.md");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_empty_body_valid() {
        let content = "\
---
routine: develop
---
";
        let path = Path::new(".decree/inbox/test.md");
        let raw = RawMessage::parse(path, content).unwrap();
        assert_eq!(raw.body, "");

        let ctx = NormalizeContext {
            default_routine: "develop".to_string(),
            migration_routine: None,
        };

        let msg = normalize(raw, &ctx, |_| Ok(None)).unwrap();
        assert_eq!(msg.body, "");
        assert_eq!(msg.routine, "develop");
    }

    #[test]
    fn test_message_write_and_reload() {
        let tmp = std::env::temp_dir().join("decree_test_msg_write");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let path = tmp.join("test-0.md");
        let msg = Message {
            id: "test-0".to_string(),
            chain: "test".to_string(),
            seq: 0,
            message_type: MessageType::Task,
            input_file: None,
            routine: "develop".to_string(),
            custom_fields: BTreeMap::new(),
            body: "Hello world.".to_string(),
            path: path.clone(),
        };

        msg.write().unwrap();

        let reloaded = RawMessage::load(&path).unwrap();
        assert_eq!(reloaded.id.as_deref(), Some("test-0"));
        assert_eq!(reloaded.chain.as_deref(), Some("test"));
        assert_eq!(reloaded.seq, Some(0));
        assert_eq!(reloaded.routine.as_deref(), Some("develop"));
        assert_eq!(reloaded.body.trim(), "Hello world.");

        let _ = fs::remove_dir_all(&tmp);
    }
}
