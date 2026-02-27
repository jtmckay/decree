use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::error::DecreeError;
use crate::llm::ChatMessage;

/// A persisted AI conversation session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub created: String,
    pub updated: String,
    pub history: Vec<ChatMessage>,
}

impl Session {
    /// Create a new session with a timestamp-based ID (YYYYMMDDHHmmss).
    pub fn new() -> Self {
        let now = Utc::now();
        let id = now.format("%Y%m%d%H%M%S").to_string();
        let ts = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        Session {
            id,
            created: ts.clone(),
            updated: ts,
            history: Vec::new(),
        }
    }

    /// Save the session to `.decree/sessions/{id}.yml` using atomic write.
    pub fn save(&mut self, project_root: &Path) -> Result<(), DecreeError> {
        self.updated = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        let sessions_dir = project_root.join(".decree/sessions");
        fs::create_dir_all(&sessions_dir)?;

        let path = sessions_dir.join(format!("{}.yml", self.id));
        let tmp_path = sessions_dir.join(format!("{}.yml.tmp", self.id));

        let content = serde_yaml::to_string(self)?;
        fs::write(&tmp_path, &content)?;
        fs::rename(&tmp_path, &path)?;

        Ok(())
    }

    /// Load a session by its ID from `.decree/sessions/{id}.yml`.
    pub fn load(project_root: &Path, id: &str) -> Result<Self, DecreeError> {
        let path = project_root
            .join(".decree/sessions")
            .join(format!("{id}.yml"));
        if !path.exists() {
            let available = list_sessions(project_root)?;
            if available.is_empty() {
                return Err(DecreeError::Session(
                    "no sessions found in .decree/sessions/".to_string(),
                ));
            }
            let list = available.join(", ");
            return Err(DecreeError::Session(format!(
                "session '{id}' not found. available sessions: {list}"
            )));
        }
        let content = fs::read_to_string(&path)?;
        let session: Session =
            serde_yaml::from_str(&content).map_err(|e| DecreeError::Session(format!("{e}")))?;
        Ok(session)
    }

    /// Load the most recently modified session file.
    pub fn load_latest(project_root: &Path) -> Result<Self, DecreeError> {
        let sessions_dir = project_root.join(".decree/sessions");
        if !sessions_dir.exists() {
            return Err(DecreeError::Session(
                "no sessions found in .decree/sessions/".to_string(),
            ));
        }

        let mut newest: Option<(PathBuf, std::time::SystemTime)> = None;
        for entry in fs::read_dir(&sessions_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("yml") {
                let modified = entry.metadata()?.modified()?;
                if newest
                    .as_ref()
                    .map_or(true, |(_, prev)| modified > *prev)
                {
                    newest = Some((path, modified));
                }
            }
        }

        match newest {
            Some((path, _)) => {
                let content = fs::read_to_string(&path)?;
                let session: Session = serde_yaml::from_str(&content)
                    .map_err(|e| DecreeError::Session(format!("{e}")))?;
                Ok(session)
            }
            None => Err(DecreeError::Session(
                "no sessions found in .decree/sessions/".to_string(),
            )),
        }
    }
}

