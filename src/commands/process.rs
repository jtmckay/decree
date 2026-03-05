use crate::config::{self, AppConfig};
use crate::error::{color, DecreeError, EXIT_PRECHECK};
use crate::hooks::{self, HookContext, HookType};
use crate::message::{self, InboxMessage};
use crate::routine;
use std::collections::BTreeMap;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// PID of the currently running child process (0 when no child is active).
/// Used by the SIGINT handler to forward the signal to the child process group.
static CHILD_PID: AtomicU32 = AtomicU32::new(0);

/// Run `decree process [--dry-run]`.
pub fn run(project_root: &Path, dry_run: bool) -> Result<(), DecreeError> {
    if dry_run {
        return run_dry(project_root);
    }

    let config = AppConfig::load_from_project(project_root)?;
    let shutdown = Arc::new(AtomicBool::new(false));
    register_signal_handlers(Arc::clone(&shutdown))?;

    let process_start = chrono::Local::now();

    // Step 1: Run beforeAll hook
    let all_ctx = HookContext::default();
    if let Err(e) = hooks::run_hook(project_root, &config.hooks, HookType::BeforeAll, &all_ctx) {
        eprintln!("{} hook failed: {e}", HookType::BeforeAll);
        return Err(DecreeError::Other(format!("beforeAll hook failed: {e}")));
    }

    let mut migrations_processed = 0u32;

    // Step 2-6: Process migrations one at a time, draining inbox after each
    loop {
        if shutdown.load(Ordering::Relaxed) {
            exit_sigint();
        }

        let unprocessed = message::unprocessed_migrations(project_root)?;
        if unprocessed.is_empty() {
            // No more migrations — drain any remaining inbox messages
            drain_inbox(project_root, &config, &shutdown, None)?;
            break;
        }

        let migration_filename = &unprocessed[0];
        migrations_processed += 1;

        let total = unprocessed.len() as u32 + migrations_processed - 1;
        let progress = format!(
            "[Migration {}/{}: {}]",
            migrations_processed, total, migration_filename
        );
        print_progress(&progress);

        // Read migration content
        let migration_path = project_root
            .join(config::DECREE_DIR)
            .join(config::MIGRATIONS_DIR)
            .join(migration_filename);
        let migration_content = std::fs::read_to_string(&migration_path)?;
        let migration = message::parse_migration(migration_filename, &migration_content)?;

        // Generate chain ID for this migration
        let now = chrono::Local::now();
        let hhmm = now.format("%H%M").to_string();
        let day = message::next_day_counter(project_root, &hhmm)?;
        let name = migration_filename.trim_end_matches(".md");
        let chain = message::build_chain_id(&day, &hhmm, name);

        // Create inbox message with migration content as body
        let seq = 0u32;
        let full_id = format!("{chain}-{seq}");
        let filename = format!("{full_id}.md");

        let msg = InboxMessage {
            id: Some(full_id),
            chain: Some(chain.clone()),
            seq: Some(seq),
            routine: migration.routine,
            migration: Some(migration_filename.clone()),
            body: migration_content,
            custom_fields: migration.custom_fields,
            filename,
        };

        let inbox_dir = project_root
            .join(config::DECREE_DIR)
            .join(config::INBOX_DIR);
        std::fs::create_dir_all(&inbox_dir)?;
        msg.write_to_inbox(project_root)?;

        // Drain inbox (process this message and any follow-ups)
        drain_inbox(project_root, &config, &shutdown, Some(&chain))?;
    }

    // Step 7: Run afterAll hook
    if let Err(e) = hooks::run_hook(project_root, &config.hooks, HookType::AfterAll, &all_ctx) {
        eprintln!("{}: afterAll hook failed: {e}", color::warning("warning"));
        // afterAll failure: log warning, exit with hook's code
        return Err(DecreeError::Other(format!("afterAll hook failed: {e}")));
    }

    // Step 8: Print total duration summary
    let process_end = chrono::Local::now();
    let duration = process_end.signed_duration_since(process_start);
    let duration_str = format_duration(duration);
    println!(
        "Processed {} migration{} in {}",
        migrations_processed,
        if migrations_processed == 1 { "" } else { "s" },
        duration_str
    );

    Ok(())
}

