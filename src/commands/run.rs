use std::collections::BTreeMap;
use std::io::{self, IsTerminal, Read};
use std::path::Path;

use crate::config::Config;
use crate::error::{find_project_root, DecreeError};
use crate::message::MessageId;
use crate::pipeline::{self, LastRun, ProcessResult};
use crate::routine;

/// Execute the `decree run` command.
pub fn run(
    name: Option<&str>,
    prompt: Option<&str>,
    vars: &[String],
) -> Result<(), DecreeError> {
    let root = find_project_root()?;
    let config = Config::load(&root)?;

    let stdin_is_tty = io::stdin().is_terminal();
    let has_flags = name.is_some() || prompt.is_some() || !vars.is_empty();

    if !has_flags && stdin_is_tty {
        run_interactive(&root, &config)
    } else {
        run_noninteractive(&root, &config, name, prompt, vars, stdin_is_tty)
    }
}

// ---------------------------------------------------------------------------
// Non-interactive mode
// ---------------------------------------------------------------------------

fn run_noninteractive(
    root: &Path,
    config: &Config,
    name: Option<&str>,
    prompt: Option<&str>,
    vars: &[String],
    stdin_is_tty: bool,
) -> Result<(), DecreeError> {
    // Parse -v KEY=VALUE pairs
    let parsed_vars = parse_vars(vars)?;

    // Resolve message body
    let body = if let Some(p) = prompt {
        p.to_string()
    } else if !stdin_is_tty {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| DecreeError::Config(format!("failed to read stdin: {e}")))?;
        buf.trim().to_string()
    } else {
        String::new()
    };

    // Generate chain ID
    let chain = MessageId::new_chain(0);
    let inbox_dir = root.join(".decree/inbox");

    // Resolve message name
    let msg_name = if let Some(n) = name {
        pipeline::to_kebab_case(n)
    } else if !body.is_empty() {
        pipeline::derive_message_name(config, &body, &inbox_dir, &chain)
    } else {
        // Derive from input_file if present
        let input_file = parsed_vars
            .iter()
            .find(|(k, _)| k == "input_file")
            .map(|(_, v)| v.as_str());
        if let Some(f) = input_file {
            let stem = Path::new(f)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| format!("run-{chain}"));
            // Strip .spec suffix if present
            let stem = stem.strip_suffix(".spec").unwrap_or(&stem).to_string();
            pipeline::to_kebab_case(&stem)
        } else {
            format!("run-{chain}")
        }
    };

    // Truncate name to 50 chars
    let msg_name = if msg_name.len() > 50 {
        msg_name[..50].trim_end_matches('-').to_string()
    } else {
        msg_name
    };

    println!("creating message: {msg_name}");

    // Create inbox message
    let msg_path = pipeline::create_inbox_message(root, &msg_name, &chain, &body, &parsed_vars)?;

    // Determine spec routine if input_file points to a spec
    let spec_routine = resolve_spec_routine(root, &parsed_vars);

    // Process the message and its chain
    let result = pipeline::process_chain(root, config, &msg_path, spec_routine.as_deref())?;

    match result {
        ProcessResult::Success => {
            println!("done");
            Ok(())
        }
        ProcessResult::DeadLettered(reason) => {
            eprintln!("message dead-lettered: {reason}");
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Interactive mode
// ---------------------------------------------------------------------------

fn run_interactive(root: &Path, config: &Config) -> Result<(), DecreeError> {
    let routines = routine::discover_routines(root, config.notebook_support)?;
    if routines.is_empty() {
        return Err(DecreeError::Config(
            "no routines found in .decree/routines/ — run `decree init` first".into(),
        ));
    }

    let last_run = LastRun::load(root);

    // 1. Select routine
    let routine_names: Vec<String> = routines.iter().map(|r| r.name.clone()).collect();
    let default_routine = last_run
        .as_ref()
        .map(|lr| lr.routine.clone())
        .unwrap_or_else(|| config.default_routine.clone());
    let default_idx = routine_names
        .iter()
        .position(|n| n == &default_routine)
        .unwrap_or(0);

    let selected_routine = inquire::Select::new("Routine:", routine_names)
        .with_starting_cursor(default_idx)
        .prompt()
        .map_err(|e| DecreeError::Config(format!("prompt cancelled: {e}")))?;

    // 2. Message name
    let default_name = last_run
        .as_ref()
        .map(|lr| lr.message_name.clone())
        .unwrap_or_default();
    let msg_name_input = inquire::Text::new("Message name:")
        .with_default(&default_name)
        .prompt()
        .map_err(|e| DecreeError::Config(format!("prompt cancelled: {e}")))?;
    let msg_name = pipeline::to_kebab_case(&msg_name_input);
    if msg_name.is_empty() {
        return Err(DecreeError::Config("message name is required".into()));
    }
    let msg_name = if msg_name.len() > 50 {
        msg_name[..50].trim_end_matches('-').to_string()
    } else {
        msg_name
    };

    // 3. Input file (optional)
    let default_input = last_run
        .as_ref()
        .and_then(|lr| lr.input_file.clone())
        .unwrap_or_default();
    let input_file_input = inquire::Text::new("Input file (optional):")
        .with_default(&default_input)
        .prompt()
        .map_err(|e| DecreeError::Config(format!("prompt cancelled: {e}")))?;
    let input_file = if input_file_input.trim().is_empty() {
        None
    } else {
        Some(input_file_input.trim().to_string())
    };

    // 4. Custom parameters
    let resolved = routine::resolve_routine(root, &selected_routine, config.notebook_support)?;
    let custom_param_names = routine::discover_custom_params(&resolved)?;
    let mut custom_values: BTreeMap<String, String> = BTreeMap::new();

    let last_custom = last_run
        .as_ref()
        .map(|lr| &lr.custom)
        .cloned()
        .unwrap_or_default();

    for param_name in &custom_param_names {
        let default_val = last_custom
            .get(param_name)
            .cloned()
            .unwrap_or_default();
        let value = inquire::Text::new(&format!("{param_name}:"))
            .with_default(&default_val)
            .prompt()
            .map_err(|e| DecreeError::Config(format!("prompt cancelled: {e}")))?;
        if !value.trim().is_empty() {
            custom_values.insert(param_name.clone(), value.trim().to_string());
        }
    }

    // 5. Message body
    let body_required = input_file.is_none();
    let body_prompt = if body_required {
        "Message body (required, empty line to finish):"
    } else {
        "Message body (optional, empty line to finish):"
    };
    println!("{body_prompt}");

    let mut body_lines: Vec<String> = Vec::new();
    let stdin = io::stdin();
    loop {
        let mut line = String::new();
        let bytes = stdin
            .read_line(&mut line)
            .map_err(|e| DecreeError::Config(format!("read error: {e}")))?;
        if bytes == 0 {
            // EOF
            break;
        }
        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
        if trimmed.is_empty() && !body_lines.is_empty() {
            break;
        }
        if trimmed.is_empty() && body_lines.is_empty() {
            if body_required {
                continue; // skip leading empty lines, keep prompting
            } else {
                break; // optional, empty = skip
            }
        }
        body_lines.push(trimmed.to_string());
    }

    let body = body_lines.join("\n");
    if body_required && body.is_empty() {
        return Err(DecreeError::Config(
            "message body is required when no input_file is provided".into(),
        ));
    }

    // Build vars
    let mut vars: Vec<(String, String)> = Vec::new();
    vars.push(("routine".to_string(), selected_routine.clone()));
    if let Some(ref f) = input_file {
        vars.push(("input_file".to_string(), f.clone()));
    }
    for (k, v) in &custom_values {
        vars.push((k.clone(), v.clone()));
    }

    // Generate chain ID
    let chain = MessageId::new_chain(0);

    println!("creating message: {msg_name}");

    // Create inbox message
    let msg_path = pipeline::create_inbox_message(root, &msg_name, &chain, &body, &vars)?;

    // Determine spec routine
    let spec_routine = resolve_spec_routine(root, &vars);

    // Process the chain
    let result = pipeline::process_chain(root, config, &msg_path, spec_routine.as_deref())?;

    // Save last-run for recall
    let last_run = LastRun {
        routine: selected_routine,
        message_name: msg_name,
        input_file,
        custom: custom_values,
    };
    last_run.save(root)?;

    match result {
        ProcessResult::Success => {
            println!("done");
            Ok(())
        }
        ProcessResult::DeadLettered(reason) => {
            eprintln!("message dead-lettered: {reason}");
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse `-v KEY=VALUE` pairs into a vec of (key, value) tuples.
fn parse_vars(vars: &[String]) -> Result<Vec<(String, String)>, DecreeError> {
    let mut parsed = Vec::new();
    for v in vars {
        let (key, value) = v.split_once('=').ok_or_else(|| {
            DecreeError::Config(format!(
                "invalid -v argument '{v}': expected KEY=VALUE format"
            ))
        })?;
        parsed.push((key.to_string(), value.to_string()));
    }
    Ok(parsed)
}

/// If `input_file` points to a spec, read its frontmatter to extract the
/// routine it declares (if any).
fn resolve_spec_routine(root: &Path, vars: &[(String, String)]) -> Option<String> {
    let input_file = vars.iter().find(|(k, _)| k == "input_file").map(|(_, v)| v.as_str())?;

    if !input_file.ends_with(".spec.md") {
        return None;
    }

    let spec_path = root.join(input_file);
    let content = std::fs::read_to_string(spec_path).ok()?;
    let fm = crate::spec::parse_spec_frontmatter(&content);
    fm.routine
}
