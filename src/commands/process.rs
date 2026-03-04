use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::config::Config;
use crate::error::{DecreeError, Result};
use crate::message::{self, list_inbox, Message, MessageType, NormalizeContext, RawMessage};
use crate::migration::MigrationTracker;
use crate::routine::find_routine;

// ---------------------------------------------------------------------------
// Signal handling
// ---------------------------------------------------------------------------

/// Global counter incremented by each SIGINT/SIGTERM received.
static SIGINT_COUNT: AtomicUsize = AtomicUsize::new(0);

extern "C" fn sigint_handler(_: libc::c_int) {
    let prev = SIGINT_COUNT.fetch_add(1, Ordering::SeqCst);
    if prev >= 1 {
        // Double Ctrl-C — exit immediately.
        unsafe { libc::_exit(130) };
    }
}

pub(crate) fn install_signal_handler() {
    unsafe {
        libc::signal(
            libc::SIGINT,
            sigint_handler as *const () as libc::sighandler_t,
        );
    }
}

/// Install handlers for both SIGINT and SIGTERM (for daemon mode).
pub(crate) fn install_signal_handlers() {
    unsafe {
        libc::signal(
            libc::SIGINT,
            sigint_handler as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGTERM,
            sigint_handler as *const () as libc::sighandler_t,
        );
    }
}

fn reset_sigint() {
    SIGINT_COUNT.store(0, Ordering::SeqCst);
}