/// Drain the inbox: process all messages LIFO, depth-first within chains.
fn drain_inbox(
    project_root: &Path,
    config: &AppConfig,
    shutdown: &Arc<AtomicBool>,
    prefer_chain: Option<&str>,
) -> Result<(), DecreeError> {
    loop {
        if shutdown.load(Ordering::Relaxed) {
            exit_sigint();
        }

        let inbox = message::list_inbox_messages(project_root)?;
        if inbox.is_empty() {
            break;
        }

        // LIFO: newest first. Within same chain, depth-first (higher seq first).
        // If prefer_chain is set, prefer messages from that chain.
        let filename = select_next_message(&inbox, prefer_chain);

        match process_single_message(project_root, config, &filename, shutdown) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("{}: {e}", color::warning("warning"));
                // Safety: ensure message is removed from inbox to prevent infinite loop.
                // process_single_message should dead-letter on all failure paths, but
                // if it didn't (e.g. early parse/IO error), dead-letter here as fallback.
                let _ = dead_letter(project_root, &filename);
            }
        }
    }
    Ok(())
}

/// Select next message from inbox: prefer current chain (depth-first), then LIFO.
fn select_next_message(inbox: &[String], prefer_chain: Option<&str>) -> String {
    if let Some(chain) = prefer_chain {
        // Find messages from this chain, pick highest seq (depth-first)
        let chain_prefix = format!("{chain}-");
        let mut chain_msgs: Vec<&String> = inbox
            .iter()
            .filter(|f| f.starts_with(&chain_prefix))
            .collect();

        if !chain_msgs.is_empty() {
            // Sort by seq descending (depth-first)
            chain_msgs.sort_by(|a, b| {
                let seq_a = extract_seq(a);
                let seq_b = extract_seq(b);
                seq_b.cmp(&seq_a)
            });
            return chain_msgs[0].clone();
        }
    }

    // LIFO: last (alphabetically last = newest by naming convention)
    inbox.last().unwrap().clone()
}

/// Extract seq number from a filename like `D0001-1432-name-3.md`.
fn extract_seq(filename: &str) -> u32 {
    let stem = filename.strip_suffix(".md").unwrap_or(filename);
    if let Some(pos) = stem.rfind('-') {
        stem[pos + 1..].parse().unwrap_or(0)
    } else {
        0
    }
}

