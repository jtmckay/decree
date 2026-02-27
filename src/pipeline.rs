use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::checkpoint;
use crate::config::Config;
use crate::error::DecreeError;
use crate::message::{self, InboxMessage, MessageType, RouterFn};
use crate::routine;
use crate::spec;

// ---------------------------------------------------------------------------
// Router helper
// ---------------------------------------------------------------------------

/// Call the router AI with a prompt and return the response text.
///
/// Dispatches to embedded AI or an external CLI command based on
/// `config.commands.router`.
pub fn call_router(config: &Config, prompt: &str) -> Result<String, DecreeError> {
    let router_cmd = &config.commands.router;

    if router_cmd == "decree ai" {
        call_embedded_router(config, prompt)
    } else if router_cmd.contains("{prompt}") {
        let escaped = shell_escape(prompt);
        let full_cmd = router_cmd.replace("{prompt}", &escaped);
        let output = Command::new("bash")
            .arg("-c")
            .arg(&full_cmd)
            .output()
            .map_err(|e| DecreeError::Config(format!("router command failed: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DecreeError::Config(format!(
                "router command exited with {}: {}",
                output.status, stderr
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(DecreeError::Config(format!(
            "unrecognized router command: {router_cmd}"
        )))
    }
}

/// Call the embedded AI (llama.cpp) as the router.
fn call_embedded_router(config: &Config, prompt: &str) -> Result<String, DecreeError> {
    use crate::llm;

    let model_path = llm::ensure_model(config)?;
    let backend = llm::init_backend(true)?;
    let model = llm::load_model(&backend, &model_path, config.ai.n_gpu_layers)?;
    let mut ctx = llm::create_context(&model, &backend, llm::DEFAULT_CTX_SIZE)?;

    let messages = vec![
        llm::ChatMessage {
            role: "system".into(),
            content: "You are a helpful assistant. Respond concisely.".into(),
        },
        llm::ChatMessage {
            role: "user".into(),
            content: prompt.to_string(),
        },
    ];

    let prompt_text = llm::build_chatml(&messages, true);
    let tokens = llm::tokenize(&model, &prompt_text, false)?;
    ctx.clear_kv_cache();

    let (output, _) = llm::generate(&mut ctx, &model, &tokens, 100, |_| {})?;
    Ok(output.trim().to_string())
}

/// Escape a string for safe embedding in a single-quoted shell argument.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ---------------------------------------------------------------------------
// Message name derivation
// ---------------------------------------------------------------------------

/// Derive a message name from the prompt text using the router AI.
///
/// Falls back to a chain-ID-based name on failure.
pub fn derive_message_name(
    config: &Config,
    prompt: &str,
    inbox_dir: &Path,
    chain_id: &str,
) -> String {
    if prompt.is_empty() {
        return format!("run-{chain_id}");
    }

    let name_prompt = format!(
        "Summarize this task as a kebab-case name under 5 words. \
         Respond with ONLY the name.\n\n{}",
        prompt
    );

    match call_router(config, &name_prompt) {
        Ok(response) => {
            let name = to_kebab_case(response.trim());
            if name.is_empty() {
                return format!("run-{chain_id}");
            }
            let name = truncate_str(&name, 50);
            dedup_name(&name, inbox_dir)
        }
        Err(_) => format!("run-{chain_id}"),
    }
}

/// Convert a string to kebab-case: lowercase, non-alphanumeric → hyphens,
/// collapse multiple hyphens, strip leading/trailing hyphens.
pub fn to_kebab_case(s: &str) -> String {
    let raw: String = s
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();

    raw.split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Truncate a string to at most `max` bytes (on a char boundary).
fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    // Don't end on a hyphen
    let truncated = &s[..end];
    truncated.trim_end_matches('-').to_string()
}

/// Ensure `name` does not collide with existing filenames in the inbox tree.
/// Checks inbox/, inbox/done/, and inbox/dead/. Appends -2, -3, … on collision.
fn dedup_name(name: &str, inbox_dir: &Path) -> String {
    let existing = collect_existing_names(inbox_dir);
    let filename = format!("{name}.md");
    if !existing.contains(&filename) {
        return name.to_string();
    }
    for i in 2u32.. {
        let candidate = format!("{name}-{i}");
        let candidate_file = format!("{candidate}.md");
        if !existing.contains(&candidate_file) {
            return candidate;
        }
    }
    unreachable!()
}

/// Collect all `.md` filenames from inbox/, inbox/done/, and inbox/dead/.
fn collect_existing_names(inbox_dir: &Path) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let dirs = [
        inbox_dir.to_path_buf(),
        inbox_dir.join("done"),
        inbox_dir.join("dead"),
    ];
    for dir in &dirs {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".md") {
                    names.insert(name);
                }
            }
        }
    }
    names
}

