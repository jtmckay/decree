use crate::commands::routine_sync;
use crate::config::{self, AppConfig};
use crate::cron::{self, CronTracker};
use crate::error::DecreeError;
use crate::hooks::{self, HookContext, HookType};
use crate::message::{self, InboxMessage};
use crate::routine;
use std::collections::BTreeMap;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Run the daemon polling loop.
pub fn run(project_root: &Path, interval: u64) -> Result<(), DecreeError> {
    let mut config = AppConfig::load_from_project(project_root)?;

    // Run discovery at startup
    if routine_sync::discover(project_root, &mut config, None)? {
        config.save(project_root)?;
    }

    // Set up signal handling for SIGINT and SIGTERM
    let shutdown = Arc::new(AtomicBool::new(false));
    register_signal_handlers(Arc::clone(&shutdown))?;

    println!("decree daemon: polling every {interval}s");

    // Run beforeAll hook
    let all_ctx = HookContext::default();
    if let Err(e) = hooks::run_hook_with_config(project_root, &config.hooks, HookType::BeforeAll, &all_ctx, Some(&config)) {
        eprintln!("beforeAll hook failed: {e}");
        return Err(DecreeError::Other(format!("beforeAll hook failed: {e}")));
    }

    let mut cron_tracker = CronTracker::new();

    // Main polling loop
    loop {
        if shutdown.load(Ordering::Relaxed) {
            println!("decree daemon: shutting down (signal received)");
            // Do NOT run afterAll on signal shutdown
            return Ok(());
        }

        // Step 1-2: Check cron and fire due jobs into inbox
        fire_due_cron_jobs(project_root, &mut cron_tracker);

        // Step 3-4: Process inbox messages
        loop {
            if shutdown.load(Ordering::Relaxed) {
                println!("decree daemon: shutting down (signal received)");
                return Ok(());
            }

            let inbox = message::list_inbox_messages(project_root)?;
            if inbox.is_empty() {
                break;
            }

            // LIFO: process last (newest) message first
            let filename = inbox.last().unwrap().clone();

            match process_single_message(project_root, &config, &filename, &shutdown) {
                Ok(()) => {}
                Err(e) => {
                    // Dead-lettered messages don't halt the daemon
                    eprintln!("decree daemon: error processing {filename}: {e}");
                }
            }
        }

        // Step 5: Sleep for the interval (check shutdown periodically)
        for _ in 0..interval {
            if shutdown.load(Ordering::Relaxed) {
                println!("decree daemon: shutting down (signal received)");
                return Ok(());
            }
            thread::sleep(Duration::from_secs(1));
        }
    }
}

/// Register SIGINT and SIGTERM handlers to set the shutdown flag.
fn register_signal_handlers(shutdown: Arc<AtomicBool>) -> Result<(), DecreeError> {
    let shutdown2 = Arc::clone(&shutdown);
    signal_hook::flag::register(signal_hook::consts::SIGINT, shutdown)?;
    signal_hook::flag::register(signal_hook::consts::SIGTERM, shutdown2)?;
    Ok(())
}

/// Check cron directory and fire due jobs into inbox.
fn fire_due_cron_jobs(project_root: &Path, tracker: &mut CronTracker) {
    let cron_files = match cron::scan_cron_files(project_root) {
        Ok(files) => files,
        Err(e) => {
            eprintln!("decree daemon: error scanning cron: {e}");
            return;
        }
    };

    for cf in &cron_files {
        if !tracker.is_due(cf) {
            continue;
        }

        match cron::cron_to_inbox_message(project_root, cf) {
            Ok(msg) => {
                // Ensure inbox directory exists
                let inbox_dir = project_root
                    .join(config::DECREE_DIR)
                    .join(config::INBOX_DIR);
                if let Err(e) = std::fs::create_dir_all(&inbox_dir) {
                    eprintln!("decree daemon: failed to create inbox dir: {e}");
                    continue;
                }
                if let Err(e) = msg.write_to_inbox(project_root) {
                    eprintln!(
                        "decree daemon: failed to write cron message for {}: {e}",
                        cf.filename
                    );
                    continue;
                }
                println!("decree daemon: cron fired: {} -> {}", cf.filename, msg.filename);
                tracker.mark_fired(cf);
            }
            Err(e) => {
                eprintln!(
                    "decree daemon: failed to create message for {}: {e}",
                    cf.filename
                );
            }
        }
    }
}