/// Process a single inbox message through the full pipeline.
fn process_single_message(
    project_root: &Path,
    config: &AppConfig,
    filename: &str,
    shutdown: &Arc<AtomicBool>,
) -> Result<(), DecreeError> {
    // Parse and normalize the message
    let mut msg = InboxMessage::from_file(project_root, filename)?;
    let was_modified = msg.normalize(project_root, config, None)?;

    if was_modified {
        msg.write_to_inbox(project_root)?;
    }

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

    // Create run directory
    let run_dir = project_root
        .join(config::DECREE_DIR)
        .join(config::RUNS_DIR)
        .join(&msg_id);
    std::fs::create_dir_all(&run_dir)?;

    // Copy normalized message to run dir
    std::fs::write(run_dir.join("message.md"), msg.serialize())?;

    // Find the routine script
    let routines_dir = project_root
        .join(config::DECREE_DIR)
        .join(config::ROUTINES_DIR);
    let script_path = match routine::find_routine_script(&routines_dir, &routine_name) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("routine not found for {msg_id}: {e}");
            mark_migration_processed_if_present(project_root, &msg)?;
            dead_letter(project_root, filename)?;
            return Err(e);
        }
    };

    let msg_file_path = project_root
        .join(config::DECREE_DIR)
        .join(config::INBOX_DIR)
        .join(filename);

    // Retry loop
    for attempt in 1..=config.max_retries {
        if shutdown.load(Ordering::Relaxed) {
            // Write end timestamp to current log before exiting
            let log_file = if attempt == 1 {
                "routine.log".to_string()
            } else {
                format!("routine-{attempt}.log")
            };
            let log_path = run_dir.join(&log_file);
            let end = chrono::Local::now();
            let _ = append_to_file(
                &log_path,
                &format!(
                    "[decree] duration 0s end {}\n",
                    end.format("%Y-%m-%dT%H:%M:%S")
                ),
            );
            exit_sigint();
        }

        // Build hook context
        let hook_ctx = HookContext {
            message_file: msg_file_path.to_string_lossy().to_string(),
            message_id: msg_id.clone(),
            message_dir: run_dir.to_string_lossy().to_string(),
            chain: chain.clone(),
            seq: seq.to_string(),
            attempt: Some(attempt),
            max_retries: Some(config.max_retries),
            routine_exit_code: None,
        };

        // Run beforeEach hook
        if let Err(e) =
            hooks::run_hook(project_root, &config.hooks, HookType::BeforeEach, &hook_ctx)
        {
            eprintln!("{}: beforeEach hook failed for {msg_id}: {e}", color::warning("warning"));
            // beforeEach failure: skip and dead-letter
            mark_migration_processed_if_present(project_root, &msg)?;
            dead_letter(project_root, filename)?;
            return Err(DecreeError::Other(format!("beforeEach failed: {e}")));
        }

        // Execute routine
        let log_file = if attempt == 1 {
            "routine.log".to_string()
        } else {
            format!("routine-{attempt}.log")
        };
        let log_path = run_dir.join(&log_file);

        let progress = format!("{msg_id} (attempt {attempt}/{}) via {routine_name}", config.max_retries);
        print_progress(&progress);

        let start = chrono::Local::now();
        let start_line = format!("[decree] start {}\n", start.format("%Y-%m-%dT%H:%M:%S"));
        std::fs::write(&log_path, &start_line)?;

        let exit_code = execute_routine(
            project_root,
            &script_path,
            &msg,
            &run_dir,
            &log_path,
            shutdown,
        )?;

        // Check for SIGINT after routine completes
        if shutdown.load(Ordering::Relaxed) {
            let end = chrono::Local::now();
            let duration = end.signed_duration_since(start);
            let end_line = format!(
                "[decree] duration {} end {}\n",
                format_duration(duration),
                end.format("%Y-%m-%dT%H:%M:%S")
            );
            let _ = append_to_file(&log_path, &end_line);
            exit_sigint();
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
                ..hook_ctx
            };
            if let Err(e) =
                hooks::run_hook(project_root, &config.hooks, HookType::AfterEach, &after_ctx)
            {
                eprintln!("{}: afterEach hook failed for {msg_id}: {e}", color::warning("warning"));
            }

            // Collect outbox
            collect_outbox(project_root, &chain, seq, config)?;

            // Delete message from inbox
            let inbox_path = project_root
                .join(config::DECREE_DIR)
                .join(config::INBOX_DIR)
                .join(filename);
            if inbox_path.exists() {
                std::fs::remove_file(&inbox_path)?;
            }

            // If message has migration field, mark as processed
            if let Some(ref migration) = msg.migration {
                message::mark_processed(project_root, migration)?;
            }

            return Ok(());
        }

        // FAILURE
        let after_ctx = HookContext {
            routine_exit_code: Some(exit_code),
            ..hook_ctx
        };
        if let Err(e) =
            hooks::run_hook(project_root, &config.hooks, HookType::AfterEach, &after_ctx)
        {
            eprintln!("{}: afterEach hook failed for {msg_id}: {e}", color::warning("warning"));
        }

        if attempt == config.max_retries {
            // EXHAUSTION
            eprintln!(
                "max retries exhausted for {msg_id} (exit code: {exit_code})"
            );

            // Clear outbox
            clear_outbox(project_root)?;

            // Mark migration as processed so it doesn't loop forever
            mark_migration_processed_if_present(project_root, &msg)?;

            // Dead-letter the message
            dead_letter(project_root, filename)?;

            return Err(DecreeError::MaxRetriesExhausted(msg_id));
        }
    }

    Ok(())
}