// ---------------------------------------------------------------------------
// Inbox message creation
// ---------------------------------------------------------------------------

/// Create an inbox message file and return its path.
///
/// `vars` are raw KEY=VALUE pairs from `-v` flags. `body` is the message body
/// text. `chain` and `msg_name` identify the message.
pub fn create_inbox_message(
    project_root: &Path,
    msg_name: &str,
    chain: &str,
    body: &str,
    vars: &[(String, String)],
) -> Result<PathBuf, DecreeError> {
    let inbox_dir = project_root.join(".decree/inbox");
    fs::create_dir_all(&inbox_dir)?;

    let filename = format!("{msg_name}.md");
    let file_path = inbox_dir.join(&filename);

    // Build frontmatter
    let mut fm = String::from("---\n");
    fm.push_str(&format!("chain: \"{chain}\"\n"));
    fm.push_str("seq: 0\n");

    // Determine type from vars
    let input_file = vars.iter().find(|(k, _)| k == "input_file").map(|(_, v)| v.as_str());
    let is_spec = input_file
        .map(|f| f.ends_with(".spec.md"))
        .unwrap_or(false);

    fm.push_str(&format!("type: {}\n", if is_spec { "spec" } else { "task" }));

    // Write all vars as frontmatter fields
    for (key, value) in vars {
        fm.push_str(&format!("{key}: {value}\n"));
    }

    fm.push_str("---\n");
    if !body.is_empty() {
        fm.push_str(body);
        if !body.ends_with('\n') {
            fm.push('\n');
        }
    }

    fs::write(&file_path, &fm)?;
    Ok(file_path)
}

// ---------------------------------------------------------------------------
// Core message processing
// ---------------------------------------------------------------------------

/// Outcome of processing a single message.
#[derive(Debug)]
pub enum ProcessResult {
    /// Message processed successfully.
    Success,
    /// Message was moved to dead-letter with the given reason.
    DeadLettered(String),
}

/// Record of a failed attempt for building failure-context.md.
struct AttemptLog {
    attempt: u32,
    exit_code: Option<i32>,
    log_content: String,
}

/// Process a chain: run the initial message and all depth-first follow-ups.
///
/// Returns `Ok(ProcessResult::Success)` if the root message succeeded (even
/// if follow-ups failed). Returns `Ok(ProcessResult::DeadLettered(_))` if
/// the root message itself was dead-lettered.
pub fn process_chain(
    project_root: &Path,
    config: &Config,
    initial_msg_path: &Path,
    spec_routine: Option<&str>,
) -> Result<ProcessResult, DecreeError> {
    let routines = routine::discover_routines(project_root, config.notebook_support)?;

    // Process the initial message
    let msg = process_single_message(
        project_root,
        config,
        initial_msg_path,
        &routines,
        spec_routine,
    )?;

    let chain_id = match &msg {
        Ok(m) => m.chain.clone(),
        Err(reason) => return Ok(ProcessResult::DeadLettered(reason.clone())),
    };

    // Depth-first: check for follow-up messages in the same chain
    let inbox_dir = project_root.join(".decree/inbox");
    loop {
        let next = find_next_chain_message(&inbox_dir, &chain_id)?;
        match next {
            Some(next_path) => {
                let result = process_single_message(
                    project_root,
                    config,
                    &next_path,
                    &routines,
                    spec_routine,
                )?;
                if let Err(_reason) = result {
                    // Follow-up dead-lettered; continue checking for more
                    continue;
                }
            }
            None => break,
        }
    }

    Ok(ProcessResult::Success)
}