/// Process a single inbox message through the full pipeline.
fn process_single_message(
    project_root: &Path,
    config: &AppConfig,
    filename: &str,
    shutdown: &AtomicBool,
) -> Result<(), DecreeError> {
    // Parse and normalize the message
    let mut msg = InboxMessage::from_file(project_root, filename)?;
    let was_modified = msg.normalize(project_root, config, None)?;

    // If normalization changed the message (including possible rename), update on disk
    // and track the new filename for all subsequent operations.
    let active_filename: String = if was_modified {
        let new_filename = msg
            .id
            .as_ref()
            .map(|id| format!("{id}.md"))
            .unwrap_or_else(|| filename.to_string());
        if new_filename != filename {
            let old_path = project_root
                .join(config::DECREE_DIR)
                .join(config::INBOX_DIR)
                .join(filename);
            let new_path = project_root
                .join(config::DECREE_DIR)
                .join(config::INBOX_DIR)
                .join(&new_filename);
            if old_path.exists() {
                std::fs::rename(&old_path, &new_path)?;
            }
            msg.filename = new_filename.clone();
            msg.write_to_inbox(project_root)?;
            new_filename
        } else {
            msg.write_to_inbox(project_root)?;
            filename.to_string()
        }
    } else {
        filename.to_string()
    };

    let chain = msg
        .chain
        .as_ref()
        .ok_or_else(|| DecreeError::Other("message has no chain after normalization".into()))?
        .clone();
    let seq = msg
        .seq
        .ok_or_else(|| DecreeError::Other("message has no seq after normalization".into()))?;
    let msg_id = msg
        .id
        .as_ref()
        .ok_or_else(|| DecreeError::Other("message has no id after normalization".into()))?
        .clone();
    let routine_name = msg
        .routine
        .as_ref()
        .ok_or_else(|| DecreeError::Other("message has no routine after normalization".into()))?
        .clone();
    let trigger = msg.trigger.clone().unwrap_or_else(|| "inbox".to_string());

    // Resolve per-routine max_attempts and timeout_s
    let effective_max_attempts = config
        .routines
        .as_ref()
        .and_then(|r| r.get(&routine_name))
        .and_then(|e| e.max_attempts)
        .unwrap_or(config.max_attempts);
    let timeout_s = config
        .routines
        .as_ref()
        .and_then(|r| r.get(&routine_name))
        .and_then(|e| e.timeout_s);

    // Check depth limit
    if seq >= config.max_depth {
        eprintln!(
            "decree daemon: max depth exceeded for {msg_id} (seq={seq}, limit={})",
            config.max_depth
        );
        dead_letter(project_root, &active_filename)?;
        return Err(DecreeError::MaxDepthExceeded(config.max_depth));
    }

    // Create run directory
    let run_dir = project_root
        .join(config::DECREE_DIR)
        .join(config::RUNS_DIR)
        .join(msg_id.clone());
    std::fs::create_dir_all(&run_dir)?;

    // Copy normalized message to run dir
    std::fs::write(run_dir.join("message.md"), msg.serialize())?;

    // Find the routine script (registry-aware layered lookup)
    let script_path = match routine::resolve_routine(project_root, config, &routine_name) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("decree daemon: routine resolution failed for {msg_id}: {e}");
            dead_letter(project_root, &active_filename)?;
            return Err(e);
        }
    };

    let msg_file_path = project_root
        .join(config::DECREE_DIR)
        .join(config::INBOX_DIR)
        .join(&active_filename);

    let run_start = chrono::Local::now();
    let mut last_exit_code: i32 = 1;
    let mut total_attempts: u32 = 0;

    // Retry loop
    for attempt in 1..=effective_max_attempts {
        total_attempts = attempt;

        if shutdown.load(Ordering::Relaxed) {
            return Ok(());
        }

        let is_final = attempt == effective_max_attempts;

        // Build hook context
        let hook_ctx = HookContext {
            message_file: msg_file_path.to_string_lossy().to_string(),
            message_id: msg_id.clone(),
            message_dir: run_dir.to_string_lossy().to_string(),
            chain: chain.clone(),
            seq: seq.to_string(),
            attempt: Some(attempt),
            max_attempts: Some(effective_max_attempts),
            routine_exit_code: None,
            final_attempt: false,
            trigger: trigger.clone(),
        };

        // Run beforeEach hook
        match hooks::run_hook_with_config(project_root, &config.hooks, HookType::BeforeEach, &hook_ctx, Some(config)) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("decree daemon: beforeEach hook failed for {msg_id}: {e}");
                // beforeEach failure: dead-letter immediately; onDeadLetter does NOT fire
                dead_letter(project_root, &active_filename)?;
                return Err(DecreeError::Other(format!("beforeEach failed: {e}")));
            }
        }

        // Execute routine
        let log_file = if attempt == 1 {
            "routine.log".to_string()
        } else {
            format!("routine-{attempt}.log")
        };
        let log_path = run_dir.join(&log_file);

        println!("decree daemon: processing {msg_id} (attempt {attempt}/{effective_max_attempts}) via {routine_name}");

        let start = chrono::Local::now();
        let start_line = format!("[decree] start {}\n", start.format("%Y-%m-%dT%H:%M:%S"));
        std::fs::write(&log_path, &start_line)?;

        let exit_code = execute_routine(
            project_root,
            &script_path,
            &msg,
            &run_dir,
            &log_path,
            &trigger,
            timeout_s,
            shutdown,
        )?;
        last_exit_code = exit_code;

        // Check for shutdown after routine completes
        if shutdown.load(Ordering::Relaxed) {
            let end = chrono::Local::now();
            let duration = end.signed_duration_since(start);
            let end_line = format!(
                "[decree] duration {} end {}\n",
                format_duration(duration),
                end.format("%Y-%m-%dT%H:%M:%S")
            );
            let _ = append_to_file(&log_path, &end_line);
            return Ok(());
        }

        // Write end timestamp to log
        let end = chrono::Local::now();
        let duration = end.signed_duration_since(start);
        let duration_str = format_duration(duration);
        let end_line = format!(
            "[decree] duration {} end {}\n",
            duration_str,
            end.format("%Y-%m-%dT%H:%M:%S")
        );
        append_to_file(&log_path, &end_line)?;

        // Truncate log if needed
        truncate_log_if_needed(&log_path, config.max_log_size)?;

        if exit_code == 0 {
            // SUCCESS
            let after_ctx = HookContext {
                routine_exit_code: Some(0),
                final_attempt: is_final,
                ..hook_ctx.clone()
            };
            if let Err(e) =
                hooks::run_hook_with_config(project_root, &config.hooks, HookType::AfterEach, &after_ctx, Some(config))
            {
                eprintln!("decree daemon: afterEach hook failed for {msg_id}: {e}");
            }

            // Collect outbox
            collect_outbox(project_root, &chain, seq, config)?;

            // Delete message from inbox
            let inbox_path = project_root
                .join(config::DECREE_DIR)
                .join(config::INBOX_DIR)
                .join(&active_filename);
            if inbox_path.exists() {
                std::fs::remove_file(&inbox_path)?;
            }

            // If message has migration field, mark as processed
            if let Some(ref migration) = msg.migration {
                message::mark_processed(project_root, migration)?;
            }

            // Write run.json
            let run_end = chrono::Local::now();
            write_run_json(
                &run_dir,
                &msg_id,
                &routine_name,
                &trigger,
                msg.migration.as_deref(),
                attempt,
                0,
                &run_start,
                &run_end,
            )?;

            return Ok(());
        }

        // FAILURE
        let after_ctx = HookContext {
            routine_exit_code: Some(exit_code),
            final_attempt: is_final,
            ..hook_ctx
        };
        if let Err(e) =
            hooks::run_hook_with_config(project_root, &config.hooks, HookType::AfterEach, &after_ctx, Some(config))
        {
            eprintln!("decree daemon: afterEach hook failed for {msg_id}: {e}");
        }

        if attempt == effective_max_attempts {
            // EXHAUSTION — all retries failed
            eprintln!(
                "decree daemon: max retries exhausted for {msg_id} (exit code: {exit_code})"
            );

            // Clear outbox (discard follow-ups from failed routine)
            clear_outbox(project_root)?;

            // Dead-letter the message
            dead_letter(project_root, &active_filename)?;

            // Write run.json before firing hook
            let run_end = chrono::Local::now();
            write_run_json(
                &run_dir,
                &msg_id,
                &routine_name,
                &trigger,
                msg.migration.as_deref(),
                attempt,
                exit_code,
                &run_start,
                &run_end,
            )?;

            // Fire onDeadLetter hook (warning-only on failure)
            let dead_file_path = project_root
                .join(config::DECREE_DIR)
                .join(config::INBOX_DIR)
                .join(config::DEAD_DIR)
                .join(&active_filename);
            let dead_ctx = HookContext {
                message_file: dead_file_path.to_string_lossy().to_string(),
                message_id: msg_id.clone(),
                message_dir: run_dir.to_string_lossy().to_string(),
                chain: chain.clone(),
                seq: seq.to_string(),
                attempt: Some(effective_max_attempts),
                max_attempts: Some(effective_max_attempts),
                routine_exit_code: Some(exit_code),
                final_attempt: false,
                trigger: trigger.clone(),
            };
            if let Err(e) = hooks::run_hook_with_config(
                project_root,
                &config.hooks,
                HookType::OnDeadLetter,
                &dead_ctx,
                Some(config),
            ) {
                eprintln!("decree daemon: onDeadLetter hook failed for {msg_id}: {e}");
            }

            return Err(DecreeError::MaxRetriesExhausted(msg_id));
        }
    }

    // Unexpected loop exit (shouldn't happen)
    let run_end = chrono::Local::now();
    let _ = write_run_json(
        &run_dir,
        &msg_id,
        &routine_name,
        &trigger,
        msg.migration.as_deref(),
        total_attempts,
        last_exit_code,
        &run_start,
        &run_end,
    );

    Ok(())
}