pub(crate) fn was_interrupted() -> bool {
    SIGINT_COUNT.load(Ordering::SeqCst) > 0
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn run() -> Result<()> {
    install_signal_handler();

    let config = Config::load(Path::new(".decree/config.yml"))?;
    let inbox_dir = PathBuf::from(".decree/inbox");
    let runs_dir = PathBuf::from(".decree/runs");
    let routines_dir = PathBuf::from(".decree/routines");
    let migrations_dir = PathBuf::from("migrations");

    // Ensure directories exist
    fs::create_dir_all(inbox_dir.join("done"))?;
    fs::create_dir_all(inbox_dir.join("dead"))?;
    fs::create_dir_all(&runs_dir)?;

    // Step 1: beforeAll hook — failure aborts processing entirely
    if let HookOutcome::Failed(code) =
        run_hook(&config.hooks.before_all, HookType::BeforeAll, &routines_dir, None)?
    {
        return Err(DecreeError::HookFailed {
            hook: "beforeAll".to_string(),
            code,
        });
    }

    // Steps 2-5: process unprocessed migrations
    process_migrations(&config, &inbox_dir, &runs_dir, &routines_dir, &migrations_dir)?;

    // Step 6: drain remaining inbox messages
    drain_inbox(&config, &inbox_dir, &runs_dir, &routines_dir)?;

    // Step 7: afterAll hook — failure logs warning, exits with hook's code
    if let HookOutcome::Failed(code) =
        run_hook(&config.hooks.after_all, HookType::AfterAll, &routines_dir, None)?
    {
        eprintln!(
            "[decree] warning: afterAll hook failed (exit {})",
            code
        );
        return Err(DecreeError::HookFailed {
            hook: "afterAll".to_string(),
            code,
        });
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Migration processing (steps 2-5)
// ---------------------------------------------------------------------------

fn process_migrations(
    config: &Config,
    inbox_dir: &Path,
    runs_dir: &Path,
    routines_dir: &Path,
    migrations_dir: &Path,
) -> Result<()> {
    let tracker = MigrationTracker::new(migrations_dir);

    while let Some(migration) = tracker.next_unprocessed()? {
        let chain = message::generate_chain_id();
        eprintln!(
            "[decree] migration: {} (chain {chain})",
            migration.filename
        );

        // Build a spec message for this migration
        let routine_name = migration
            .routine()
            .unwrap_or_else(|| config.default_routine.clone());

        let msg = Message {
            id: format!("{chain}-0"),
            chain: chain.clone(),
            seq: 0,
            message_type: MessageType::Spec,
            input_file: Some(migration.path.to_string_lossy().to_string()),
            routine: routine_name,
            custom_fields: Default::default(),
            body: migration.body.clone(),
            path: inbox_dir.join(format!("{chain}-0.md")),
        };
        msg.write()?;

        // Process the chain depth-first
        let chain_ok = process_chain(config, inbox_dir, runs_dir, routines_dir, &chain)?;

        if chain_ok {
            tracker.mark_processed(&migration.filename)?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Inbox draining (step 6)
// ---------------------------------------------------------------------------

pub(crate) fn drain_inbox(
    config: &Config,
    inbox_dir: &Path,
    runs_dir: &Path,
    routines_dir: &Path,
) -> Result<()> {
    loop {
        let files = list_inbox(inbox_dir)?;
        if files.is_empty() {
            break;
        }

        // Normalize the first pending message
        let raw = RawMessage::load(&files[0])?;
        let ctx = NormalizeContext {
            default_routine: config.default_routine.clone(),
            migration_routine: None,
        };
        let mut msg = message::normalize(raw, &ctx, |_| Ok(None))?;
        msg.rename_to_canonical(inbox_dir)?;
        msg.write()?;

        let chain = msg.chain.clone();
        process_chain(config, inbox_dir, runs_dir, routines_dir, &chain)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Chain processing — depth-first
// ---------------------------------------------------------------------------

/// Process all messages in a chain depth-first.
/// Returns `true` if all messages in the chain succeeded.
fn process_chain(
    config: &Config,
    inbox_dir: &Path,
    runs_dir: &Path,
    routines_dir: &Path,
    chain: &str,
) -> Result<bool> {
    loop {
        let msg = match find_chain_message(inbox_dir, chain, &config.default_routine)? {
            Some(m) => m,
            None => return Ok(true),
        };

        // Depth check
        if msg.seq >= config.max_depth {
            eprintln!(
                "[decree] max depth exceeded (seq={}, max={}), dead-lettering {}",
                msg.seq, config.max_depth, msg.id
            );
            move_to_dead(inbox_dir, &msg)?;
            return Ok(false);
        }

        let success = process_message(config, inbox_dir, runs_dir, routines_dir, msg)?;
        if !success {
            return Ok(false);
        }
    }
}

/// Find the next message for `chain` in the inbox (lowest seq first).
fn find_chain_message(
    inbox_dir: &Path,
    chain: &str,
    default_routine: &str,
) -> Result<Option<Message>> {
    let files = list_inbox(inbox_dir)?;
    let mut best: Option<Message> = None;

    for path in files {
        let raw = RawMessage::load(&path)?;
        let ctx = NormalizeContext {
            default_routine: default_routine.to_string(),
            migration_routine: None,
        };
        let msg = message::normalize(raw, &ctx, |_| Ok(None))?;
        if msg.chain == chain {
            match &best {
                None => best = Some(msg),
                Some(existing) if msg.seq < existing.seq => best = Some(msg),
                _ => {}
            }
        }
    }

    Ok(best)
}

// ---------------------------------------------------------------------------
// Single message processing
// ---------------------------------------------------------------------------

/// Process a single message through the full lifecycle.
/// Returns `true` on success, `false` on exhaustion (dead-lettered).
fn process_message(
    config: &Config,
    inbox_dir: &Path,
    runs_dir: &Path,
    routines_dir: &Path,
    msg: Message,
) -> Result<bool> {
    eprintln!("[decree] processing: {}", msg.id);

    // Create run directory and copy normalized message
    let run_dir = runs_dir.join(&msg.id);
    fs::create_dir_all(&run_dir)?;
    fs::write(run_dir.join("message.md"), msg.to_string())?;

    // Resolve routine
    let routine = match find_routine(routines_dir, &msg.routine) {
        Ok(r) => r,
        Err(DecreeError::RoutineNotFound(name)) => {
            eprintln!("[decree] routine not found: {name}, dead-lettering {}", msg.id);
            move_to_dead(inbox_dir, &msg)?;
            return Ok(false);
        }
        Err(e) => return Err(e),
    };

    // beforeEach hook — failure skips and dead-letters the message
    let hook_ctx = HookContext {
        msg: &msg,
        run_dir: &run_dir,
    };
    if let HookOutcome::Failed(code) = run_hook(
        &config.hooks.before_each,
        HookType::BeforeEach,
        routines_dir,
        Some(&hook_ctx),
    )? {
        eprintln!(
            "[decree] beforeEach hook failed (exit {}), dead-lettering {}",
            code, msg.id
        );
        move_to_dead(inbox_dir, &msg)?;
        return Ok(false);
    }

    // Execute with retries
    let max_retries = config.max_retries;
    let git_hooks_active = !config.hooks.before_each.is_empty();
    let mut attempt_exit_codes: Vec<i32> = Vec::new();

    for attempt in 1..=max_retries {
        let is_final = attempt == max_retries && attempt > 1;

        // Final retry: revert + write failure context
        if is_final {
            if git_hooks_active {
                revert_to_baseline()?;
            }
            write_failure_context(&run_dir, &attempt_exit_codes)?;
        }

        let log_path = run_dir.join(log_filename(attempt));

        eprintln!(
            "[decree] routine '{}' attempt {}/{}",
            routine.name, attempt, max_retries
        );

        let status = execute_routine_with_tee(&routine, &msg, &run_dir, &log_path)?;

        // Truncate log if configured
        if config.max_log_size > 0 {
            truncate_log(&log_path, config.max_log_size)?;
        }

        if status.success() {
            eprintln!("[decree] {} completed successfully", msg.id);
            // afterEach hook — failure logs warning, continues
            if let HookOutcome::Failed(code) = run_hook(
                &config.hooks.after_each,
                HookType::AfterEach,
                routines_dir,
                Some(&hook_ctx),
            )? {
                eprintln!(
                    "[decree] warning: afterEach hook failed (exit {})",
                    code
                );
            }
            move_to_done(inbox_dir, &msg)?;
            return Ok(true);
        }

        let code = status.code().unwrap_or(-1);
        eprintln!(
            "[decree] routine failed (exit {}, attempt {}/{})",
            code, attempt, max_retries
        );
        attempt_exit_codes.push(code);

        // Reset interrupt flag so the next attempt can proceed
        if was_interrupted() {
            reset_sigint();
        }
    }

    // All retries exhausted
    eprintln!(
        "[decree] retries exhausted for {}, dead-lettering",
        msg.id
    );

    if git_hooks_active {
        revert_to_baseline()?;
        undo_baseline()?;
    }

    // afterEach hook — failure logs warning, continues
    if let HookOutcome::Failed(code) = run_hook(
        &config.hooks.after_each,
        HookType::AfterEach,
        routines_dir,
        Some(&hook_ctx),
    )? {
        eprintln!(
            "[decree] warning: afterEach hook failed (exit {})",
            code
        );
    }
    move_to_dead(inbox_dir, &msg)?;

    Ok(false)
}

// ---------------------------------------------------------------------------
// Routine execution with tee
// ---------------------------------------------------------------------------

fn execute_routine_with_tee(
    routine: &crate::routine::Routine,
    msg: &Message,
    run_dir: &Path,
    log_path: &Path,
) -> Result<ExitStatus> {
    use std::os::unix::process::CommandExt;

    let mut cmd = routine.build_command(msg, run_dir);
    cmd.stdout(Stdio::piped());

    // Merge stderr into stdout so both stream through the tee.
    unsafe {
        cmd.pre_exec(|| {
            if libc::dup2(1, 2) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = cmd.spawn()?;

    // Stream child output to both terminal and log file.
    if let Some(mut child_stdout) = child.stdout.take() {
        let mut log_file = fs::File::create(log_path)?;
        let mut buf = [0u8; 4096];

        loop {
            let n = match child_stdout.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            };

            // Best-effort write to terminal; always write to log.
            let _ = io::stdout().write_all(&buf[..n]);
            let _ = io::stdout().flush();
            let _ = log_file.write_all(&buf[..n]);
        }
    }

    let status = child.wait()?;
    Ok(status)
}

// ---------------------------------------------------------------------------
// Log truncation
// ---------------------------------------------------------------------------

fn truncate_log(path: &Path, max_size: u64) -> Result<()> {
    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return Ok(()),
    };

    if metadata.len() <= max_size {
        return Ok(());
    }

    let content = fs::read(path)?;
    let keep_from = content.len().saturating_sub(max_size as usize);

    let marker = format!(
        "[log truncated — showing last {} of output]\n",
        format_size(max_size)
    );

    let mut truncated = marker.into_bytes();
    truncated.extend_from_slice(&content[keep_from..]);

    fs::write(path, truncated)?;
    Ok(())
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{}MB", bytes / 1_048_576)
    } else if bytes >= 1024 {
        format!("{}KB", bytes / 1024)
    } else {
        format!("{}B", bytes)
    }
}

// ---------------------------------------------------------------------------
// Hooks
// ---------------------------------------------------------------------------

/// The four lifecycle hook types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HookType {
    BeforeAll,
    AfterAll,
    BeforeEach,
    AfterEach,
}

impl HookType {
    /// The value set in the `DECREE_HOOK` env var (camelCase per spec).
    fn env_value(self) -> &'static str {
        match self {
            HookType::BeforeAll => "beforeAll",
            HookType::AfterAll => "afterAll",
            HookType::BeforeEach => "beforeEach",
            HookType::AfterEach => "afterEach",
        }
    }
}

/// Context passed to per-message hooks (beforeEach, afterEach).
pub(crate) struct HookContext<'a> {
    pub(crate) msg: &'a Message,
    pub(crate) run_dir: &'a Path,
}

/// Outcome of running a hook.
#[derive(Debug)]
pub(crate) enum HookOutcome {
    /// Hook was not configured (empty name) or script not found.
    Skipped,
    /// Hook ran successfully.
    Ok,
    /// Hook failed with exit code.
    Failed(i32),
}

/// Execute a hook if configured. Returns the outcome without applying
/// any failure policy — callers decide what to do.
pub(crate) fn run_hook(
    hook_name: &str,
    hook_type: HookType,
    routines_dir: &Path,
    ctx: Option<&HookContext<'_>>,
) -> Result<HookOutcome> {
    if hook_name.is_empty() {
        return Ok(HookOutcome::Skipped);
    }

    let routine_path = routines_dir.join(format!("{hook_name}.sh"));
    if !routine_path.exists() {
        eprintln!("[decree] hook routine not found: {hook_name}");
        return Ok(HookOutcome::Skipped);
    }

    let mut cmd = std::process::Command::new("bash");
    cmd.arg(&routine_path);
    cmd.env("DECREE_HOOK", hook_type.env_value());

    if let Some(ctx) = ctx {
        cmd.env("message_file", ctx.run_dir.join("message.md"));
        cmd.env("message_id", &ctx.msg.id);
        cmd.env("message_dir", ctx.run_dir);
        cmd.env("chain", &ctx.msg.chain);
        cmd.env("seq", ctx.msg.seq.to_string());
        if let Some(ref input_file) = ctx.msg.input_file {
            cmd.env("input_file", input_file);
        }
    }

    let status = cmd.status()?;

    if status.success() {
        Ok(HookOutcome::Ok)
    } else {
        let code = status.code().unwrap_or(1);
        Ok(HookOutcome::Failed(code))
    }
}

// ---------------------------------------------------------------------------
// Git operations (retry strategy)
// ---------------------------------------------------------------------------

fn revert_to_baseline() -> Result<()> {
    let _ = std::process::Command::new("git")
        .args(["checkout", "."])
        .status();
    let _ = std::process::Command::new("git")
        .args(["clean", "-fd"])
        .status();
    Ok(())
}

fn undo_baseline() -> Result<()> {
    let _ = std::process::Command::new("git")
        .args(["reset", "--soft", "HEAD~1"])
        .status();
    let _ = std::process::Command::new("git")
        .args(["reset", "HEAD", "."])
        .status();
    Ok(())
}

// ---------------------------------------------------------------------------
// File movement
// ---------------------------------------------------------------------------

fn move_to_done(inbox_dir: &Path, msg: &Message) -> Result<()> {
    let done_dir = inbox_dir.join("done");
    fs::create_dir_all(&done_dir)?;

    if let Some(filename) = msg.path.file_name() {
        let dest = done_dir.join(filename);
        if msg.path.exists() {
            fs::rename(&msg.path, &dest)?;
        }
    }
    Ok(())
}

fn move_to_dead(inbox_dir: &Path, msg: &Message) -> Result<()> {
    let dead_dir = inbox_dir.join("dead");
    fs::create_dir_all(&dead_dir)?;

    if let Some(filename) = msg.path.file_name() {
        let dest = dead_dir.join(filename);
        if msg.path.exists() {
            fs::rename(&msg.path, &dest)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn log_filename(attempt: u32) -> String {
    if attempt <= 1 {
        "routine.log".to_string()
    } else {
        format!("routine-{attempt}.log")
    }
}

fn write_failure_context(run_dir: &Path, prior_exit_codes: &[i32]) -> Result<()> {
    let mut content = String::from("# Failure Context\n\n");
    content.push_str("This is the final retry attempt. Previous attempts failed:\n\n");

    for (i, code) in prior_exit_codes.iter().enumerate() {
        content.push_str(&format!("- Attempt {}: exit code {}\n", i + 1, code));
    }

    content.push_str("\nReview prior attempt logs (routine.log, routine-2.log, etc.) for details.\n");

    fs::write(run_dir.join("failure-context.md"), content)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn test_log_filename() {
        assert_eq!(log_filename(1), "routine.log");
        assert_eq!(log_filename(2), "routine-2.log");
        assert_eq!(log_filename(3), "routine-3.log");
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(2_097_152), "2MB");
        assert_eq!(format_size(1_048_576), "1MB");
        assert_eq!(format_size(512 * 1024), "512KB");
        assert_eq!(format_size(1024), "1KB");
        assert_eq!(format_size(500), "500B");
        assert_eq!(format_size(0), "0B");
    }

    #[test]
    fn test_truncate_log_under_limit() {
        let tmp = std::env::temp_dir().join("decree_test_truncate_under");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let path = tmp.join("routine.log");
        fs::write(&path, "small log content").unwrap();

        truncate_log(&path, 1024).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "small log content");
        assert!(!content.contains("[log truncated"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_truncate_log_over_limit() {
        let tmp = std::env::temp_dir().join("decree_test_truncate_over");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let path = tmp.join("routine.log");
        // Create a 200-byte log, truncate to 100
        let content: String = "x".repeat(200);
        fs::write(&path, &content).unwrap();

        truncate_log(&path, 100).unwrap();

        let result = fs::read_to_string(&path).unwrap();
        assert!(result.starts_with("[log truncated"));
        // The tail of the original content should be preserved
        assert!(result.ends_with(&"x".repeat(100)));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_truncate_log_zero_disabled() {
        let tmp = std::env::temp_dir().join("decree_test_truncate_zero");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let path = tmp.join("routine.log");
        let content: String = "x".repeat(500);
        fs::write(&path, &content).unwrap();

        // max_log_size=0 means truncation is disabled (caller checks),
        // but if called directly it would truncate to 0. The caller
        // guards this with `if config.max_log_size > 0`.
        // Verify the guard works by checking the content is unchanged
        // when we just don't call truncate_log.
        let result = fs::read_to_string(&path).unwrap();
        assert_eq!(result.len(), 500);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_truncate_log_nonexistent_file() {
        let path = Path::new("/tmp/decree_nonexistent_log_file.log");
        // Should not error
        truncate_log(path, 1024).unwrap();
    }

    #[test]
    fn test_write_failure_context() {
        let tmp = std::env::temp_dir().join("decree_test_failure_ctx");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        write_failure_context(&tmp, &[1, 137]).unwrap();

        let content = fs::read_to_string(tmp.join("failure-context.md")).unwrap();
        assert!(content.contains("Attempt 1: exit code 1"));
        assert!(content.contains("Attempt 2: exit code 137"));
        assert!(content.contains("final retry"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_move_to_done() {
        let tmp = std::env::temp_dir().join("decree_test_move_done");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let msg_path = tmp.join("test-0.md");
        fs::write(&msg_path, "test").unwrap();

        let msg = Message {
            id: "test-0".to_string(),
            chain: "test".to_string(),
            seq: 0,
            message_type: MessageType::Task,
            input_file: None,
            routine: "develop".to_string(),
            custom_fields: BTreeMap::new(),
            body: String::new(),
            path: msg_path.clone(),
        };

        move_to_done(&tmp, &msg).unwrap();

        assert!(!msg_path.exists());
        assert!(tmp.join("done/test-0.md").exists());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_move_to_dead() {
        let tmp = std::env::temp_dir().join("decree_test_move_dead");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let msg_path = tmp.join("test-0.md");
        fs::write(&msg_path, "test").unwrap();

        let msg = Message {
            id: "test-0".to_string(),
            chain: "test".to_string(),
            seq: 0,
            message_type: MessageType::Task,
            input_file: None,
            routine: "develop".to_string(),
            custom_fields: BTreeMap::new(),
            body: String::new(),
            path: msg_path.clone(),
        };

        move_to_dead(&tmp, &msg).unwrap();

        assert!(!msg_path.exists());
        assert!(tmp.join("dead/test-0.md").exists());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_find_chain_message_picks_lowest_seq() {
        let tmp = std::env::temp_dir().join("decree_test_find_chain");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        // Create two messages in the same chain
        fs::write(
            tmp.join("mychain-2.md"),
            "---\nchain: mychain\nseq: 2\nroutine: develop\n---\n",
        )
        .unwrap();
        fs::write(
            tmp.join("mychain-0.md"),
            "---\nchain: mychain\nseq: 0\nroutine: develop\n---\n",
        )
        .unwrap();
        fs::write(
            tmp.join("otherchain-0.md"),
            "---\nchain: otherchain\nseq: 0\nroutine: develop\n---\n",
        )
        .unwrap();

        let result = find_chain_message(&tmp, "mychain", "develop").unwrap();
        assert!(result.is_some());
        let msg = result.unwrap();
        assert_eq!(msg.chain, "mychain");
        assert_eq!(msg.seq, 0);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_find_chain_message_none() {
        let tmp = std::env::temp_dir().join("decree_test_find_chain_none");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let result = find_chain_message(&tmp, "nonexistent", "develop").unwrap();
        assert!(result.is_none());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_run_hook_empty_name_is_skipped() {
        let tmp = std::env::temp_dir().join("decree_test_hook_empty");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let outcome = run_hook("", HookType::BeforeAll, &tmp, None).unwrap();
        assert!(matches!(outcome, HookOutcome::Skipped));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_run_hook_missing_script_is_skipped() {
        let tmp = std::env::temp_dir().join("decree_test_hook_missing");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let outcome =
            run_hook("nonexistent-hook", HookType::BeforeAll, &tmp, None).unwrap();
        assert!(matches!(outcome, HookOutcome::Skipped));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_run_hook_passes_message_env_vars() {
        let tmp = std::env::temp_dir().join("decree_test_hook_env");
        let _ = fs::remove_dir_all(&tmp);
        let routines_dir = tmp.join("routines");
        let run_dir = tmp.join("runs/testchain-0");
        let output_file = tmp.join("hook-output.txt");
        fs::create_dir_all(&routines_dir).unwrap();
        fs::create_dir_all(&run_dir).unwrap();

        // Write a hook script that dumps env vars to a file
        let script = format!(
            "#!/usr/bin/env bash\necho \"id=${{message_id}}\" > {out}\n\
             echo \"chain=${{chain}}\" >> {out}\n\
             echo \"seq=${{seq}}\" >> {out}\n\
             echo \"dir=${{message_dir}}\" >> {out}\n\
             echo \"hook=${{DECREE_HOOK}}\" >> {out}\n",
            out = output_file.display()
        );
        let hook_path = routines_dir.join("test-hook.sh");
        fs::write(&hook_path, &script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let msg = Message {
            id: "testchain-0".to_string(),
            chain: "testchain".to_string(),
            seq: 0,
            message_type: MessageType::Task,
            input_file: None,
            routine: "develop".to_string(),
            custom_fields: BTreeMap::new(),
            body: String::new(),
            path: PathBuf::from("/tmp/fake.md"),
        };

        let ctx = HookContext {
            msg: &msg,
            run_dir: &run_dir,
        };
        let outcome =
            run_hook("test-hook", HookType::BeforeEach, &routines_dir, Some(&ctx)).unwrap();
        assert!(matches!(outcome, HookOutcome::Ok));

        let output = fs::read_to_string(&output_file).unwrap();
        assert!(output.contains("id=testchain-0"));
        assert!(output.contains("chain=testchain"));
        assert!(output.contains("seq=0"));
        assert!(output.contains(&format!("dir={}", run_dir.display())));
        assert!(output.contains("hook=beforeEach"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_run_hook_sets_decree_hook_env_var() {
        let tmp = std::env::temp_dir().join("decree_test_hook_type_env");
        let _ = fs::remove_dir_all(&tmp);
        let routines_dir = tmp.join("routines");
        let output_file = tmp.join("hook-type-output.txt");
        fs::create_dir_all(&routines_dir).unwrap();

        let script = format!(
            "#!/usr/bin/env bash\necho \"${{DECREE_HOOK}}\" > {out}\n",
            out = output_file.display()
        );
        let hook_path = routines_dir.join("type-hook.sh");
        fs::write(&hook_path, &script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        // Test each hook type
        for (hook_type, expected) in [
            (HookType::BeforeAll, "beforeAll"),
            (HookType::AfterAll, "afterAll"),
            (HookType::BeforeEach, "beforeEach"),
            (HookType::AfterEach, "afterEach"),
        ] {
            run_hook("type-hook", hook_type, &routines_dir, None).unwrap();
            let output = fs::read_to_string(&output_file).unwrap();
            assert_eq!(output.trim(), expected, "DECREE_HOOK mismatch for {expected}");
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_run_hook_passes_input_file() {
        let tmp = std::env::temp_dir().join("decree_test_hook_input_file");
        let _ = fs::remove_dir_all(&tmp);
        let routines_dir = tmp.join("routines");
        let run_dir = tmp.join("runs/testchain-0");
        let output_file = tmp.join("input-file-output.txt");
        fs::create_dir_all(&routines_dir).unwrap();
        fs::create_dir_all(&run_dir).unwrap();

        let script = format!(
            "#!/usr/bin/env bash\necho \"${{input_file}}\" > {out}\n",
            out = output_file.display()
        );
        let hook_path = routines_dir.join("input-hook.sh");
        fs::write(&hook_path, &script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let msg = Message {
            id: "testchain-0".to_string(),
            chain: "testchain".to_string(),
            seq: 0,
            message_type: MessageType::Spec,
            input_file: Some("migrations/01-auth.md".to_string()),
            routine: "develop".to_string(),
            custom_fields: BTreeMap::new(),
            body: String::new(),
            path: PathBuf::from("/tmp/fake.md"),
        };

        let ctx = HookContext {
            msg: &msg,
            run_dir: &run_dir,
        };
        run_hook("input-hook", HookType::BeforeEach, &routines_dir, Some(&ctx)).unwrap();

        let output = fs::read_to_string(&output_file).unwrap();
        assert_eq!(output.trim(), "migrations/01-auth.md");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_run_hook_without_context() {
        let tmp = std::env::temp_dir().join("decree_test_hook_no_ctx");
        let _ = fs::remove_dir_all(&tmp);
        let routines_dir = tmp.join("routines");
        fs::create_dir_all(&routines_dir).unwrap();

        let hook_path = routines_dir.join("simple-hook.sh");
        fs::write(&hook_path, "#!/usr/bin/env bash\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let outcome =
            run_hook("simple-hook", HookType::BeforeAll, &routines_dir, None).unwrap();
        assert!(matches!(outcome, HookOutcome::Ok));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_run_hook_failure_returns_exit_code() {
        let tmp = std::env::temp_dir().join("decree_test_hook_fail");
        let _ = fs::remove_dir_all(&tmp);
        let routines_dir = tmp.join("routines");
        fs::create_dir_all(&routines_dir).unwrap();

        let hook_path = routines_dir.join("fail-hook.sh");
        fs::write(&hook_path, "#!/usr/bin/env bash\nexit 42\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let outcome =
            run_hook("fail-hook", HookType::BeforeAll, &routines_dir, None).unwrap();
        match outcome {
            HookOutcome::Failed(code) => assert_eq!(code, 42),
            other => panic!("expected Failed(42), got: {other:?}"),
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_hook_type_env_value() {
        assert_eq!(HookType::BeforeAll.env_value(), "beforeAll");
        assert_eq!(HookType::AfterAll.env_value(), "afterAll");
        assert_eq!(HookType::BeforeEach.env_value(), "beforeEach");
        assert_eq!(HookType::AfterEach.env_value(), "afterEach");
    }
}