/// Process a single message through the full lifecycle.
///
/// Returns `Ok(Ok(InboxMessage))` on success, `Ok(Err(reason))` if
/// dead-lettered.
fn process_single_message(
    project_root: &Path,
    config: &Config,
    msg_path: &Path,
    routines: &[routine::RoutineInfo],
    spec_routine: Option<&str>,
) -> Result<Result<InboxMessage, String>, DecreeError> {
    let runs_dir = project_root.join(".decree/runs");

    // Build router function
    let config_clone = config.clone();
    let router_fn: Option<Box<RouterFn>> = Some(Box::new(move |prompt: &str| {
        call_router(&config_clone, prompt)
    }));

    // Normalize the message
    let msg = match message::normalize_message(
        msg_path,
        config,
        routines,
        router_fn,
        spec_routine,
    ) {
        Ok(m) => m,
        Err(e) => {
            // Normalization failed — dead-letter
            let reason = format!("normalization failed: {e}");
            eprintln!("  dead-letter: {reason}");
            move_to_dead(project_root, msg_path)?;
            return Ok(Err(reason));
        }
    };

    // Depth check
    if msg.seq >= config.max_depth {
        let reason = format!("MaxDepthExceeded (seq={}, max={})", msg.seq, config.max_depth);
        eprintln!("  dead-letter: {reason}");
        // Append error note to the message
        let mut content = fs::read_to_string(msg_path).unwrap_or_default();
        content.push_str(&format!(
            "\n\n<!-- decree: dead-lettered — {} -->\n",
            reason
        ));
        fs::write(msg_path, content)?;
        move_to_dead(project_root, msg_path)?;
        return Ok(Err(reason));
    }

    // Create run directory
    let msg_dir = runs_dir.join(&msg.id);
    fs::create_dir_all(&msg_dir)?;

    // Copy normalized message to run directory
    fs::copy(msg_path, msg_dir.join("message.md"))?;

    println!("  processing {} (routine: {})", msg.id, msg.routine);

    // Resolve routine
    let resolved = match routine::resolve_routine(
        project_root,
        &msg.routine,
        config.notebook_support,
    ) {
        Ok(r) => r,
        Err(e) => {
            let reason = format!("routine resolution failed: {e}");
            eprintln!("  dead-letter: {reason}");
            move_to_dead(project_root, msg_path)?;
            return Ok(Err(reason));
        }
    };

    // Ensure venv for notebook routines
    if resolved.format == routine::RoutineFormat::Notebook {
        routine::ensure_venv(project_root)?;
    }

    // Create checkpoint (before first attempt)
    let checkpoint = checkpoint::create_checkpoint(project_root, &msg_dir)?;

    // Retry loop
    let max_retries = config.max_retries.max(1);
    let mut attempt_logs: Vec<AttemptLog> = Vec::new();

    for attempt in 1..=max_retries {
        // Before final attempt (if there were prior failures): revert + context
        if attempt == max_retries && attempt > 1 {
            // Capture diff of accumulated partial work
            let _ = checkpoint::finalize_diff(project_root, &checkpoint, &msg_dir);
            // Revert to clean slate
            checkpoint::revert_to_checkpoint(project_root, &checkpoint)?;
            // Write failure context
            write_failure_context(&msg_dir, &attempt_logs)?;
        }

        println!("    attempt {attempt}/{max_retries}");

        // Execute routine
        let result = routine::execute_routine(project_root, &resolved, &msg, &msg_dir)?;

        if result.success {
            // SUCCESS
            let _ = checkpoint::finalize_diff(project_root, &checkpoint, &msg_dir);
            move_to_done(project_root, msg_path)?;

            // If type=spec, mark as processed
            if msg.msg_type == MessageType::Spec {
                if let Some(ref input_file) = msg.input_file {
                    let spec_name = Path::new(input_file)
                        .file_name()
                        .map(|f| f.to_string_lossy().to_string())
                        .unwrap_or_else(|| input_file.clone());
                    spec::mark_processed(project_root, &spec_name)?;
                }
            }

            println!("    success");
            return Ok(Ok(msg));
        }

        // FAILED
        let log_content = fs::read_to_string(&result.log_path).unwrap_or_default();
        let exit_code = result.exit_code;

        eprintln!(
            "    attempt {attempt} failed (exit code: {})",
            exit_code.map(|c| c.to_string()).unwrap_or_else(|| "signal".into())
        );

        attempt_logs.push(AttemptLog {
            attempt,
            exit_code,
            log_content,
        });

        // For non-final attempts: capture diff, keep changes
        if attempt < max_retries {
            let _ = checkpoint::finalize_diff(project_root, &checkpoint, &msg_dir);
        }
    }

    // All retries exhausted
    let _ = checkpoint::finalize_diff(project_root, &checkpoint, &msg_dir);
    checkpoint::revert_to_checkpoint(project_root, &checkpoint)?;
    move_to_dead(project_root, msg_path)?;

    let reason = format!(
        "all {} retries exhausted for {}",
        max_retries, msg.id
    );
    eprintln!("  dead-letter: {reason}");

    Ok(Err(reason))
}