/// List all session IDs in `.decree/sessions/`.
pub fn list_sessions(project_root: &Path) -> Result<Vec<String>, DecreeError> {
    let sessions_dir = project_root.join(".decree/sessions");
    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }
    let mut ids = Vec::new();
    for entry in fs::read_dir(&sessions_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("yml") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                ids.push(stem.to_string());
            }
        }
    }
    ids.sort();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_new_session_has_14_digit_id() {
        let session = Session::new();
        assert_eq!(session.id.len(), 14);
        assert!(session.id.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn test_new_session_has_empty_history() {
        let session = Session::new();
        assert!(session.history.is_empty());
    }

    #[test]
    fn test_new_session_timestamps_are_rfc3339() {
        let session = Session::new();
        assert!(session.created.contains('T'));
        assert!(session.created.ends_with('Z'));
    }

    #[test]
    fn test_session_save_and_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".decree/sessions")).unwrap();

        let mut session = Session::new();
        session.history.push(ChatMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
        });
        session.history.push(ChatMessage {
            role: "assistant".to_string(),
            content: "Hi there!".to_string(),
        });
        session.save(root).unwrap();

        let loaded = Session::load(root, &session.id).unwrap();
        assert_eq!(loaded.id, session.id);
        assert_eq!(loaded.history.len(), 2);
        assert_eq!(loaded.history[0].role, "user");
        assert_eq!(loaded.history[0].content, "Hello");
        assert_eq!(loaded.history[1].role, "assistant");
        assert_eq!(loaded.history[1].content, "Hi there!");
    }

    #[test]
    fn test_session_save_is_atomic() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".decree/sessions")).unwrap();

        let mut session = Session::new();
        session.save(root).unwrap();

        // The .tmp file should not exist after save
        let tmp_path = root
            .join(".decree/sessions")
            .join(format!("{}.yml.tmp", session.id));
        assert!(!tmp_path.exists());

        // The actual file should exist
        let path = root
            .join(".decree/sessions")
            .join(format!("{}.yml", session.id));
        assert!(path.exists());
    }

    #[test]
    fn test_session_file_is_valid_yaml() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".decree/sessions")).unwrap();

        let mut session = Session::new();
        session.history.push(ChatMessage {
            role: "user".to_string(),
            content: "What is Rust?".to_string(),
        });
        session.history.push(ChatMessage {
            role: "assistant".to_string(),
            content: "Rust is a systems programming language.".to_string(),
        });
        session.save(root).unwrap();

        let path = root
            .join(".decree/sessions")
            .join(format!("{}.yml", session.id));
        let content = std::fs::read_to_string(&path).unwrap();
        let value: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();

        assert!(value["id"].is_string());
        assert!(value["created"].is_string());
        assert!(value["updated"].is_string());
        assert!(value["history"].is_sequence());
        let history = value["history"].as_sequence().unwrap();
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn test_load_latest_session() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".decree/sessions")).unwrap();

        let mut s1 = Session::new();
        s1.id = "20260226140000".to_string();
        s1.save(root).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(50));

        let mut s2 = Session::new();
        s2.id = "20260226150000".to_string();
        s2.save(root).unwrap();

        let latest = Session::load_latest(root).unwrap();
        assert_eq!(latest.id, "20260226150000");
    }

    #[test]
    fn test_load_nonexistent_session() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".decree/sessions")).unwrap();

        let result = Session::load(root, "99999999999999");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no sessions found"));
    }

    #[test]
    fn test_load_latest_no_sessions() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".decree/sessions")).unwrap();

        let result = Session::load_latest(root);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no sessions found"));
    }

    #[test]
    fn test_list_sessions() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".decree/sessions")).unwrap();

        let mut s1 = Session::new();
        s1.id = "20260226140000".to_string();
        s1.save(root).unwrap();

        let mut s2 = Session::new();
        s2.id = "20260226150000".to_string();
        s2.save(root).unwrap();

        let ids = list_sessions(root).unwrap();
        assert_eq!(ids, vec!["20260226140000", "20260226150000"]);
    }

    #[test]
    fn test_session_history_grows_monotonically() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".decree/sessions")).unwrap();

        let mut session = Session::new();

        session.history.push(ChatMessage {
            role: "user".to_string(),
            content: "Q1".to_string(),
        });
        session.history.push(ChatMessage {
            role: "assistant".to_string(),
            content: "A1".to_string(),
        });
        session.save(root).unwrap();

        session.history.push(ChatMessage {
            role: "user".to_string(),
            content: "Q2".to_string(),
        });
        session.history.push(ChatMessage {
            role: "assistant".to_string(),
            content: "A2".to_string(),
        });
        session.save(root).unwrap();

        let loaded = Session::load(root, &session.id).unwrap();
        assert_eq!(loaded.history.len(), 4);
        assert_eq!(loaded.history[0].content, "Q1");
        assert_eq!(loaded.history[1].content, "A1");
        assert_eq!(loaded.history[2].content, "Q2");
        assert_eq!(loaded.history[3].content, "A2");
    }
}