/// Execute a routine script and return its exit code.
fn execute_routine(
    project_root: &Path,
    script_path: &Path,
    msg: &InboxMessage,
    run_dir: &Path,
    log_path: &Path,
    trigger: &str,
    timeout_s: Option<u32>,
    shutdown: &AtomicBool,
) -> Result<i32, DecreeError> {
    let msg_file_path = project_root
        .join(config::DECREE_DIR)
        .join(config::INBOX_DIR)
        .join(&msg.filename);

    let msg_id = msg.id.as_deref().unwrap_or("");
    let chain = msg.chain.as_deref().unwrap_or("");
    let seq = msg.seq.map(|s| s.to_string()).unwrap_or_default();

    // Execute: bash <script> 2>&1 | tee -a <log_path>
    // Using shell to handle the pipe with pipefail so we get the script's exit code
    let cmd_str = format!(
        "set -o pipefail; bash {} 2>&1 | tee -a {}",
        shell_escape(script_path.to_string_lossy().as_ref()),
        shell_escape(log_path.to_string_lossy().as_ref()),
    );

    let mut cmd = std::process::Command::new("bash");
    cmd.arg("-c")
        .arg(&cmd_str)
        .current_dir(project_root)
        .env("message_file", msg_file_path.to_string_lossy().as_ref())
        .env("message_id", msg_id)
        .env("message_dir", run_dir.to_string_lossy().as_ref())
        .env("chain", chain)
        .env("seq", &seq)
        .env("DECREE_TRIGGER", trigger);

    // Pass custom fields as env vars
    for (key, value) in &msg.custom_fields {
        if let Some(s) = value_as_env_string(value) {
            cmd.env(key, &s);
        }
    }

    // Put child in its own process group so we can kill the entire tree.
    cmd.process_group(0);

    // Routines run unattended — no terminal input needed.
    cmd.stdin(std::process::Stdio::null());

    let mut child = cmd.spawn()?;
    let child_id = child.id();

    let start_time = std::time::Instant::now();

    // Poll for completion, checking for shutdown and timeout between iterations.
    let exit_code = loop {
        match child.try_wait()? {
            Some(status) => break status.code().unwrap_or(1),
            None => {
                if shutdown.load(Ordering::Relaxed) {
                    unsafe {
                        libc::kill(-(child_id as i32), libc::SIGTERM);
                    }
                    let _ = child.wait();
                    // Return — caller sees shutdown flag and returns Ok
                    return Ok(130);
                }
                if let Some(t) = timeout_s {
                    if start_time.elapsed() >= std::time::Duration::from_secs(t as u64) {
                        unsafe {
                            libc::kill(-(child_id as i32), libc::SIGTERM);
                        }
                        let _ = child.wait();
                        return Ok(1);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    };

    Ok(exit_code)
}

/// Collect outbox messages and move them to inbox.
fn collect_outbox(
    project_root: &Path,
    chain: &str,
    current_seq: u32,
    config: &AppConfig,
) -> Result<(), DecreeError> {
    let outbox_dir = project_root
        .join(config::DECREE_DIR)
        .join(config::OUTBOX_DIR);

    if !outbox_dir.exists() {
        return Ok(());
    }

    let mut entries: Vec<String> = std::fs::read_dir(&outbox_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();

    entries.sort();

    // Warn about non-.md files
    for entry in &entries {
        if !entry.ends_with(".md") {
            eprintln!("Warning: non-.md file in outbox ignored: {entry}");
        }
    }

    let md_files: Vec<String> = entries
        .into_iter()
        .filter(|e| e.ends_with(".md"))
        .collect();

    let mut next_seq = current_seq + 1;

    let inbox_dir = project_root
        .join(config::DECREE_DIR)
        .join(config::INBOX_DIR);
    std::fs::create_dir_all(&inbox_dir)?;

    let outbox_dead_dir = outbox_dir.join(config::DEAD_DIR);

    for file in &md_files {
        let file_path = outbox_dir.join(file);
        let content = std::fs::read_to_string(&file_path)?;

        // Check depth limit
        if next_seq >= config.max_depth {
            eprintln!(
                "Warning: MaxDepthExceeded for outbox file {file} (seq={next_seq}, limit={})",
                config.max_depth
            );
            std::fs::create_dir_all(&outbox_dead_dir)?;
            std::fs::rename(&file_path, outbox_dead_dir.join(file))?;
            continue;
        }

        let (fields, body) = message::parse_frontmatter(&content)?;

        // Build inbox message
        let id = format!("{chain}-{next_seq}");
        let inbox_filename = format!("{id}.md");

        let routine = fields.get("routine").and_then(|v| match v {
            serde_yaml::Value::String(s) => Some(s.clone()),
            _ => None,
        });

        // Collect custom fields (strip known message fields)
        let known: &[&str] = &["id", "chain", "seq", "routine", "migration", "trigger"];
        let custom_fields: BTreeMap<String, serde_yaml::Value> = fields
            .into_iter()
            .filter(|(k, _)| !known.contains(&k.as_str()))
            .collect();

        let inbox_msg = InboxMessage {
            id: Some(id),
            chain: Some(chain.to_string()),
            seq: Some(next_seq),
            routine,
            migration: None,
            trigger: Some("chain".to_string()),
            body,
            custom_fields,
            filename: inbox_filename,
        };

        inbox_msg.write_to_inbox(project_root)?;
        std::fs::remove_file(&file_path)?;
        next_seq += 1;
    }

    Ok(())
}

/// Clear the outbox without collecting (used on exhaustion).
fn clear_outbox(project_root: &Path) -> Result<(), DecreeError> {
    let outbox_dir = project_root
        .join(config::DECREE_DIR)
        .join(config::OUTBOX_DIR);

    if !outbox_dir.exists() {
        return Ok(());
    }

    for entry in std::fs::read_dir(&outbox_dir)? {
        let entry = entry?;
        if entry.path().is_file() {
            std::fs::remove_file(entry.path())?;
        }
    }

    Ok(())
}

/// Move a message to the dead-letter directory.
fn dead_letter(project_root: &Path, filename: &str) -> Result<(), DecreeError> {
    let inbox_path = project_root
        .join(config::DECREE_DIR)
        .join(config::INBOX_DIR)
        .join(filename);

    let dead_dir = project_root
        .join(config::DECREE_DIR)
        .join(config::INBOX_DIR)
        .join(config::DEAD_DIR);

    std::fs::create_dir_all(&dead_dir)?;

    let dead_path = dead_dir.join(filename);
    if inbox_path.exists() {
        std::fs::rename(&inbox_path, &dead_path)?;
    }

    Ok(())
}

/// Append text to a file.
fn append_to_file(path: &Path, text: &str) -> Result<(), DecreeError> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(text.as_bytes())?;
    Ok(())
}

/// Truncate a log file to max_log_size bytes, keeping the tail.
fn truncate_log_if_needed(path: &Path, max_size: u64) -> Result<(), DecreeError> {
    if max_size == 0 {
        return Ok(());
    }

    let metadata = std::fs::metadata(path)?;
    if metadata.len() <= max_size {
        return Ok(());
    }

    let content = std::fs::read(path)?;
    let skip = content.len() - max_size as usize;
    let truncated = &content[skip..];

    let marker = format!("[log truncated — showing last {} of output]\n", format_bytes(max_size));
    let mut new_content = marker.into_bytes();
    new_content.extend_from_slice(truncated);

    std::fs::write(path, &new_content)?;

    Ok(())
}

/// Write run.json metadata to the run directory.
#[allow(clippy::too_many_arguments)]
fn write_run_json(
    run_dir: &Path,
    message_id: &str,
    routine: &str,
    trigger: &str,
    migration: Option<&str>,
    attempts: u32,
    exit_code: i32,
    start: &chrono::DateTime<chrono::Local>,
    end: &chrono::DateTime<chrono::Local>,
) -> Result<(), DecreeError> {
    let duration_s = end.signed_duration_since(*start).num_seconds();

    let mut obj = serde_json::Map::new();
    obj.insert("message_id".into(), serde_json::Value::String(message_id.into()));
    obj.insert("routine".into(), serde_json::Value::String(routine.into()));
    obj.insert("trigger".into(), serde_json::Value::String(trigger.into()));
    if let Some(m) = migration {
        obj.insert("migration".into(), serde_json::Value::String(m.into()));
    }
    obj.insert("attempts".into(), serde_json::Value::Number(attempts.into()));
    obj.insert("exit_code".into(), serde_json::Value::Number(exit_code.into()));
    obj.insert("start".into(), serde_json::Value::String(start.format("%Y-%m-%dT%H:%M:%S").to_string()));
    obj.insert("end".into(), serde_json::Value::String(end.format("%Y-%m-%dT%H:%M:%S").to_string()));
    obj.insert("duration_s".into(), serde_json::Value::Number(duration_s.into()));

    let json = serde_json::to_string_pretty(&serde_json::Value::Object(obj))
        .map_err(|e| DecreeError::Other(format!("failed to serialize run.json: {e}")))?;

    std::fs::write(run_dir.join("run.json"), json)?;
    Ok(())
}

/// Format a byte count for the truncation marker.
fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{}MB", bytes / 1_048_576)
    } else if bytes >= 1024 {
        format!("{}KB", bytes / 1024)
    } else {
        format!("{bytes}B")
    }
}

/// Format a chrono Duration as human-readable.
fn format_duration(d: chrono::TimeDelta) -> String {
    let total_secs = d.num_seconds();
    if total_secs < 60 {
        format!("{total_secs}s")
    } else {
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        format!("{mins}m{secs:02}s")
    }
}

/// Simple shell escaping for paths.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Convert a serde_yaml::Value to a string suitable for env vars.
fn value_as_env_string(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        serde_yaml::Value::Null => None,
        serde_yaml::Value::Sequence(_) | serde_yaml::Value::Mapping(_) => {
            let json_val: serde_json::Value = serde_json::to_value(v).ok()?;
            Some(json_val.to_string())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_decree_dir(dir: &TempDir) {
        let decree = dir.path().join(".decree");
        std::fs::create_dir_all(decree.join("inbox")).unwrap();
        std::fs::create_dir_all(decree.join("outbox")).unwrap();
        std::fs::create_dir_all(decree.join("runs")).unwrap();
        std::fs::create_dir_all(decree.join("routines")).unwrap();
        std::fs::create_dir_all(decree.join("cron")).unwrap();
        std::fs::write(decree.join("processed.md"), "").unwrap();
        std::fs::write(
            decree.join("config.yml"),
            "commands:\n  ai_router: echo\n  ai_interactive: echo\n",
        )
        .unwrap();
    }

    #[test]
    fn test_format_duration_seconds() {
        let d = chrono::TimeDelta::seconds(45);
        assert_eq!(format_duration(d), "45s");
    }

    #[test]
    fn test_format_duration_minutes() {
        let d = chrono::TimeDelta::seconds(125);
        assert_eq!(format_duration(d), "2m05s");
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500B");
        assert_eq!(format_bytes(2048), "2KB");
        assert_eq!(format_bytes(2_097_152), "2MB");
    }

    #[test]
    fn test_shell_escape() {
        assert_eq!(shell_escape("simple"), "'simple'");
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }

    #[test]
    fn test_dead_letter() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        let inbox = dir.path().join(".decree/inbox");
        std::fs::write(inbox.join("test-0.md"), "content").unwrap();

        dead_letter(dir.path(), "test-0.md").unwrap();

        assert!(!inbox.join("test-0.md").exists());
        assert!(inbox.join("dead/test-0.md").exists());
    }

    #[test]
    fn test_dead_letter_nonexistent() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        // Should not error on nonexistent file
        let result = dead_letter(dir.path(), "nonexistent.md");
        assert!(result.is_ok());
    }

    #[test]
    fn test_clear_outbox() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        let outbox = dir.path().join(".decree/outbox");
        std::fs::write(outbox.join("msg1.md"), "content1").unwrap();
        std::fs::write(outbox.join("msg2.md"), "content2").unwrap();

        clear_outbox(dir.path()).unwrap();

        let remaining: Vec<_> = std::fs::read_dir(&outbox)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .collect();
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_clear_outbox_no_dir() {
        let dir = TempDir::new().unwrap();
        // No .decree at all
        let result = clear_outbox(dir.path());
        assert!(result.is_ok());
    }

    #[test]
    fn test_collect_outbox_empty() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        let config = AppConfig::default();
        let result = collect_outbox(dir.path(), "D0001-1432-test", 0, &config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_collect_outbox_creates_inbox_messages() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        let outbox = dir.path().join(".decree/outbox");
        std::fs::write(
            outbox.join("followup.md"),
            "---\nroutine: develop\n---\nFollow-up task.\n",
        )
        .unwrap();

        let config = AppConfig::default();
        collect_outbox(dir.path(), "D0001-1432-test", 0, &config).unwrap();

        // Should create inbox message with seq=1
        let inbox = dir.path().join(".decree/inbox");
        assert!(inbox.join("D0001-1432-test-1.md").exists());

        // Outbox file should be removed
        assert!(!outbox.join("followup.md").exists());

        // Verify inbox message content
        let content = std::fs::read_to_string(inbox.join("D0001-1432-test-1.md")).unwrap();
        assert!(content.contains("chain: D0001-1432-test"));
        assert!(content.contains("seq: 1"));
        assert!(content.contains("routine: develop"));
        assert!(content.contains("Follow-up task."));
        assert!(content.contains("trigger: chain"));
    }

    #[test]
    fn test_collect_outbox_preserves_custom_fields() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        let outbox = dir.path().join(".decree/outbox");
        std::fs::write(
            outbox.join("followup.md"),
            "---\npriority: high\n---\nBody.\n",
        )
        .unwrap();

        let config = AppConfig::default();
        collect_outbox(dir.path(), "D0001-1432-test", 0, &config).unwrap();

        let content =
            std::fs::read_to_string(dir.path().join(".decree/inbox/D0001-1432-test-1.md"))
                .unwrap();
        assert!(content.contains("priority: high"));
    }

    #[test]
    fn test_collect_outbox_multiple_files() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        let outbox = dir.path().join(".decree/outbox");
        std::fs::write(outbox.join("01-first.md"), "First.\n").unwrap();
        std::fs::write(outbox.join("02-second.md"), "Second.\n").unwrap();

        let config = AppConfig::default();
        collect_outbox(dir.path(), "D0001-1432-test", 0, &config).unwrap();

        let inbox = dir.path().join(".decree/inbox");
        assert!(inbox.join("D0001-1432-test-1.md").exists());
        assert!(inbox.join("D0001-1432-test-2.md").exists());
    }

    #[test]
    fn test_collect_outbox_depth_limit() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        let outbox = dir.path().join(".decree/outbox");
        std::fs::write(outbox.join("followup.md"), "Too deep.\n").unwrap();

        let config = AppConfig {
            max_depth: 3,
            ..AppConfig::default()
        };

        // Current seq is 2, so next would be 3 which equals max_depth
        collect_outbox(dir.path(), "D0001-1432-test", 2, &config).unwrap();

        // Should be dead-lettered in outbox, not moved to inbox
        let inbox = dir.path().join(".decree/inbox");
        assert!(!inbox.join("D0001-1432-test-3.md").exists());
        assert!(outbox.join("dead/followup.md").exists());
    }

    #[test]
    fn test_truncate_log_disabled() {
        let dir = TempDir::new().unwrap();
        let log = dir.path().join("test.log");
        std::fs::write(&log, "a".repeat(5000)).unwrap();

        // max_size = 0 disables truncation
        truncate_log_if_needed(&log, 0).unwrap();
        assert_eq!(std::fs::metadata(&log).unwrap().len(), 5000);
    }

    #[test]
    fn test_truncate_log_under_limit() {
        let dir = TempDir::new().unwrap();
        let log = dir.path().join("test.log");
        std::fs::write(&log, "small log").unwrap();

        truncate_log_if_needed(&log, 1000).unwrap();
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "small log");
    }

    #[test]
    fn test_truncate_log_over_limit() {
        let dir = TempDir::new().unwrap();
        let log = dir.path().join("test.log");
        let content = "x".repeat(200);
        std::fs::write(&log, &content).unwrap();

        truncate_log_if_needed(&log, 100).unwrap();

        let result = std::fs::read_to_string(&log).unwrap();
        assert!(result.starts_with("[log truncated"));
        assert!(result.contains("100B"));
        // Should end with 100 'x' chars
        assert!(result.ends_with(&"x".repeat(100)));
    }

    #[test]
    fn test_process_single_message_success() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        // Write a trivial routine
        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\necho 'done'\n",
        )
        .unwrap();

        // Write an inbox message
        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\nroutine: develop\ntrigger: inbox\n---\nTest body.\n";
        std::fs::write(
            dir.path().join(".decree/inbox/D0001-1432-test-0.md"),
            content,
        )
        .unwrap();

        let config = AppConfig::load_from_project(dir.path()).unwrap();
        let shutdown = AtomicBool::new(false);

        let result =
            process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);
        assert!(result.is_ok());

        // Message should be removed from inbox
        assert!(!dir
            .path()
            .join(".decree/inbox/D0001-1432-test-0.md")
            .exists());

        // Run directory should exist
        assert!(dir
            .path()
            .join(".decree/runs/D0001-1432-test-0/message.md")
            .exists());
        assert!(dir
            .path()
            .join(".decree/runs/D0001-1432-test-0/routine.log")
            .exists());
    }

    #[test]
    fn test_process_single_message_writes_run_json() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\necho 'done'\n",
        )
        .unwrap();

        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\nroutine: develop\ntrigger: inbox\n---\nBody.\n";
        std::fs::write(
            dir.path().join(".decree/inbox/D0001-1432-test-0.md"),
            content,
        )
        .unwrap();

        let config = AppConfig::load_from_project(dir.path()).unwrap();
        let shutdown = AtomicBool::new(false);

        process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown).unwrap();

        let run_json_path = dir.path().join(".decree/runs/D0001-1432-test-0/run.json");
        assert!(run_json_path.exists());

        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&run_json_path).unwrap()).unwrap();
        assert_eq!(json["exit_code"], 0);
        assert_eq!(json["attempts"], 1);
        assert_eq!(json["trigger"], "inbox");
        assert_eq!(json["routine"], "develop");
    }

    #[test]
    fn test_process_single_message_failure_dead_letters() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        // Write a routine that fails
        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\nexit 1\n",
        )
        .unwrap();

        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\nroutine: develop\ntrigger: inbox\n---\nTest.\n";
        std::fs::write(
            dir.path().join(".decree/inbox/D0001-1432-test-0.md"),
            content,
        )
        .unwrap();

        let config = AppConfig {
            max_attempts: 1,
            ..AppConfig::load_from_project(dir.path()).unwrap()
        };
        let shutdown = AtomicBool::new(false);

        let result =
            process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);
        assert!(result.is_err());

        // Message should be dead-lettered
        assert!(dir
            .path()
            .join(".decree/inbox/dead/D0001-1432-test-0.md")
            .exists());
    }

    #[test]
    fn test_process_single_message_retries() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        // Write a routine that fails
        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\nexit 1\n",
        )
        .unwrap();

        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\nroutine: develop\ntrigger: inbox\n---\nTest.\n";
        std::fs::write(
            dir.path().join(".decree/inbox/D0001-1432-test-0.md"),
            content,
        )
        .unwrap();

        let config = AppConfig {
            max_attempts: 3,
            ..AppConfig::load_from_project(dir.path()).unwrap()
        };
        let shutdown = AtomicBool::new(false);

        let result =
            process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);
        assert!(result.is_err());

        // Should have 3 log files (one per attempt)
        let run_dir = dir.path().join(".decree/runs/D0001-1432-test-0");
        assert!(run_dir.join("routine.log").exists());
        assert!(run_dir.join("routine-2.log").exists());
        assert!(run_dir.join("routine-3.log").exists());
    }

    #[test]
    fn test_process_marks_migration_processed() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\necho 'done'\n",
        )
        .unwrap();

        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\nroutine: develop\nmigration: 01-auth.md\ntrigger: inbox\n---\nAdd auth.\n";
        std::fs::write(
            dir.path().join(".decree/inbox/D0001-1432-test-0.md"),
            content,
        )
        .unwrap();

        let config = AppConfig::load_from_project(dir.path()).unwrap();
        let shutdown = AtomicBool::new(false);

        process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown).unwrap();

        let processed = std::fs::read_to_string(dir.path().join(".decree/processed.md")).unwrap();
        assert!(processed.contains("01-auth.md"));
    }

    #[test]
    fn test_per_routine_max_attempts() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\nexit 1\n",
        )
        .unwrap();

        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\nroutine: develop\ntrigger: inbox\n---\nTest.\n";
        std::fs::write(
            dir.path().join(".decree/inbox/D0001-1432-test-0.md"),
            content,
        )
        .unwrap();

        let mut routines = std::collections::BTreeMap::new();
        routines.insert(
            "develop".to_string(),
            crate::config::RoutineEntry {
                enabled: true,
                deprecated: false,
                max_attempts: Some(1),
                timeout_s: None,
            },
        );

        let config = AppConfig {
            max_attempts: 5,
            routines: Some(routines),
            ..AppConfig::load_from_project(dir.path()).unwrap()
        };
        let shutdown = AtomicBool::new(false);

        process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown)
            .unwrap_err();

        // Only 1 log file (1 attempt, not 5)
        let run_dir = dir.path().join(".decree/runs/D0001-1432-test-0");
        assert!(run_dir.join("routine.log").exists());
        assert!(!run_dir.join("routine-2.log").exists());
    }

    #[test]
    fn test_fire_due_cron_jobs() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        // Write a cron file that fires every minute
        std::fs::write(
            dir.path().join(".decree/cron/every-minute.md"),
            "---\ncron: \"* * * * *\"\nroutine: develop\n---\nMinutely task.\n",
        )
        .unwrap();

        let mut tracker = CronTracker::new();
        fire_due_cron_jobs(dir.path(), &mut tracker);

        // Should have created an inbox message
        let inbox_files = message::list_inbox_messages(dir.path()).unwrap();
        assert_eq!(inbox_files.len(), 1);

        // Verify the inbox message content
        let content = std::fs::read_to_string(
            dir.path().join(".decree/inbox").join(&inbox_files[0]),
        )
        .unwrap();
        assert!(content.contains("routine: develop"));
        assert!(content.contains("Minutely task."));
        assert!(content.contains("trigger: cron:every-minute"));
        // cron field should NOT be present as a standalone YAML key
        assert!(!content.lines().any(|l| l.starts_with("cron:")));

        // Second fire within same minute should not create duplicate
        fire_due_cron_jobs(dir.path(), &mut tracker);
        let inbox_files2 = message::list_inbox_messages(dir.path()).unwrap();
        assert_eq!(inbox_files2.len(), 1); // Still just one
    }

    #[test]
    fn test_value_as_env_string() {
        assert_eq!(
            value_as_env_string(&serde_yaml::Value::String("hello".into())),
            Some("hello".to_string())
        );
        assert_eq!(
            value_as_env_string(&serde_yaml::Value::Bool(true)),
            Some("true".to_string())
        );
        assert_eq!(value_as_env_string(&serde_yaml::Value::Null), None);
    }

    #[test]
    fn test_value_as_env_string_array() {
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            "- input_image: some_path.png [output]\n  output_prefix: some_prefix",
        )
        .unwrap();
        assert_eq!(
            value_as_env_string(&yaml),
            Some(
                r#"[{"input_image":"some_path.png [output]","output_prefix":"some_prefix"}]"#
                    .to_string()
            )
        );
    }

    #[test]
    fn test_value_as_env_string_mapping() {
        let yaml: serde_yaml::Value = serde_yaml::from_str("key: val").unwrap();
        assert_eq!(
            value_as_env_string(&yaml),
            Some(r#"{"key":"val"}"#.to_string())
        );
    }
}