/// Write a failure-context.md summarizing all prior attempt failures.
fn write_failure_context(msg_dir: &Path, logs: &[AttemptLog]) -> Result<(), DecreeError> {
    let mut content = String::from("# Failure Context\n\n");
    content.push_str(&format!(
        "This message has failed {} previous attempt(s). \
         This is the final retry with a clean slate.\n\n",
        logs.len()
    ));

    for log in logs {
        content.push_str(&format!("## Attempt {}\n\n", log.attempt));
        content.push_str(&format!(
            "Exit code: {}\n\n",
            log.exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "killed by signal".into())
        ));
        content.push_str("```\n");
        // Include last 200 lines of log to keep the file manageable
        let lines: Vec<&str> = log.log_content.lines().collect();
        let start = lines.len().saturating_sub(200);
        for line in &lines[start..] {
            content.push_str(line);
            content.push('\n');
        }
        content.push_str("```\n\n");
    }

    fs::write(msg_dir.join("failure-context.md"), content)?;
    Ok(())
}

/// Move a message file to `.decree/inbox/done/`.
fn move_to_done(project_root: &Path, msg_path: &Path) -> Result<(), DecreeError> {
    let done_dir = project_root.join(".decree/inbox/done");
    fs::create_dir_all(&done_dir)?;
    let filename = msg_path
        .file_name()
        .ok_or_else(|| DecreeError::Config("invalid message path".into()))?;
    let dest = done_dir.join(filename);
    fs::rename(msg_path, dest)?;
    Ok(())
}

/// Move a message file to `.decree/inbox/dead/`.
fn move_to_dead(project_root: &Path, msg_path: &Path) -> Result<(), DecreeError> {
    let dead_dir = project_root.join(".decree/inbox/dead");
    fs::create_dir_all(&dead_dir)?;
    let filename = msg_path
        .file_name()
        .ok_or_else(|| DecreeError::Config("invalid message path".into()))?;
    let dest = dead_dir.join(filename);
    fs::rename(msg_path, dest)?;
    Ok(())
}

