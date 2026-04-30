use crate::commands::routine_sync;
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

    let mut config = AppConfig::load_from_project(project_root)?;

    // Run discovery before processing
    if routine_sync::discover(project_root, &mut config, None)? {
        config.save(project_root)?;
    }

    let shutdown = Arc::new(AtomicBool::new(false));
    register_signal_handlers(Arc::clone(&shutdown))?;

    let process_start = chrono::Local::now();

    // Step 1: Run beforeAll hook
    let all_ctx = HookContext::default();
    match hooks::run_hook_with_config(project_root, &config.hooks, HookType::BeforeAll, &all_ctx, Some(&config)) {
        Ok(hook_output) => {
            if !hook_output.is_empty() {
                eprintln!("{}", hook_output.output);
            }
        }
        Err(e) => {
            if !e.output.is_empty() {
                eprintln!("{}", e.output);
            }
            eprintln!("{} hook failed: {e}", HookType::BeforeAll);
            return Err(DecreeError::Other(format!("beforeAll hook failed: {e}")));
        }
    }

    let mut migrations_processed = 0u32;

    // Step 2-6: Process migrations one at a time, draining inbox after each
    loop {
        if shutdown.load(Ordering::Relaxed) {
            exit_sigint();
        }

        let unprocessed = message::unprocessed_migrations(project_root)?;
        if unprocessed.is_empty() {
            // No more migrations — drain any remaining inbox messages.
            // Dead-letters here do not stop the loop (inbox-only drain).
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
            trigger: Some("inbox".to_string()),
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
        let drain_result = drain_inbox(project_root, &config, &shutdown, Some(&chain))?;
        if drain_result.dead_lettered {
            eprintln!(
                "[Migration {}/{}: {}] FAILED — stopping. Fix the migration or dead-letter it manually, then re-run `decree process`.",
                migrations_processed, total, migration_filename
            );
            return Err(DecreeError::Other(format!(
                "migration {} failed and was dead-lettered",
                migration_filename
            )));
        }
    }

    // Step 7: Run afterAll hook
    match hooks::run_hook_with_config(project_root, &config.hooks, HookType::AfterAll, &all_ctx, Some(&config)) {
        Ok(hook_output) => {
            if !hook_output.is_empty() {
                eprintln!("{}", hook_output.output);
            }
        }
        Err(e) => {
            if !e.output.is_empty() {
                eprintln!("{}", e.output);
            }
            eprintln!("{}: afterAll hook failed: {e}", color::warning("warning"));
            return Err(DecreeError::Other(format!("afterAll hook failed: {e}")));
        }
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

/// Result returned from `drain_inbox`.
pub struct DrainResult {
    /// True if at least one message was moved to `inbox/dead/` during this drain.
    pub dead_lettered: bool,
}

/// Drain the inbox: process all messages LIFO, depth-first within chains.
fn drain_inbox(
    project_root: &Path,
    config: &AppConfig,
    shutdown: &Arc<AtomicBool>,
    prefer_chain: Option<&str>,
) -> Result<DrainResult, DecreeError> {
    let mut result = DrainResult { dead_lettered: false };
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
                result.dead_lettered = true;
            }
        }
    }
    Ok(result)
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
///
/// This handles normalization, routine resolution, the retry loop with
/// beforeEach/afterEach hooks, outbox collection, and dead-lettering.
/// It does NOT run beforeAll/afterAll hooks or drain the inbox.
pub fn process_single_message(
    project_root: &Path,
    config: &AppConfig,
    filename: &str,
    shutdown: &Arc<AtomicBool>,
) -> Result<(), DecreeError> {
    // Parse and normalize the message
    let mut msg = InboxMessage::from_file(project_root, filename)?;

    // Build the AI router callback if configured
    let ai_router_cmd = config.commands.ai_router.clone();
    let ai_router_fn: Option<Box<dyn Fn(&str) -> Result<String, DecreeError>>> =
        if ai_router_cmd.is_empty() {
            None
        } else {
            Some(Box::new(move |prompt: &str| {
                invoke_ai_router(&ai_router_cmd, prompt)
            }))
        };
    let ai_router_ref = ai_router_fn
        .as_ref()
        .map(|f| f.as_ref() as &dyn Fn(&str) -> Result<String, DecreeError>);
    let was_modified = msg.normalize(project_root, config, ai_router_ref)?;

    // After normalization, rename the inbox file if the ID-based name differs from the original.
    let active_filename: String = if was_modified {
        let new_filename = format!(
            "{}.md",
            msg.id.as_deref().unwrap_or(filename.strip_suffix(".md").unwrap_or(filename))
        );
        if new_filename != filename {
            let old_path = project_root
                .join(config::DECREE_DIR)
                .join(config::INBOX_DIR)
                .join(filename);
            let new_path = project_root
                .join(config::DECREE_DIR)
                .join(config::INBOX_DIR)
                .join(&new_filename);
            std::fs::rename(&old_path, &new_path)?;
            msg.filename = new_filename.clone();
            // Write updated content to the renamed file
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

    // Determine effective max_retries (per-routine override or global)
    let effective_max_retries = config
        .routines
        .as_ref()
        .and_then(|r| r.get(&routine_name))
        .and_then(|e| e.max_retries)
        .unwrap_or(config.max_retries);

    // Determine per-routine timeout_s
    let timeout_s = config
        .routines
        .as_ref()
        .and_then(|r| r.get(&routine_name))
        .and_then(|e| e.timeout_s);

    // Consume any pending session ID from a prior token-exhaustion wait.
    // Only used for the first attempt of this message (attempt == 1).
    let token_session_path = project_root
        .join(config::DECREE_DIR)
        .join("token_session.txt");
    let previous_session_id: Option<String> = if token_session_path.exists() {
        let id = std::fs::read_to_string(&token_session_path).unwrap_or_default();
        let _ = std::fs::remove_file(&token_session_path);
        if id.trim().is_empty() { None } else { Some(id.trim().to_string()) }
    } else {
        None
    };

    // Create run directory
    let run_dir = project_root
        .join(config::DECREE_DIR)
        .join(config::RUNS_DIR)
        .join(&msg_id);
    std::fs::create_dir_all(&run_dir)?;

    // Copy normalized message to run dir
    std::fs::write(run_dir.join("message.md"), msg.serialize())?;

    // Find the routine script (registry-aware layered lookup)
    let script_path = match routine::resolve_routine(project_root, config, &routine_name) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("routine resolution failed for {msg_id}: {e}");
            mark_migration_processed_if_present(project_root, &msg)?;
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
    for attempt in 1..=effective_max_retries {
        total_attempts = attempt;

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

        let is_final = attempt == effective_max_retries;

        // Build hook context
        let hook_ctx = HookContext {
            message_file: msg_file_path.to_string_lossy().to_string(),
            message_id: msg_id.clone(),
            message_dir: run_dir.to_string_lossy().to_string(),
            chain: chain.clone(),
            seq: seq.to_string(),
            attempt: Some(attempt),
            max_retries: Some(effective_max_retries),
            routine_exit_code: None,
            final_attempt: false,
            trigger: trigger.clone(),
        };

        // Initialize log file for this attempt
        let log_file = if attempt == 1 {
            "routine.log".to_string()
        } else {
            format!("routine-{attempt}.log")
        };
        let log_path = run_dir.join(&log_file);

        let start = chrono::Local::now();
        let start_line = format!("[decree] start {}\n", start.format("%Y-%m-%dT%H:%M:%S"));
        std::fs::write(&log_path, &start_line)?;

        // Run beforeEach hook
        match hooks::run_hook_with_config(project_root, &config.hooks, HookType::BeforeEach, &hook_ctx, Some(config)) {
            Ok(hook_output) => {
                write_hook_log(&log_path, HookType::BeforeEach, &hook_output.output)?;
            }
            Err(e) => {
                write_hook_log(&log_path, HookType::BeforeEach, &e.output)?;
                eprintln!("{}: beforeEach hook failed for {msg_id}: {e}", color::warning("warning"));
                // beforeEach failure: skip and dead-letter (onDeadLetter does NOT fire here)
                mark_migration_processed_if_present(project_root, &msg)?;
                dead_letter(project_root, &active_filename)?;
                return Err(DecreeError::Other(format!("beforeEach failed: {e}")));
            }
        }

        // Execute routine
        let progress = format!("{msg_id} (attempt {attempt}/{effective_max_retries}) via {routine_name}");
        print_progress(&progress);

        let session_id_for_attempt = if attempt == 1 { previous_session_id.as_deref() } else { None };
        let exit_code = execute_routine(
            project_root,
            &script_path,
            &msg,
            &run_dir,
            &log_path,
            &trigger,
            timeout_s,
            session_id_for_attempt,
            shutdown,
        )?;
        last_exit_code = exit_code;

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

        // Extract and persist session ID from this attempt's log
        let log_content_for_session = std::fs::read_to_string(&log_path).unwrap_or_default();
        if let Some(sid) = extract_session_id(&log_content_for_session) {
            let _ = std::fs::write(run_dir.join("session_id.txt"), &sid);
        }

        if exit_code == 0 {
            // SUCCESS
            let after_ctx = HookContext {
                routine_exit_code: Some(0),
                final_attempt: is_final,
                ..hook_ctx.clone()
            };
            match hooks::run_hook_with_config(project_root, &config.hooks, HookType::AfterEach, &after_ctx, Some(config)) {
                Ok(hook_output) => {
                    let _ = write_hook_log(&log_path, HookType::AfterEach, &hook_output.output);
                }
                Err(e) => {
                    let _ = write_hook_log(&log_path, HookType::AfterEach, &e.output);
                    eprintln!("{}: afterEach hook failed for {msg_id}: {e}", color::warning("warning"));
                }
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
        match hooks::run_hook_with_config(project_root, &config.hooks, HookType::AfterEach, &after_ctx, Some(config)) {
            Ok(hook_output) => {
                let _ = write_hook_log(&log_path, HookType::AfterEach, &hook_output.output);
            }
            Err(e) => {
                let _ = write_hook_log(&log_path, HookType::AfterEach, &e.output);
                eprintln!("{}: afterEach hook failed for {msg_id}: {e}", color::warning("warning"));
            }
        }

        // Check for Claude token exhaustion before retrying or dead-lettering.
        // Detect on any failed attempt so we don't burn retries on an exhausted token.
        let log_content = std::fs::read_to_string(&log_path).unwrap_or_default();
        if detect_token_exhaustion(&log_content) {
            let reset_at = extract_reset_time(&log_content);
            let migration_name = msg.migration.as_deref().unwrap_or("unknown");
            wait_for_token_reset(reset_at, migration_name, shutdown);
            clear_outbox(project_root)?;
            // Ensure migration is not marked processed so the outer loop retries it.
            if let Some(ref migration) = msg.migration {
                let _ = message::unmark_processed(project_root, migration);
            }
            // Remove the inbox message so drain_inbox's inner loop exits cleanly.
            let inbox_path = project_root
                .join(config::DECREE_DIR)
                .join(config::INBOX_DIR)
                .join(&active_filename);
            if inbox_path.exists() {
                std::fs::remove_file(&inbox_path)?;
            }
            // Remove any stale dead-lettered copy from a prior run.
            let dead_path = project_root
                .join(config::DECREE_DIR)
                .join(config::INBOX_DIR)
                .join(config::DEAD_DIR)
                .join(&active_filename);
            if dead_path.exists() {
                std::fs::remove_file(&dead_path)?;
            }
            // Propagate session ID to the next attempt via a well-known file.
            let session_id_path = run_dir.join("session_id.txt");
            if session_id_path.exists() {
                let token_session_path = project_root
                    .join(config::DECREE_DIR)
                    .join("token_session.txt");
                let _ = std::fs::copy(&session_id_path, &token_session_path);
            }
            // Return Ok so DrainResult.dead_lettered is NOT set.
            return Ok(());
        }

        if attempt == effective_max_retries {
            // EXHAUSTION
            eprintln!(
                "max retries exhausted for {msg_id} (exit code: {exit_code})"
            );

            // Clear outbox
            clear_outbox(project_root)?;

            // Mark migration as processed so it doesn't loop forever
            mark_migration_processed_if_present(project_root, &msg)?;

            // Dead-letter the message
            dead_letter(project_root, &active_filename)?;

            // Write run.json before returning error
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
                attempt: Some(effective_max_retries),
                max_retries: Some(effective_max_retries),
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
                eprintln!("{}: onDeadLetter hook failed for {msg_id}: {e}", color::warning("warning"));
            }

            return Err(DecreeError::MaxRetriesExhausted(msg_id));
        }
    }

    // Write run.json for unexpected loop exit (shouldn't reach here normally)
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
    previous_session_id: Option<&str>,
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
        .env("seq", &seq)
        .env("DECREE_TRIGGER", trigger);

    // Pass custom fields as env vars
    for (key, value) in &msg.custom_fields {
        if let Some(s) = value_as_env_string(value) {
            cmd.env(key, &s);
        }
    }

    if let Some(id) = previous_session_id {
        cmd.env("DECREE_PREVIOUS_SESSION_ID", id);
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

    let start_time = std::time::Instant::now();

    // Poll for completion, checking for SIGINT and timeout between iterations.
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
                if let Some(t) = timeout_s {
                    if start_time.elapsed() >= std::time::Duration::from_secs(t as u64) {
                        unsafe {
                            libc::kill(-(child_id as i32), libc::SIGTERM);
                        }
                        let _ = child.wait();
                        CHILD_PID.store(0, Ordering::SeqCst);
                        return Ok(1);
                    }
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

/// Invoke the AI router command with the given prompt.
///
/// The router command template uses `{prompt}` as a placeholder for the actual prompt.
/// Falls back to passing the prompt as a trailing argument if no placeholder is found.
fn invoke_ai_router(cmd_template: &str, prompt: &str) -> Result<String, DecreeError> {
    let cmd_str = if cmd_template.contains("{prompt}") {
        cmd_template.replace("{prompt}", &shell_escape(prompt))
    } else {
        format!("{} {}", cmd_template, shell_escape(prompt))
    };

    let output = std::process::Command::new("bash")
        .arg("-c")
        .arg(&cmd_str)
        .output()
        .map_err(|e| DecreeError::Other(format!("failed to run AI router: {e}")))?;

    if !output.status.success() {
        return Err(DecreeError::Other(format!(
            "AI router exited with code {}",
            output.status.code().unwrap_or(1)
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Write hook output to a log file.
fn write_hook_log(
    log_path: &Path,
    hook_type: HookType,
    output: &str,
) -> Result<(), DecreeError> {
    if output.is_empty() {
        return Ok(());
    }

    let now = chrono::Local::now();
    let timestamp = now.format("%Y-%m-%dT%H:%M:%S").to_string();

    let block = format!(
        "[decree] hook {} start {}\n{}\n[decree] hook {} end {}\n",
        hook_type, timestamp, output, hook_type, timestamp,
    );

    append_to_file(log_path, &block)
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
        serde_yaml::Value::Null => None,
        serde_yaml::Value::Sequence(_) | serde_yaml::Value::Mapping(_) => {
            let json_val: serde_json::Value = serde_json::to_value(v).ok()?;
            Some(json_val.to_string())
        }
        _ => None,
    }
}

/// Return true when the log output matches the Claude token-exhaustion pattern.
///
/// Requires (case-insensitive) both "usage limit" and "reset" to be present.
fn detect_token_exhaustion(log_content: &str) -> bool {
    let lower = log_content.to_lowercase();
    lower.contains("usage limit") && lower.contains("reset")
}

/// Extract the Claude session ID from a log, if present.
///
/// Scans for the pattern (case-insensitive):
///   [Ss]ession(?:\s+[Ii][Dd])?:\s*([a-zA-Z0-9_-]+)
/// Returns the captured ID, or `None` if not found.
fn extract_session_id(log_content: &str) -> Option<String> {
    for line in log_content.lines() {
        let lower = line.to_lowercase();
        let pos = if let Some(p) = lower.find("session id:") {
            p + "session id:".len()
        } else if let Some(p) = lower.find("session:") {
            p + "session:".len()
        } else {
            continue;
        };
        let rest = line[pos..].trim_start();
        let id: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        if !id.is_empty() {
            return Some(id);
        }
    }
    None
}

/// Parse the reset time from a log that contains the token-exhaustion pattern.
///
/// Looks for the sequence "limit(s) reset(s) [at] H:MM AM/PM" and converts it
/// to a local `DateTime`. If the parsed time is in the past (already elapsed
/// today), adds 24 hours so the wait targets the next occurrence. Returns
/// `None` if no parseable time is found.
fn extract_reset_time(log_content: &str) -> Option<chrono::DateTime<chrono::Local>> {
    use chrono::{Local, NaiveTime, TimeZone};

    let lower = log_content.to_lowercase();

    // Find "limit" (or "limits") then the first "reset" after it.
    let limit_pos = lower.find("limit")?;
    let reset_offset = lower[limit_pos..].find("reset")?;
    // Advance cursor to the character after "reset" (5 bytes).
    let cursor = limit_pos + reset_offset + 5;
    if cursor >= lower.len() {
        return None;
    }
    let after = &lower[cursor..std::cmp::min(cursor + 80, lower.len())];

    let mut pos = 0;

    // Optional trailing "s" for "resets".
    if after.as_bytes().first() == Some(&b's') {
        pos += 1;
    }
    // Skip whitespace.
    while pos < after.len() && after.as_bytes()[pos] == b' ' {
        pos += 1;
    }
    // Skip optional "at ".
    if after[pos..].starts_with("at ") {
        pos += 3;
    }
    // Skip whitespace.
    while pos < after.len() && after.as_bytes()[pos] == b' ' {
        pos += 1;
    }

    // Parse H:MM or HH:MM.
    let time_str = &after[pos..];
    let colon_pos = time_str.find(':')?;
    let hour_str = time_str[..colon_pos].trim();
    if hour_str.is_empty() || hour_str.len() > 2 {
        return None;
    }
    let hour: u32 = hour_str.parse().ok()?;

    let after_colon = &time_str[colon_pos + 1..];
    if after_colon.len() < 2
        || !after_colon.as_bytes()[0].is_ascii_digit()
        || !after_colon.as_bytes()[1].is_ascii_digit()
    {
        return None;
    }
    let min: u32 = after_colon[..2].parse().ok()?;

    // Skip optional whitespace before AM/PM.
    let mut j = 2;
    while j < after_colon.len() && after_colon.as_bytes()[j] == b' ' {
        j += 1;
    }

    let is_pm = if after_colon[j..].starts_with("pm") {
        true
    } else if after_colon[j..].starts_with("am") {
        false
    } else {
        return None;
    };

    if hour > 12 || min > 59 {
        return None;
    }

    let hour24 = match (is_pm, hour) {
        (true, 12) => 12,
        (true, h) => h + 12,
        (false, 12) => 0,
        (false, h) => h,
    };

    let t = NaiveTime::from_hms_opt(hour24, min, 0)?;
    let now = Local::now();
    let today_reset = now.date_naive().and_time(t);
    let reset_dt = Local.from_local_datetime(&today_reset).single()?;

    let reset_dt = if reset_dt <= now {
        reset_dt + chrono::Duration::hours(24)
    } else {
        reset_dt
    };

    Some(reset_dt)
}

/// Print a waiting message then sleep until `reset_at` (or 1 hour from now
/// if `reset_at` is `None`). Polls the shutdown flag every 100 ms; exits
/// with code 130 if SIGINT is received while waiting.
fn wait_for_token_reset(
    reset_at: Option<chrono::DateTime<chrono::Local>>,
    migration: &str,
    shutdown: &Arc<AtomicBool>,
) {
    // Skip the actual sleep in unit tests — the behaviour under test is
    // detection and cleanup, not the sleep duration.
    #[cfg(test)]
    {
        let _ = (reset_at, shutdown);
        eprintln!("[Claude token limit] Usage limit reached (test mode). Retrying migration: {migration}");
        return;
    }

    #[cfg(not(test))]
    {
        let now = chrono::Local::now();
        let target = reset_at.unwrap_or_else(|| now + chrono::Duration::hours(1));
        let wait_secs = (target - now).num_seconds().max(0) as u64;
        let mins = wait_secs / 60;
        let secs = wait_secs % 60;

        eprintln!(
            "[Claude token limit] Usage limit reached. Waiting until {} ({}m {}s) to retry.",
            target.format("%H:%M"),
            mins,
            secs,
        );

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(wait_secs);
        loop {
            if shutdown.load(Ordering::Relaxed) {
                exit_sigint();
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            std::thread::sleep(remaining.min(std::time::Duration::from_millis(100)));
        }

        eprintln!("[Claude token limit] Retrying migration: {migration}");
    }
}

/// `decree process --dry-run`: list migrations, resolve routines, run pre-checks.
fn run_dry(project_root: &Path) -> Result<(), DecreeError> {
    let mut config = AppConfig::load_from_project(project_root)?;

    // Run discovery for dry-run too
    if routine_sync::discover(project_root, &mut config, None)? {
        config.save(project_root)?;
    }

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
        let result = routine::run_precheck(project_root, &config, routine_name);
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
        assert!(content.contains("trigger: chain"));
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
        assert!(dir
            .path()
            .join(".decree/runs/D0001-1432-test-0/run.json")
            .exists());
    }

    #[test]
    fn test_run_json_written_on_success() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\necho 'done'\n",
        )
        .unwrap();

        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\nroutine: develop\nmigration: 01-auth.md\n---\nTest.\n";
        std::fs::write(
            dir.path().join(".decree/inbox/D0001-1432-test-0.md"),
            content,
        )
        .unwrap();

        let config = AppConfig::load_from_project(dir.path()).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));

        process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown).unwrap();

        let json_path = dir.path().join(".decree/runs/D0001-1432-test-0/run.json");
        assert!(json_path.exists());

        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&json_path).unwrap()).unwrap();
        assert_eq!(json["message_id"], "D0001-1432-test-0");
        assert_eq!(json["routine"], "develop");
        assert_eq!(json["exit_code"], 0);
        assert_eq!(json["attempts"], 1);
        assert_eq!(json["migration"], "01-auth.md");
        assert!(json["trigger"].is_string());
    }

    #[test]
    fn test_run_json_written_on_failure() {
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
            max_retries: 2,
            ..AppConfig::load_from_project(dir.path()).unwrap()
        };
        let shutdown = Arc::new(AtomicBool::new(false));

        let _ = process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);

        let json_path = dir.path().join(".decree/runs/D0001-1432-test-0/run.json");
        assert!(json_path.exists());

        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&json_path).unwrap()).unwrap();
        assert_eq!(json["attempts"], 2);
        assert_eq!(json["exit_code"], 1);
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
    fn test_per_routine_max_retries() {
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

        // Global max_retries=1, but per-routine max_retries=3
        let mut routines = std::collections::BTreeMap::new();
        routines.insert(
            "develop".to_string(),
            crate::config::RoutineEntry {
                enabled: true,
                deprecated: false,
                max_retries: Some(3),
                timeout_s: None,
            },
        );
        let config = AppConfig {
            max_retries: 1,
            routines: Some(routines),
            ..AppConfig::load_from_project(dir.path()).unwrap()
        };
        let shutdown = Arc::new(AtomicBool::new(false));

        let _ = process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);

        // Should have 3 log files (per-routine max_retries=3)
        let run_dir = dir.path().join(".decree/runs/D0001-1432-test-0");
        assert!(run_dir.join("routine.log").exists());
        assert!(run_dir.join("routine-2.log").exists());
        assert!(run_dir.join("routine-3.log").exists());
        // Should NOT have a 4th attempt
        assert!(!run_dir.join("routine-4.log").exists());
    }

    #[test]
    fn test_per_routine_timeout_kills_process() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        // Routine that sleeps for 10 seconds
        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\nsleep 10\n",
        )
        .unwrap();

        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\nroutine: develop\n---\nTest.\n";
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
                max_retries: None,
                timeout_s: Some(1), // 1 second timeout
            },
        );
        let config = AppConfig {
            max_retries: 1,
            routines: Some(routines),
            ..AppConfig::load_from_project(dir.path()).unwrap()
        };
        let shutdown = Arc::new(AtomicBool::new(false));

        let start = std::time::Instant::now();
        let result = process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);
        let elapsed = start.elapsed();

        // Should have timed out and dead-lettered
        assert!(result.is_err());
        // Should have completed in well under 10 seconds
        assert!(elapsed.as_secs() < 5, "timeout didn't work: took {}s", elapsed.as_secs());
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
    fn test_inbox_file_renamed_after_normalize() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\necho 'done'\n",
        )
        .unwrap();

        // Drop a bare file with no frontmatter — gets renamed during normalize
        std::fs::write(
            dir.path().join(".decree/inbox/fix-errors.md"),
            "Fix the errors.\n",
        )
        .unwrap();

        let config = AppConfig::load_from_project(dir.path()).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));

        let result = process_single_message(dir.path(), &config, "fix-errors.md", &shutdown);
        assert!(result.is_ok());

        // Original filename should no longer exist
        assert!(!dir.path().join(".decree/inbox/fix-errors.md").exists());
        // A run directory should have been created (named with the new ID)
        let runs: Vec<_> = std::fs::read_dir(dir.path().join(".decree/runs"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .collect();
        assert!(!runs.is_empty());
        // Run dir name should contain "fix-errors"
        let run_name = runs[0].file_name().to_string_lossy().to_string();
        assert!(run_name.contains("fix-errors"), "unexpected run name: {run_name}");
    }

    #[test]
    fn test_on_dead_letter_hook_fires_on_exhaustion() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\nexit 1\n",
        )
        .unwrap();
        // Marker file created by the hook
        let marker = dir.path().join("dead_letter_fired");
        std::fs::write(
            dir.path().join(".decree/routines/on-dead-letter.sh"),
            format!(
                "#!/usr/bin/env bash\ntouch {}\n",
                marker.to_string_lossy()
            ),
        )
        .unwrap();

        std::fs::write(
            dir.path().join(".decree/config.yml"),
            "commands:\n  ai_router: echo\n  ai_interactive: echo\nhooks:\n  onDeadLetter: on-dead-letter\n",
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

        let _ = process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);

        assert!(marker.exists(), "onDeadLetter hook did not fire");
    }

    #[test]
    fn test_on_dead_letter_hook_does_not_fire_on_before_each_failure() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\nexit 0\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".decree/routines/fail-before.sh"),
            "#!/usr/bin/env bash\nexit 1\n",
        )
        .unwrap();
        let marker = dir.path().join("dead_letter_fired");
        std::fs::write(
            dir.path().join(".decree/routines/on-dead-letter.sh"),
            format!(
                "#!/usr/bin/env bash\ntouch {}\n",
                marker.to_string_lossy()
            ),
        )
        .unwrap();

        std::fs::write(
            dir.path().join(".decree/config.yml"),
            "commands:\n  ai_router: echo\n  ai_interactive: echo\nhooks:\n  beforeEach: fail-before\n  onDeadLetter: on-dead-letter\n",
        )
        .unwrap();

        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\nroutine: develop\n---\nTest.\n";
        std::fs::write(
            dir.path().join(".decree/inbox/D0001-1432-test-0.md"),
            content,
        )
        .unwrap();

        let config = AppConfig::load_from_project(dir.path()).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));

        let _ = process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);

        assert!(!marker.exists(), "onDeadLetter should NOT fire on beforeEach failure");
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

    // ── Token-exhaustion helpers ───────────────────────────────────────────

    #[test]
    fn test_detect_token_exhaustion_positive() {
        assert!(detect_token_exhaustion(
            "Claude AI usage limit reached. Limits reset at 10:00 PM"
        ));
        assert!(detect_token_exhaustion("usage limit reached\nresets at 5:00 AM"));
        assert!(detect_token_exhaustion("USAGE LIMIT exceeded. Will RESET tomorrow."));
    }

    #[test]
    fn test_detect_token_exhaustion_negative() {
        assert!(!detect_token_exhaustion("error: command not found"));
        assert!(!detect_token_exhaustion("usage limit reached")); // no "reset"
        assert!(!detect_token_exhaustion("system reset performed")); // no "usage limit"
        assert!(!detect_token_exhaustion(""));
    }

    #[test]
    fn test_extract_reset_time_parseable_pm() {
        let log = "Claude AI usage limit reached. Limits reset at 11:59 PM";
        let result = extract_reset_time(log);
        assert!(result.is_some(), "expected Some but got None");
        // The extracted time must be in the future.
        assert!(result.unwrap() > chrono::Local::now());
    }

    #[test]
    fn test_extract_reset_time_parseable_am() {
        let log = "usage limit reached. Limits reset at 6:00 AM";
        let result = extract_reset_time(log);
        assert!(result.is_some(), "expected Some but got None");
        assert!(result.unwrap() > chrono::Local::now());
    }

    #[test]
    fn test_extract_reset_time_resets_variant() {
        let log = "Limits resets at 10:00 PM";
        let result = extract_reset_time(log);
        assert!(result.is_some(), "expected Some but got None");
    }

    #[test]
    fn test_extract_reset_time_no_ampm() {
        // Time present but no AM/PM marker → None
        let log = "Limits reset at 10:00 today";
        let result = extract_reset_time(log);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_reset_time_no_time_in_log() {
        let log = "usage limit reached. Please wait for the reset.";
        let result = extract_reset_time(log);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_reset_time_adds_24h_when_past() {
        // Pick a time that is definitely in the past (midnight = 12:00 AM).
        // If current time is past midnight (which it always is), it should add 24h.
        let log = "Limits reset at 12:00 AM";
        let result = extract_reset_time(log);
        // 12:00 AM = midnight; unless we're running at exactly midnight, this is in the past
        // and should be bumped to tomorrow midnight, so always > now.
        assert!(result.is_some());
        assert!(result.unwrap() > chrono::Local::now());
    }

    // ── Token-exhaustion integration ──────────────────────────────────────

    #[test]
    fn test_token_exhaustion_does_not_dead_letter() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        // Routine prints the usage-limit message and exits non-zero.
        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\necho 'Claude AI usage limit reached. Limits reset at 11:59 PM'\nexit 1\n",
        )
        .unwrap();

        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\nroutine: develop\nmigration: 01-token-test.md\n---\nTest.\n";
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

        let result = process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);

        // Must succeed (not an error) so DrainResult.dead_lettered is not set.
        assert!(result.is_ok(), "expected Ok, got: {result:?}");

        // Message must NOT be in dead-letter dir.
        assert!(
            !dir.path().join(".decree/inbox/dead/D0001-1432-test-0.md").exists(),
            "message was incorrectly dead-lettered"
        );

        // Migration must NOT appear in processed.md.
        let processed = std::fs::read_to_string(dir.path().join(".decree/processed.md")).unwrap();
        assert!(
            !processed.contains("01-token-test.md"),
            "migration was incorrectly marked as processed"
        );
    }

    #[test]
    fn test_token_exhaustion_inbox_message_removed() {
        // After token-exhaustion handling the inbox message is deleted so the
        // outer migration loop can re-create it fresh.
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\necho 'usage limit reached. Limits reset at 11:59 PM'\nexit 1\n",
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

        let _ = process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);

        // Inbox message must be gone (so drain_inbox exits cleanly).
        assert!(
            !dir.path().join(".decree/inbox/D0001-1432-test-0.md").exists(),
            "inbox message was not removed"
        );
    }

    #[test]
    fn test_token_exhaustion_detected_on_first_attempt() {
        // Even when max_retries > 1, token exhaustion should be caught on the
        // first failed attempt (no wasted retries).
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\necho 'Claude AI usage limit reached. Limits reset at 11:59 PM'\nexit 1\n",
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

        let result = process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);

        // Should return Ok (not exhaust all retries then die).
        assert!(result.is_ok(), "expected Ok, got: {result:?}");

        // Only routine.log should exist (not routine-2.log), proving it
        // exited after the first attempt.
        let run_dir = dir.path().join(".decree/runs/D0001-1432-test-0");
        assert!(run_dir.join("routine.log").exists());
        assert!(!run_dir.join("routine-2.log").exists());
    }

    #[test]
    fn test_normal_failure_still_dead_letters_with_exhaustion_env_set() {
        // A routine with a normal error (no token-exhaustion pattern) must still
        // dead-letter even when the skip-wait env var is set.
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\necho 'some normal error'\nexit 1\n",
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

        let result = process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);
        assert!(result.is_err());
        assert!(dir
            .path()
            .join(".decree/inbox/dead/D0001-1432-test-0.md")
            .exists());
    }

    #[test]
    fn test_drain_inbox_dead_lettered_on_exhaustion() {
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

        let result = drain_inbox(dir.path(), &config, &shutdown, Some("D0001-1432-test")).unwrap();
        assert!(result.dead_lettered);
        assert!(dir.path().join(".decree/inbox/dead/D0001-1432-test-0.md").exists());
    }

    #[test]
    fn test_drain_inbox_not_dead_lettered_on_success() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\necho 'done'\n",
        )
        .unwrap();

        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\nroutine: develop\n---\nTest.\n";
        std::fs::write(
            dir.path().join(".decree/inbox/D0001-1432-test-0.md"),
            content,
        )
        .unwrap();

        let config = AppConfig::load_from_project(dir.path()).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));

        let result = drain_inbox(dir.path(), &config, &shutdown, Some("D0001-1432-test")).unwrap();
        assert!(!result.dead_lettered);
    }

    #[test]
    fn test_drain_inbox_inbox_only_dead_lettered_flag_is_set() {
        // Inbox-only drain (prefer_chain=None) still sets dead_lettered when
        // a message fails — but the caller (run()) ignores it for this path.
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

        // prefer_chain=None (inbox-only drain)
        let result = drain_inbox(dir.path(), &config, &shutdown, None).unwrap();
        // dead_lettered is set — but the migration loop ignores this for inbox-only drains
        assert!(result.dead_lettered);
    }

    #[test]
    fn test_dry_run_no_migrations() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);
        // No migrations dir content
        let result = run_dry(dir.path());
        assert!(result.is_ok());
    }

    #[test]
    fn test_hook_output_captured_in_log() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        // Create routine and hook scripts
        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\necho 'done'\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".decree/routines/git-baseline.sh"),
            "#!/usr/bin/env bash\necho 'BASELINE SAVED'\n",
        )
        .unwrap();

        // Config with beforeEach hook
        std::fs::write(
            dir.path().join(".decree/config.yml"),
            "commands:\n  ai_router: echo\n  ai_interactive: echo\nhooks:\n  beforeEach: git-baseline\n",
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

        let log = std::fs::read_to_string(
            dir.path().join(".decree/runs/D0001-1432-test-0/routine.log"),
        )
        .unwrap();
        assert!(log.contains("[decree] hook beforeEach start"));
        assert!(log.contains("BASELINE SAVED"));
        assert!(log.contains("[decree] hook beforeEach end"));
    }

    #[test]
    fn test_hook_no_output_no_log_block() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\necho 'done'\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".decree/routines/silent.sh"),
            "#!/usr/bin/env bash\nexit 0\n",
        )
        .unwrap();

        std::fs::write(
            dir.path().join(".decree/config.yml"),
            "commands:\n  ai_router: echo\n  ai_interactive: echo\nhooks:\n  beforeEach: silent\n",
        )
        .unwrap();

        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\nroutine: develop\n---\nTest.\n";
        std::fs::write(
            dir.path().join(".decree/inbox/D0001-1432-test-0.md"),
            content,
        )
        .unwrap();

        let config = AppConfig::load_from_project(dir.path()).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));

        process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown).unwrap();

        let log = std::fs::read_to_string(
            dir.path().join(".decree/runs/D0001-1432-test-0/routine.log"),
        )
        .unwrap();
        // Silent hook should produce no hook log block
        assert!(!log.contains("[decree] hook"));
    }

    #[test]
    fn test_hook_failure_output_in_log() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\necho 'done'\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".decree/routines/fail-hook.sh"),
            "#!/usr/bin/env bash\necho 'partial output'\necho 'error info' >&2\nexit 1\n",
        )
        .unwrap();

        std::fs::write(
            dir.path().join(".decree/config.yml"),
            "commands:\n  ai_router: echo\n  ai_interactive: echo\nhooks:\n  beforeEach: fail-hook\n",
        )
        .unwrap();

        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\nroutine: develop\n---\nTest.\n";
        std::fs::write(
            dir.path().join(".decree/inbox/D0001-1432-test-0.md"),
            content,
        )
        .unwrap();

        let config = AppConfig::load_from_project(dir.path()).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));

        let result =
            process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);
        assert!(result.is_err());

        let run_dir = dir.path().join(".decree/runs/D0001-1432-test-0");
        let log = std::fs::read_to_string(run_dir.join("routine.log")).unwrap();
        assert!(log.contains("partial output"));
        assert!(log.contains("error info"));
    }

    #[test]
    fn test_write_hook_log_empty_no_write() {
        let dir = TempDir::new().unwrap();
        let log_path = dir.path().join("routine.log");

        write_hook_log(&log_path, HookType::BeforeEach, "").unwrap();

        // No log file should be created
        assert!(!log_path.exists());
    }

    #[test]
    fn test_write_hook_log_with_output() {
        let dir = TempDir::new().unwrap();
        let log_path = dir.path().join("routine.log");

        write_hook_log(&log_path, HookType::AfterEach, "hook output here").unwrap();

        let log = std::fs::read_to_string(&log_path).unwrap();
        assert!(log.contains("[decree] hook afterEach start"));
        assert!(log.contains("hook output here"));
        assert!(log.contains("[decree] hook afterEach end"));
    }

    #[test]
    fn test_invoke_ai_router_success() {
        // Use printf to avoid trailing args from the prompt
        let result = invoke_ai_router("printf rust-develop", "ignored prompt");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "rust-develop");
    }

    #[test]
    fn test_invoke_ai_router_failure() {
        let result = invoke_ai_router("exit 1", "test prompt");
        assert!(result.is_err());
    }

    #[test]
    fn test_invoke_ai_router_with_prompt_placeholder() {
        let result = invoke_ai_router("echo {prompt}", "hello world");
        assert!(result.is_ok());
        // The prompt is shell-escaped, so it comes through as the literal string
        assert!(result.unwrap().contains("hello world"));
    }

    #[test]
    fn test_ai_router_used_in_normalize() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        // Create both routines
        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\necho 'done'\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".decree/routines/rust-develop.sh"),
            "#!/usr/bin/env bash\necho 'done'\n",
        )
        .unwrap();

        // Router template
        std::fs::write(
            dir.path().join(".decree/router.md"),
            "Select routine.\n\n{routines}\n\n{message}\n",
        )
        .unwrap();

        // Config with ai_router that prints "rust-develop" (printf ignores extra args)
        std::fs::write(
            dir.path().join(".decree/config.yml"),
            "commands:\n  ai_router: printf rust-develop\n  ai_interactive: echo\n",
        )
        .unwrap();

        // Message with NO routine field — should trigger router
        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\n---\nTest body.\n";
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

        // Verify the message was normalized with "rust-develop" routine
        let run_msg = std::fs::read_to_string(
            dir.path().join(".decree/runs/D0001-1432-test-0/message.md"),
        )
        .unwrap();
        assert!(run_msg.contains("routine: rust-develop"));
    }

    #[test]
    fn test_ai_router_fallback_on_empty_config() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\necho 'done'\n",
        )
        .unwrap();

        // Config with empty ai_router
        std::fs::write(
            dir.path().join(".decree/config.yml"),
            "commands:\n  ai_router: ''\n  ai_interactive: echo\ndefault_routine: develop\n",
        )
        .unwrap();

        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\n---\nTest.\n";
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

        let run_msg = std::fs::read_to_string(
            dir.path().join(".decree/runs/D0001-1432-test-0/message.md"),
        )
        .unwrap();
        assert!(run_msg.contains("routine: develop"));
    }

    #[test]
    fn test_ai_router_fallback_on_failure() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\necho 'done'\n",
        )
        .unwrap();

        std::fs::write(
            dir.path().join(".decree/router.md"),
            "{routines}\n{message}\n",
        )
        .unwrap();

        // Router command that fails
        std::fs::write(
            dir.path().join(".decree/config.yml"),
            "commands:\n  ai_router: 'exit 1'\n  ai_interactive: echo\ndefault_routine: develop\n",
        )
        .unwrap();

        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\n---\nTest.\n";
        std::fs::write(
            dir.path().join(".decree/inbox/D0001-1432-test-0.md"),
            content,
        )
        .unwrap();

        let config = AppConfig::load_from_project(dir.path()).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));

        // Should succeed with fallback to default_routine
        let result =
            process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);
        assert!(result.is_ok());

        let run_msg = std::fs::read_to_string(
            dir.path().join(".decree/runs/D0001-1432-test-0/message.md"),
        )
        .unwrap();
        assert!(run_msg.contains("routine: develop"));
    }

    // ── Session ID extraction ─────────────────────────────────────────────

    #[test]
    fn test_extract_session_id_plain() {
        assert_eq!(
            extract_session_id("Session ID: abc123"),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn test_extract_session_id_case_insensitive() {
        assert_eq!(
            extract_session_id("session id: XYZ-789"),
            Some("XYZ-789".to_string())
        );
    }

    #[test]
    fn test_extract_session_id_short_form() {
        assert_eq!(
            extract_session_id("Session: my_session_id"),
            Some("my_session_id".to_string())
        );
    }

    #[test]
    fn test_extract_session_id_multiline() {
        let log = "some output\nSession ID: sess-abc-123\nmore output";
        assert_eq!(
            extract_session_id(log),
            Some("sess-abc-123".to_string())
        );
    }

    #[test]
    fn test_extract_session_id_none_when_absent() {
        assert_eq!(extract_session_id("no session info here"), None);
        assert_eq!(extract_session_id(""), None);
    }

    #[test]
    fn test_extract_session_id_stops_at_non_alphanum() {
        // Only alphanumeric, underscore, and hyphen are captured
        assert_eq!(
            extract_session_id("Session ID: abc123 extra"),
            Some("abc123".to_string())
        );
    }

    // ── Session ID written to run dir after attempt ───────────────────────

    #[test]
    fn test_session_id_written_after_successful_attempt() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\necho 'Session ID: sess-ok-1'\nexit 0\n",
        )
        .unwrap();

        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\nroutine: develop\n---\nTest.\n";
        std::fs::write(
            dir.path().join(".decree/inbox/D0001-1432-test-0.md"),
            content,
        )
        .unwrap();

        let config = AppConfig::load_from_project(dir.path()).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));

        let result = process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);
        assert!(result.is_ok());

        let sid = std::fs::read_to_string(
            dir.path().join(".decree/runs/D0001-1432-test-0/session_id.txt"),
        )
        .unwrap();
        assert_eq!(sid, "sess-ok-1");
    }

    #[test]
    fn test_session_id_not_written_when_absent_from_log() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\necho 'no session info'\nexit 0\n",
        )
        .unwrap();

        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\nroutine: develop\n---\nTest.\n";
        std::fs::write(
            dir.path().join(".decree/inbox/D0001-1432-test-0.md"),
            content,
        )
        .unwrap();

        let config = AppConfig::load_from_project(dir.path()).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));

        let _ = process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);

        assert!(
            !dir.path().join(".decree/runs/D0001-1432-test-0/session_id.txt").exists(),
            "session_id.txt should not be written when log has no session ID"
        );
    }

    // ── Token-exhaustion propagates session ID ────────────────────────────

    #[test]
    fn test_token_exhaustion_writes_token_session_file() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        // Routine prints both the session ID and the token-exhaustion pattern
        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\necho 'Session ID: ses-exhaust-1'\necho 'usage limit reached. Limits reset at 11:59 PM'\nexit 1\n",
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

        let result = process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);
        assert!(result.is_ok());

        let token_session = std::fs::read_to_string(
            dir.path().join(".decree/token_session.txt"),
        )
        .unwrap();
        assert_eq!(token_session, "ses-exhaust-1");
    }

    #[test]
    fn test_token_exhaustion_no_token_session_when_no_session_id() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        // Token exhaustion pattern but no session ID
        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\necho 'usage limit reached. Limits reset at 11:59 PM'\nexit 1\n",
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

        let _ = process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);

        assert!(
            !dir.path().join(".decree/.token_session.txt").exists(),
            "token_session.txt should not be written when log has no session ID"
        );
    }

    #[test]
    fn test_previous_session_id_env_set_on_token_exhaustion_retry() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        // Routine captures DECREE_PREVIOUS_SESSION_ID to a file
        let marker = dir.path().join("captured_session_id.txt");
        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            format!(
                "#!/usr/bin/env bash\necho \"${{DECREE_PREVIOUS_SESSION_ID:-}}\" > {}\nexit 0\n",
                marker.to_string_lossy()
            ),
        )
        .unwrap();

        // Pre-populate token_session.txt as if a prior token-exhaustion occurred
        std::fs::write(
            dir.path().join(".decree/token_session.txt"),
            "prior-session-abc",
        )
        .unwrap();

        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\nroutine: develop\n---\nTest.\n";
        std::fs::write(
            dir.path().join(".decree/inbox/D0001-1432-test-0.md"),
            content,
        )
        .unwrap();

        let config = AppConfig::load_from_project(dir.path()).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));

        let result = process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);
        assert!(result.is_ok());

        let captured = std::fs::read_to_string(&marker).unwrap();
        assert_eq!(captured.trim(), "prior-session-abc");

        // token_session.txt must be consumed (deleted) after first use
        assert!(
            !dir.path().join(".decree/token_session.txt").exists(),
            "token_session.txt should be deleted after being consumed"
        );
    }

    #[test]
    fn test_previous_session_id_not_set_on_normal_retry() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        // First attempt captures the env var, second attempt also captures it.
        // No token_session.txt is present.
        let marker1 = dir.path().join("attempt1_session.txt");
        let marker2 = dir.path().join("attempt2_session.txt");
        let counter = dir.path().join("attempt_counter.txt");
        std::fs::write(&counter, "0").unwrap();
        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            format!(
                "#!/usr/bin/env bash\n\
                count=$(cat {counter})\n\
                count=$((count + 1))\n\
                echo $count > {counter}\n\
                if [ \"$count\" -eq 1 ]; then\n\
                  echo \"${{DECREE_PREVIOUS_SESSION_ID:-none}}\" > {m1}\n\
                  exit 1\n\
                else\n\
                  echo \"${{DECREE_PREVIOUS_SESSION_ID:-none}}\" > {m2}\n\
                  exit 0\n\
                fi\n",
                counter = counter.to_string_lossy(),
                m1 = marker1.to_string_lossy(),
                m2 = marker2.to_string_lossy(),
            ),
        )
        .unwrap();

        let content = "---\nid: D0001-1432-test-0\nchain: D0001-1432-test\nseq: 0\nroutine: develop\n---\nTest.\n";
        std::fs::write(
            dir.path().join(".decree/inbox/D0001-1432-test-0.md"),
            content,
        )
        .unwrap();

        let config = AppConfig {
            max_retries: 2,
            ..AppConfig::load_from_project(dir.path()).unwrap()
        };
        let shutdown = Arc::new(AtomicBool::new(false));

        let result = process_single_message(dir.path(), &config, "D0001-1432-test-0.md", &shutdown);
        assert!(result.is_ok());

        // Neither attempt should have DECREE_PREVIOUS_SESSION_ID set
        let s1 = std::fs::read_to_string(&marker1).unwrap_or_default();
        let s2 = std::fs::read_to_string(&marker2).unwrap_or_default();
        assert_eq!(s1.trim(), "none", "attempt 1 should not have session ID");
        assert_eq!(s2.trim(), "none", "attempt 2 (normal retry) should not have session ID");
    }
}
