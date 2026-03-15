use crate::config::{self, AppConfig};
use crate::error::{color, DecreeError, EXIT_PRECHECK};
use crate::hooks;
use crate::message::{self, InboxMessage, RoutineInfo};
use crate::routine::{self, CustomParam, RoutineDetail};
use chrono::Local;
use std::collections::BTreeMap;
use std::io::{self, BufRead};
use std::path::Path;

/// Run the `decree routine [name]` command.
pub fn run(project_root: &Path, name: Option<&str>) -> Result<(), DecreeError> {
    let mut config = AppConfig::load_from_project(project_root)?;

    // Run discovery so we see newly added routines
    if super::routine_sync::discover(project_root, &mut config, None)? {
        config.save(project_root)?;
    }

    let routines = message::list_routines(project_root, &config)?;

    if routines.is_empty() {
        println!("No routines found in .decree/routines/");
        return Ok(());
    }

    match name {
        Some(name) => run_named(project_root, &config, &routines, name),
        None => run_select(project_root, &config, &routines),
    }
}

/// Run with a specific routine name given on the command line.
fn run_named(
    project_root: &Path,
    config: &AppConfig,
    routines: &[RoutineInfo],
    name: &str,
) -> Result<(), DecreeError> {
    // Find the routine
    let info = match routines.iter().find(|r| r.name == name) {
        Some(r) => r,
        None => return routine_not_found(name, routines),
    };

    let detail = routine::routine_detail(project_root, &config, info)?;

    if !color::is_tty() {
        print_detail_view(&detail);
        return Ok(());
    }

    guided_flow(project_root, &config, &detail)
}

/// Run with interactive selection (no name given).
fn run_select(
    project_root: &Path,
    config: &AppConfig,
    routines: &[RoutineInfo],
) -> Result<(), DecreeError> {
    if !color::is_tty() {
        print_list_view(routines);
        return Ok(());
    }

    // Interactive: arrow-key selector
    let options: Vec<String> = routines
        .iter()
        .map(|r| {
            if r.description.is_empty() {
                r.name.clone()
            } else {
                format!("{:<16} {}", r.name, r.description)
            }
        })
        .collect();

    // Pre-highlight the default routine
    let default_idx = routines
        .iter()
        .position(|r| r.name == config.default_routine)
        .unwrap_or(0);

    let selection = inquire::Select::new("Select a routine:", options)
        .with_starting_cursor(default_idx)
        .prompt()
        .map_err(|e| DecreeError::Other(format!("selection cancelled: {e}")))?;

    // Extract routine name from selection string
    let selected_name = selection.split_whitespace().next().unwrap_or(&selection);
    let info = routines
        .iter()
        .find(|r| r.name == selected_name)
        .ok_or_else(|| DecreeError::Other("selected routine not found".into()))?;

    let detail = routine::routine_detail(project_root, config, info)?;
    guided_flow(project_root, config, &detail)
}

/// Print the routine list for non-TTY output.
fn print_list_view(routines: &[RoutineInfo]) {
    for r in routines {
        if r.description.is_empty() {
            println!("  {}", r.name);
        } else {
            println!("  {:<16} {}", r.name, r.description);
        }
    }
}

/// Print the detail view for non-TTY output.
fn print_detail_view(detail: &RoutineDetail) {
    let rel_path = if let Some(pos) = detail.script_path.find(".decree/") {
        &detail.script_path[pos..]
    } else {
        &detail.script_path
    };

    println!("{} ({})", detail.info.name, rel_path);
    if !detail.long_description.is_empty() {
        println!();
        for line in detail.long_description.lines() {
            println!("  {line}");
        }
    }
    if !detail.custom_params.is_empty() {
        println!();
        println!("  Parameters:");
        for p in &detail.custom_params {
            println!("    {}: [default: \"{}\"]", p.name, p.default);
        }
    }
}

/// The interactive guided flow: description → pre-check → params → body → execute.
fn guided_flow(
    project_root: &Path,
    config: &AppConfig,
    detail: &RoutineDetail,
) -> Result<(), DecreeError> {
    // Step 2: Show description and pre-check
    print_detail_view(detail);
    println!();

    let precheck_result = routine::run_precheck(project_root, config, &detail.info.name)?;
    match &precheck_result {
        None => {
            println!("  Pre-check: {}", color::success("PASS"));
        }
        Some(reason) => {
            println!(
                "  Pre-check: {}: {}",
                color::error("FAIL"),
                reason
            );
            println!();
            let cont = inquire::Confirm::new("Continue anyway?")
                .with_default(false)
                .prompt()
                .map_err(|e| DecreeError::Other(format!("prompt cancelled: {e}")))?;
            if !cont {
                return Ok(());
            }
        }
    }
    println!();

    // Step 3: Prompt for custom parameters
    let mut param_values: Vec<(String, String)> = Vec::new();
    for p in &detail.custom_params {
        let value = prompt_param(p)?;
        param_values.push((p.name.clone(), value));
    }

    // Step 4: Message body
    let body = prompt_body()?;

    // Step 5: Summary and execute
    println!();
    println!("Running {}:", color::bold(&detail.info.name));
    for (name, value) in &param_values {
        println!("  {name}: {value}");
    }
    if !body.is_empty() {
        let display_body = if body.len() > 60 {
            format!("\"{}...\"", &body[..57])
        } else {
            format!("\"{body}\"")
        };
        println!("  body: {display_body}");
    }
    println!();
    println!("Press Enter to run, Ctrl-C to cancel.");

    // Wait for Enter
    let mut buf = String::new();
    io::stdin()
        .read_line(&mut buf)
        .map_err(DecreeError::Io)?;

    // Create and process the message
    execute_routine(project_root, config, detail, &param_values, &body)
}