/// Find the next unprocessed message in the inbox for a given chain,
/// sorted by sequence number.
fn find_next_chain_message(
    inbox_dir: &Path,
    chain: &str,
) -> Result<Option<PathBuf>, DecreeError> {
    if !inbox_dir.is_dir() {
        return Ok(None);
    }

    let mut candidates: Vec<(u32, PathBuf)> = Vec::new();

    for entry in fs::read_dir(inbox_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let filename = entry.file_name().to_string_lossy().to_string();
        if !filename.ends_with(".md") {
            continue;
        }

        // Read the file to check chain ID in frontmatter
        let content = fs::read_to_string(&path)?;
        let (fm, _) = message::parse_message_file(&content);

        let msg_chain = fm.chain.or_else(|| {
            message::chain_seq_from_filename(&filename).map(|(c, _)| c)
        });

        if msg_chain.as_deref() == Some(chain) {
            let seq = fm.seq.unwrap_or_else(|| {
                message::chain_seq_from_filename(&filename)
                    .map(|(_, s)| s)
                    .unwrap_or(0)
            });
            candidates.push((seq, path));
        }
    }

    candidates.sort_by_key(|(seq, _)| *seq);
    Ok(candidates.into_iter().next().map(|(_, path)| path))
}

// ---------------------------------------------------------------------------
// Last-run persistence (interactive recall)
// ---------------------------------------------------------------------------

/// Parameters from the last interactive run, for recall.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LastRun {
    pub routine: String,
    pub message_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_file: Option<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub custom: std::collections::BTreeMap<String, String>,
}

impl LastRun {
    /// Load from `.decree/last-run.yml`, returning `None` if missing.
    pub fn load(project_root: &Path) -> Option<Self> {
        let path = project_root.join(".decree/last-run.yml");
        let content = fs::read_to_string(path).ok()?;
        serde_yaml::from_str(&content).ok()
    }