/// Execute a routine script and return its exit code.
fn execute_routine(
    project_root: &Path,
    script_path: &Path,
    msg: &InboxMessage,
    run_dir: &Path,
    log_path: &Path,
    shutdown: &Arc<AtomicBool>,
) -> Result<i32, DecreeError> {
    let msg_file_path = project_root
        .join(config::DECREE_DIR)
        .join(config::INBOX_DIR)
        .join(&msg.filename);

    let msg_id = msg.id.as_deref().unwrap_or("");
    let chain = msg.chain.as_deref().unwrap_or("");
    let seq = msg.seq.map(|s| s.to_string()).unwrap_or_default();

    // Execute: bash <script> 2>&1 | tee -a <log_path>
    let cmd_str = format!(
        "set -o pipefail; bash {} 2>&1 | tee -a {}",
        shell_escape(script_path.to_string_lossy().as_ref()),
        shell_escape(log_path.to_string_lossy().as_ref()),
    );

    let mut cmd = std::process::Command::new("bash");
    cmd.arg("-c")
        .arg(&cmd_str)
        .current_dir(project_root)
        .env_remove("CLAUDECODE")
        .env("message_file", msg_file_path.to_string_lossy().as_ref())
        .env("message_id", msg_id)
        .env("message_dir", run_dir.to_string_lossy().as_ref())
        .env("chain", chain)
        .env("seq", &seq);

    // Pass custom fields as env vars
    for (key, value) in &msg.custom_fields {
        if let Some(s) = value_as_env_string(value) {
            cmd.env(key, &s);
        }
    }

    // Put child in its own process group so we can kill the entire tree on SIGINT.
    cmd.process_group(0);

    // Ignore SIGTTIN/SIGTTOU in the child so the background process group
    // doesn't get stopped when writing to the terminal (tee) or if any
    // subprocess probes the TTY. The parent stays in the foreground group
    // so it receives Ctrl+C (SIGINT) and can kill the child group.
    unsafe {
        cmd.pre_exec(|| {
            libc::signal(libc::SIGTTIN, libc::SIG_IGN);
            libc::signal(libc::SIGTTOU, libc::SIG_IGN);
            Ok(())
        });
    }

    // Routines run unattended — no terminal input needed.
    cmd.stdin(std::process::Stdio::null());

    let mut child = cmd.spawn()?;
    let child_id = child.id();
    CHILD_PID.store(child_id, Ordering::SeqCst);

    // Poll for completion, checking for SIGINT between iterations.
    let exit_code = loop {
        match child.try_wait()? {
            Some(status) => break status.code().unwrap_or(1),
            None => {
                if shutdown.load(Ordering::SeqCst) {
                    // Kill the child's entire process group
                    unsafe {
                        libc::kill(-(child_id as i32), libc::SIGTERM);
                    }
                    let _ = child.wait();
                    CHILD_PID.store(0, Ordering::SeqCst);
                    // Return — caller checks shutdown flag and exits 130
                    return Ok(130);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    };

    CHILD_PID.store(0, Ordering::SeqCst);
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
            eprintln!(
                "{}: non-.md file in outbox ignored: {entry}",
                color::warning("Warning")
            );
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
                "{}: MaxDepthExceeded for outbox file {file} (seq={next_seq}, limit={})",
                color::warning("Warning"),
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
        let known: &[&str] = &["id", "chain", "seq", "routine", "migration"];
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

/// If the message originated from a migration, mark it as processed
/// so the outer migration loop doesn't retry it infinitely.
fn mark_migration_processed_if_present(
    project_root: &Path,
    msg: &InboxMessage,
) -> Result<(), DecreeError> {
    if let Some(ref migration) = msg.migration {
        message::mark_processed(project_root, migration)?;
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

/// Register SIGINT handler to set shutdown flag and forward to child process group.
fn register_signal_handlers(shutdown: Arc<AtomicBool>) -> Result<(), DecreeError> {
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&shutdown))?;

    // Forward SIGTERM to the child's process group so all subprocesses are killed.
    unsafe {
        signal_hook::low_level::register(signal_hook::consts::SIGINT, || {
            let pid = CHILD_PID.load(Ordering::SeqCst);
            if pid != 0 {
                // Kill the entire child process group (child is leader via process_group(0))
                libc::kill(-(pid as i32), libc::SIGTERM);
            }
        })?;
    }

    Ok(())
}

/// Exit immediately with code 130 (SIGINT).
fn exit_sigint() -> ! {
    std::process::exit(130)
}

/// Print a progress line.
fn print_progress(msg: &str) {
    if color::is_tty() {
        // TTY: print status line
        eprintln!("{}", color::dim(msg));
    } else {
        println!("{msg}");
    }
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

    let marker = format!(
        "[log truncated — showing last {} of output]\n",
        format_bytes(max_size)
    );
    let mut new_content = marker.into_bytes();
    new_content.extend_from_slice(truncated);

    std::fs::write(path, &new_content)?;

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
        _ => None,
    }
}

/// `decree process --dry-run`: list migrations, resolve routines, run pre-checks.
fn run_dry(project_root: &Path) -> Result<(), DecreeError> {
    let config = AppConfig::load_from_project(project_root)?;
    let unprocessed = message::unprocessed_migrations(project_root)?;

    if unprocessed.is_empty() {
        println!("No unprocessed migrations.");
        return Ok(());
    }

    println!();
    println!("Dry run — no messages will be created:");

    let mut failures = 0u32;
    let total = unprocessed.len();

    for filename in &unprocessed {
        // Read migration to check for routine frontmatter
        let migration_path = project_root
            .join(config::DECREE_DIR)
            .join(config::MIGRATIONS_DIR)
            .join(filename);
        let content = std::fs::read_to_string(&migration_path)?;
        let migration = message::parse_migration(filename, &content)?;

        let routine_name = migration
            .routine
            .as_deref()
            .unwrap_or(&config.default_routine);

        // Run pre-check
        let result = routine::run_precheck(project_root, routine_name);
        match result {
            Ok(None) => {
                println!(
                    "  {:<24} → {:<16} {}",
                    filename,
                    routine_name,
                    color::success("PASS")
                );
            }
            Ok(Some(reason)) => {
                println!(
                    "  {:<24} → {:<16} {}: {}",
                    filename,
                    routine_name,
                    color::error("FAIL"),
                    reason
                );
                failures += 1;
            }
            Err(_) => {
                println!(
                    "  {:<24} → {:<16} {}: routine not found",
                    filename,
                    routine_name,
                    color::error("FAIL"),
                );
                failures += 1;
            }
        }
    }

    if failures > 0 {
        println!();
        println!("Pre-check failures: {} of {}", failures, total);
        std::process::exit(EXIT_PRECHECK);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_decree_dir(dir: &TempDir) {
        let decree = dir.path().join(".decree");
        std::fs::create_dir_all(decree.join("inbox")).unwrap();
        std::fs::create_dir_all(decree.join("inbox/dead")).unwrap();
        std::fs::create_dir_all(decree.join("outbox")).unwrap();
        std::fs::create_dir_all(decree.join("outbox/dead")).unwrap();
        std::fs::create_dir_all(decree.join("runs")).unwrap();
        std::fs::create_dir_all(decree.join("routines")).unwrap();
        std::fs::create_dir_all(decree.join("migrations")).unwrap();
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

        let inbox = dir.path().join(".decree/inbox");
        assert!(inbox.join("D0001-1432-test-1.md").exists());
        assert!(!outbox.join("followup.md").exists());

        let content = std::fs::read_to_string(inbox.join("D0001-1432-test-1.md")).unwrap();
        assert!(content.contains("chain: D0001-1432-test"));
        assert!(content.contains("seq: 1"));
        assert!(content.contains("routine: develop"));
        assert!(content.contains("Follow-up task."));
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

        collect_outbox(dir.path(), "D0001-1432-test", 2, &config).unwrap();

        let inbox = dir.path().join(".decree/inbox");
        assert!(!inbox.join("D0001-1432-test-3.md").exists());
        assert!(outbox.join("dead/followup.md").exists());
    }

    #[test]
    fn test_truncate_log_disabled() {
        let dir = TempDir::new().unwrap();
        let log = dir.path().join("test.log");
        std::fs::write(&log, "a".repeat(5000)).unwrap();

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
        assert!(result.ends_with(&"x".repeat(100)));
    }

    #[test]
    fn test_process_single_message_success() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\necho 'done'\n",
        )
        .unwrap();

        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\nroutine: develop\n---\nTest body.\n";
        std::fs::write(
            dir.path().join(".decree/inbox/D0001-1432-test-0.md"),
            content,
        )
        .unwrap();

        let config = AppConfig::load_from_project(dir.path()).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));

        let result =
            process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);
        assert!(result.is_ok());

        assert!(!dir
            .path()
            .join(".decree/inbox/D0001-1432-test-0.md")
            .exists());
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
    fn test_process_single_message_failure_dead_letters() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\nexit 1\n",
        )
        .unwrap();

        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\nroutine: develop\n---\nTest.\n";
        std::fs::write(
            dir.path().join(".decree/inbox/D0001-1432-test-0.md"),
            content,
        )
        .unwrap();

        let config = AppConfig {
            max_retries: 1,
            ..AppConfig::load_from_project(dir.path()).unwrap()
        };
        let shutdown = Arc::new(AtomicBool::new(false));

        let result =
            process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);
        assert!(result.is_err());

        assert!(dir
            .path()
            .join(".decree/inbox/dead/D0001-1432-test-0.md")
            .exists());
    }

    #[test]
    fn test_process_single_message_retries() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\nexit 1\n",
        )
        .unwrap();

        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\nroutine: develop\n---\nTest.\n";
        std::fs::write(
            dir.path().join(".decree/inbox/D0001-1432-test-0.md"),
            content,
        )
        .unwrap();

        let config = AppConfig {
            max_retries: 3,
            ..AppConfig::load_from_project(dir.path()).unwrap()
        };
        let shutdown = Arc::new(AtomicBool::new(false));

        let result =
            process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);
        assert!(result.is_err());

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

        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\nroutine: develop\nmigration: 01-auth.md\n---\nAdd auth.\n";
        std::fs::write(
            dir.path().join(".decree/inbox/D0001-1432-test-0.md"),
            content,
        )
        .unwrap();

        let config = AppConfig::load_from_project(dir.path()).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));

        process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown).unwrap();

        let processed = std::fs::read_to_string(dir.path().join(".decree/processed.md")).unwrap();
        assert!(processed.contains("01-auth.md"));
    }

    #[test]
    fn test_select_next_message_lifo() {
        let inbox = vec![
            "D0001-1432-alpha-0.md".to_string(),
            "D0001-1432-beta-0.md".to_string(),
            "D0001-1432-gamma-0.md".to_string(),
        ];
        let result = select_next_message(&inbox, None);
        assert_eq!(result, "D0001-1432-gamma-0.md");
    }

    #[test]
    fn test_select_next_message_prefer_chain() {
        let inbox = vec![
            "D0001-1432-alpha-0.md".to_string(),
            "D0001-1432-alpha-1.md".to_string(),
            "D0001-1432-beta-0.md".to_string(),
        ];
        let result = select_next_message(&inbox, Some("D0001-1432-alpha"));
        // Should pick alpha-1 (highest seq in preferred chain)
        assert_eq!(result, "D0001-1432-alpha-1.md");
    }

    #[test]
    fn test_select_next_message_prefer_chain_not_found() {
        let inbox = vec![
            "D0001-1432-alpha-0.md".to_string(),
            "D0001-1432-beta-0.md".to_string(),
        ];
        let result = select_next_message(&inbox, Some("D0001-1432-gamma"));
        // No gamma messages, fall back to LIFO
        assert_eq!(result, "D0001-1432-beta-0.md");
    }

    #[test]
    fn test_extract_seq() {
        assert_eq!(extract_seq("D0001-1432-test-0.md"), 0);
        assert_eq!(extract_seq("D0001-1432-test-3.md"), 3);
        assert_eq!(extract_seq("D0001-1432-01-add-auth-1.md"), 1);
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
    fn test_dry_run_no_migrations() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        // No migrations dir content
        let result = run_dry(dir.path());
        assert!(result.is_ok());
    }
}