/// Prompt for a single custom parameter value.
fn prompt_param(param: &CustomParam) -> Result<String, DecreeError> {
    let prompt_text = format!("{} [default: \"{}\"]", param.name, param.default);
    let input = inquire::Text::new(&prompt_text)
        .with_default(&param.default)
        .prompt()
        .map_err(|e| DecreeError::Other(format!("prompt cancelled: {e}")))?;

    Ok(input)
}

/// Prompt for multi-line message body. Empty line submits.
fn prompt_body() -> Result<String, DecreeError> {
    println!("Message body [recommended, empty line to submit]:");

    let stdin = io::stdin();
    let mut lines = Vec::new();

    for line in stdin.lock().lines() {
        let line = line.map_err(DecreeError::Io)?;
        if line.is_empty() {
            break;
        }
        lines.push(line);
    }

    Ok(lines.join("\n"))
}

/// Create an inbox message and process only that single message.
///
/// Unlike `process::run()`, this does NOT:
/// - Run beforeAll/afterAll hooks
/// - Scan for pending migrations
/// - Drain other inbox messages
///
/// It DOES run beforeEach/afterEach hooks, retry logic, and dead-lettering.
fn execute_routine(
    project_root: &Path,
    config: &AppConfig,
    detail: &RoutineDetail,
    param_values: &[(String, String)],
    body: &str,
) -> Result<(), DecreeError> {
    let now = Local::now();
    let hhmm = now.format("%H%M").to_string();
    let day = message::next_day_counter(project_root, &hhmm)?;
    let chain = message::build_chain_id(&day, &hhmm, &detail.info.name);
    let seq = 0u32;
    let full_id = format!("{chain}-{seq}");
    let filename = format!("{full_id}.md");

    let mut custom_fields = BTreeMap::new();
    for (name, value) in param_values {
        custom_fields.insert(
            name.clone(),
            serde_yaml::Value::String(value.clone()),
        );
    }

    let msg = InboxMessage {
        id: Some(full_id),
        chain: Some(chain),
        seq: Some(seq),
        routine: Some(detail.info.name.clone()),
        migration: None,
        body: body.to_string(),
        custom_fields,
        filename: filename.clone(),
    };

    // Write to inbox
    let inbox_dir = project_root
        .join(config::DECREE_DIR)
        .join(config::INBOX_DIR);
    std::fs::create_dir_all(&inbox_dir)?;
    msg.write_to_inbox(project_root)?;

    println!("Message created: {}", msg.filename);

    // Process only this single message (no beforeAll/afterAll, no inbox drain)
    let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    super::process::process_single_message(project_root, config, &filename, &shutdown)
}

/// Handle unknown routine: fuzzy match or list available.
fn routine_not_found(name: &str, routines: &[RoutineInfo]) -> Result<(), DecreeError> {
    if let Some(suggestion) = routine::find_closest_routine(name, routines, 3) {
        Err(DecreeError::Other(format!(
            "unknown routine '{name}'\n\nDid you mean '{suggestion}'?"
        )))
    } else {
        let mut msg = format!("unknown routine '{name}'\n\nAvailable routines:");
        for r in routines {
            if r.description.is_empty() {
                msg.push_str(&format!("\n  {}", r.name));
            } else {
                msg.push_str(&format!("\n  {:<16} {}", r.name, r.description));
            }
        }
        Err(DecreeError::Other(msg))
    }
}

/// Run the `decree verify` command — run all pre-checks.
pub fn verify(project_root: &Path) -> Result<(), DecreeError> {
    let mut config = AppConfig::load_from_project(project_root)?;

    // Run discovery so verify sees newly added/removed routines
    if super::routine_sync::discover(project_root, &mut config, None)? {
        config.save(project_root)?;
    }

    let routines = message::list_routines(project_root, &config)?;

    if routines.is_empty() {
        println!("No routines found in .decree/routines/");
        return Ok(());
    }

    println!();
    println!("Routine pre-checks:");

    let mut pass_count = 0;
    let total = routines.len();

    for r in &routines {
        let result = routine::run_precheck(project_root, &config, &r.name)?;
        match result {
            None => {
                println!("  {:<16} {}", r.name, color::success("PASS"));
                pass_count += 1;
            }
            Some(reason) => {
                println!(
                    "  {:<16} {}: {}",
                    r.name,
                    color::error("FAIL"),
                    reason
                );
            }
        }
    }

    println!();
    println!("{pass_count} of {total} routines ready.");

    // Check configured hook routines
    let hook_entries = hooks::configured_hook_names(&config.hooks);
    let routine_names: std::collections::HashSet<&str> =
        routines.iter().map(|r| r.name.as_str()).collect();

    let mut hook_fail = false;

    if !hook_entries.is_empty() {
        println!();
        println!("Hook pre-checks:");

        for (name, hook_type) in &hook_entries {
            let label = format!("{name} ({hook_type})");

            if routine_names.contains(name) {
                let result = routine::run_precheck(project_root, &config, name)?;
                match result {
                    None => {
                        println!("  {:<32} {}", label, color::success("PASS"));
                    }
                    Some(reason) => {
                        println!(
                            "  {:<32} {}: {}",
                            label,
                            color::error("FAIL"),
                            reason
                        );
                        hook_fail = true;
                    }
                }
            } else {
                println!(
                    "  {:<32} {}: routine not found",
                    label,
                    color::error("FAIL"),
                );
                hook_fail = true;
            }
        }
    }

    if pass_count < total || hook_fail {
        std::process::exit(EXIT_PRECHECK);
    }

    Ok(())
}