    /// Save to `.decree/last-run.yml`.
    pub fn save(&self, project_root: &Path) -> Result<(), DecreeError> {
        let path = project_root.join(".decree/last-run.yml");
        let content = serde_yaml::to_string(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_kebab_case_simple() {
        assert_eq!(to_kebab_case("Fix Auth Types"), "fix-auth-types");
    }

    #[test]
    fn test_to_kebab_case_special_chars() {
        assert_eq!(
            to_kebab_case("fix_auth--types!!"),
            "fix-auth-types"
        );
    }

    #[test]
    fn test_to_kebab_case_already_kebab() {
        assert_eq!(to_kebab_case("fix-auth-types"), "fix-auth-types");
    }

    #[test]
    fn test_to_kebab_case_leading_trailing() {
        assert_eq!(to_kebab_case("  --fix--  "), "fix");
    }

    #[test]
    fn test_truncate_str_short() {
        assert_eq!(truncate_str("hello", 50), "hello");
    }

    #[test]
    fn test_truncate_str_exact() {
        let s = "a".repeat(50);
        assert_eq!(truncate_str(&s, 50), s);
    }

    #[test]
    fn test_truncate_str_long() {
        let s = "a".repeat(60);
        assert_eq!(truncate_str(&s, 50).len(), 50);
    }

    #[test]
    fn test_truncate_str_no_trailing_hyphen() {
        // When truncation cuts at a hyphen, it should be stripped
        assert_eq!(truncate_str("abc-def", 4), "abc");
    }

    #[test]
    fn test_dedup_name_no_collision() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(dedup_name("fix-types", dir.path()), "fix-types");
    }

    #[test]
    fn test_dedup_name_with_collision() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("fix-types.md"), "").unwrap();
        assert_eq!(dedup_name("fix-types", dir.path()), "fix-types-2");
    }

    #[test]
    fn test_dedup_name_multiple_collisions() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("fix-types.md"), "").unwrap();
        fs::write(dir.path().join("fix-types-2.md"), "").unwrap();
        assert_eq!(dedup_name("fix-types", dir.path()), "fix-types-3");
    }

    #[test]
    fn test_dedup_name_checks_done_subdir() {
        let dir = tempfile::tempdir().unwrap();
        let done = dir.path().join("done");
        fs::create_dir_all(&done).unwrap();
        fs::write(done.join("fix-types.md"), "").unwrap();
        assert_eq!(dedup_name("fix-types", dir.path()), "fix-types-2");
    }

    #[test]
    fn test_create_inbox_message_task() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".decree/inbox")).unwrap();

        let vars = vec![
            ("routine".to_string(), "develop".to_string()),
        ];
        let path = create_inbox_message(root, "fix-types", "20260226143200", "Fix the types", &vars).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("type: task"));
        assert!(content.contains("chain: \"20260226143200\""));
        assert!(content.contains("routine: develop"));
        assert!(content.contains("Fix the types"));
    }

    #[test]
    fn test_create_inbox_message_spec() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".decree/inbox")).unwrap();

        let vars = vec![
            ("input_file".to_string(), "specs/01-add-auth.spec.md".to_string()),
        ];
        let path = create_inbox_message(root, "add-auth", "20260226143200", "", &vars).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("type: spec"));
        assert!(content.contains("input_file: specs/01-add-auth.spec.md"));
    }

    #[test]
    fn test_shell_escape() {
        assert_eq!(shell_escape("hello"), "'hello'");
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }

    #[test]
    fn test_failure_context_format() {
        let dir = tempfile::tempdir().unwrap();
        let logs = vec![
            AttemptLog {
                attempt: 1,
                exit_code: Some(1),
                log_content: "error: compilation failed\n".to_string(),
            },
            AttemptLog {
                attempt: 2,
                exit_code: Some(1),
                log_content: "error: tests failed\n".to_string(),
            },
        ];

        write_failure_context(dir.path(), &logs).unwrap();
        let content = fs::read_to_string(dir.path().join("failure-context.md")).unwrap();

        assert!(content.contains("# Failure Context"));
        assert!(content.contains("## Attempt 1"));
        assert!(content.contains("## Attempt 2"));
        assert!(content.contains("compilation failed"));
        assert!(content.contains("tests failed"));
        assert!(content.contains("failed 2 previous attempt(s)"));
    }

    #[test]
    fn test_move_to_done() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let inbox = root.join(".decree/inbox");
        fs::create_dir_all(&inbox).unwrap();

        let msg = inbox.join("test-msg.md");
        fs::write(&msg, "test").unwrap();

        move_to_done(root, &msg).unwrap();
        assert!(!msg.exists());
        assert!(root.join(".decree/inbox/done/test-msg.md").exists());
    }

    #[test]
    fn test_move_to_dead() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let inbox = root.join(".decree/inbox");
        fs::create_dir_all(&inbox).unwrap();

        let msg = inbox.join("test-msg.md");
        fs::write(&msg, "test").unwrap();

        move_to_dead(root, &msg).unwrap();
        assert!(!msg.exists());
        assert!(root.join(".decree/inbox/dead/test-msg.md").exists());
    }

    #[test]
    fn test_last_run_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".decree")).unwrap();

        let lr = LastRun {
            routine: "develop".into(),
            message_name: "fix-auth".into(),
            input_file: Some("specs/01.spec.md".into()),
            custom: [("target_branch".into(), "main".into())].into(),
        };

        lr.save(root).unwrap();
        let loaded = LastRun::load(root).unwrap();
        assert_eq!(loaded.routine, "develop");
        assert_eq!(loaded.message_name, "fix-auth");
        assert_eq!(loaded.input_file.as_deref(), Some("specs/01.spec.md"));
        assert_eq!(loaded.custom.get("target_branch").unwrap(), "main");
    }
}
