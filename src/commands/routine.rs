use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;

use dialoguer::{Confirm, FuzzySelect, Input};
use serde_yaml::Value;

use crate::error::{DecreeError, Result};
use crate::message::{generate_chain_id, Message, MessageType};
use crate::routine::{discover_routines, Routine};

/// Run the `decree routine` command.
pub fn run(name: Option<&str>) -> Result<()> {
    let routines_dir = Path::new(".decree/routines");
    let routines = discover_routines(routines_dir)?;

    if routines.is_empty() {
        eprintln!("No routines found in .decree/routines/");
        return Ok(());
    }

    let is_tty = io::stdin().is_terminal();

    match name {
        Some(name) => {
            let routine = match routines.iter().find(|r| r.name == name) {
                Some(r) => r.clone(),
                None => {
                    eprintln!("\nAvailable routines:");
                    let name_width = calc_name_width(&routines);
                    for r in &routines {
                        eprintln!(
                            "  {:<width$} {}",
                            r.name,
                            r.short_description,
                            width = name_width
                        );
                    }
                    return Err(DecreeError::RoutineNotFound(name.to_string()));
                }
            };

            if !is_tty {
                print_detail(&routine);
                return Ok(());
            }

            guided_flow(&routine)
        }
        None => {
            if !is_tty {
                let name_width = calc_name_width(&routines);
                for r in &routines {
                    println!(
                        "  {:<width$} {}",
                        r.name,
                        r.short_description,
                        width = name_width
                    );
                }
                return Ok(());
            }

            let routine = select_routine(&routines)?;
            guided_flow(&routine)
        }
    }
}

/// Calculate the column width for routine names.
fn calc_name_width(routines: &[Routine]) -> usize {
    routines
        .iter()
        .map(|r| r.name.len())
        .max()
        .unwrap_or(0)
        .max(14)
        + 2
}

/// Extract the first line of a string (for clean error display).
fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}

/// Print the detail view for a routine (non-TTY mode).
fn print_detail(routine: &Routine) {
    println!("{} ({})", routine.name, routine.path.display());
    println!();
    if !routine.description.is_empty() {
        for line in routine.description.lines() {
            println!("  {line}");
        }
        println!();
    }

    match routine.run_pre_check() {
        Ok(()) => println!("  Pre-check: PASS"),
        Err(DecreeError::PreCheckFailed { reason, .. }) => {
            println!("  Pre-check: FAIL: {}", first_line(&reason));
        }
        Err(e) => {
            let msg = e.to_string();
            println!("  Pre-check: FAIL: {}", first_line(&msg));
        }
    }
}

/// Step 1: Select routine using fuzzy finder.
fn select_routine(routines: &[Routine]) -> Result<Routine> {
    let default_routine = load_default_routine();
    let name_width = calc_name_width(routines);

    let items: Vec<String> = routines
        .iter()
        .map(|r| {
            format!(
                "{:<width$} {}",
                r.name,
                r.short_description,
                width = name_width
            )
        })
        .collect();

    let default_idx = routines
        .iter()
        .position(|r| r.name == default_routine)
        .unwrap_or(0);

    let selection = FuzzySelect::new()
        .with_prompt("Select a routine")
        .items(&items)
        .default(default_idx)
        .interact()
        .map_err(|e| DecreeError::Config(format!("selection failed: {e}")))?;

    Ok(routines[selection].clone())
}

/// Show routine header: name, path, description.
fn show_routine_header(routine: &Routine) {
    println!("{} ({})", routine.name, routine.path.display());
    println!();
    if !routine.description.is_empty() {
        for line in routine.description.lines() {
            println!("  {line}");
        }
        println!();
    }
}

/// Run pre-check and display the result.
/// Returns Ok(true) if passed, Ok(false) if failed.
fn show_precheck(routine: &Routine) -> Result<bool> {
    match routine.run_pre_check() {
        Ok(()) => {
            println!("  Pre-check: PASS");
            Ok(true)
        }
        Err(DecreeError::PreCheckFailed { reason, .. }) => {
            println!("  Pre-check: FAIL: {}", first_line(&reason));
            Ok(false)
        }
        Err(e) => {
            let msg = e.to_string();
            println!("  Pre-check: FAIL: {}", first_line(&msg));
            Ok(false)
        }
    }
}

/// Steps 2–6: guided interactive flow.
fn guided_flow(routine: &Routine) -> Result<()> {
    // Step 2: Show description and pre-check
    show_routine_header(routine);
    let passed = show_precheck(routine)?;

    if !passed {
        println!();
        let cont = Confirm::new()
            .with_prompt("  Continue anyway?")
            .default(false)
            .interact()
            .map_err(|e| DecreeError::Config(format!("prompt failed: {e}")))?;
        if !cont {
            return Ok(());
        }
    }

    println!();

    // Step 3: Prompt for input file
    let input_file: String = Input::new()
        .with_prompt("Input file [optional]")
        .allow_empty(true)
        .default(String::new())
        .show_default(false)
        .interact_text()
        .map_err(|e| DecreeError::Config(format!("prompt failed: {e}")))?;
    let input_file = if input_file.trim().is_empty() {
        None
    } else {
        Some(input_file)
    };

    // Step 4: Prompt for custom parameters
    let mut custom_values: BTreeMap<String, String> = BTreeMap::new();
    for param in &routine.custom_params {
        let prompt = format!("{} [default: \"{}\"]", param.name, param.default);
        let value: String = Input::new()
            .with_prompt(&prompt)
            .default(param.default.clone())
            .show_default(false)
            .allow_empty(true)
            .interact_text()
            .map_err(|e| DecreeError::Config(format!("prompt failed: {e}")))?;
        custom_values.insert(param.name.clone(), value);
    }

    // Step 5: Message body
    let hint = if input_file.is_some() {
        "optional"
    } else {
        "recommended"
    };
    eprintln!("Message body [{hint}, empty line to submit]:");
    let body = read_multiline_body()?;

    // Step 6: Summary and execute
    println!();
    println!("Running {}:", routine.name);
    if let Some(ref f) = input_file {
        println!("  input_file: {f}");
    }
    for (name, value) in &custom_values {
        if !value.is_empty() {
            println!("  {name}: {value}");
        }
    }
    if !body.is_empty() {
        println!("  body: \"{body}\"");
    }
    println!();

    eprint!("Press Enter to run, Ctrl-C to cancel.");
    io::stderr().flush()?;

    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;

    execute_routine(routine, input_file.as_deref(), &custom_values, &body)
}

/// Read multi-line body from stdin, terminated by an empty line.
fn read_multiline_body() -> Result<String> {
    let stdin = io::stdin();
    let mut lines = Vec::new();

    for line_result in stdin.lock().lines() {
        let line = line_result?;
        if line.is_empty() {
            break;
        }
        lines.push(line);
    }

    Ok(lines.join("\n"))
}

/// Create inbox message and execute the routine.
fn execute_routine(
    routine: &Routine,
    input_file: Option<&str>,
    custom_values: &BTreeMap<String, String>,
    body: &str,
) -> Result<()> {
    let chain = generate_chain_id();
    let seq: u32 = 0;
    let id = format!("{chain}-{seq}");

    let mut custom_fields = BTreeMap::new();
    for (name, value) in custom_values {
        custom_fields.insert(name.clone(), Value::String(value.clone()));
    }

    let message_type = if input_file.is_some() {
        MessageType::Spec
    } else {
        MessageType::Task
    };

    let inbox_dir = Path::new(".decree/inbox");
    let message_path = inbox_dir.join(format!("{id}.md"));

    let message = Message {
        id: id.clone(),
        chain,
        seq,
        message_type,
        input_file: input_file.map(|s| s.to_string()),
        routine: routine.name.clone(),
        custom_fields,
        body: body.to_string(),
        path: message_path,
    };

    // Write message to inbox
    message.write()?;

    // Create run directory
    let run_dir = Path::new(".decree/runs").join(&id);
    fs::create_dir_all(&run_dir)?;

    // Copy message to run directory
    fs::write(run_dir.join("message.md"), message.to_string())?;

    // Execute routine with inherited stdio
    let mut cmd = routine.build_command(&message, &run_dir);
    let status = cmd.status()?;

    // Move message to done
    let done_dir = inbox_dir.join("done");
    fs::create_dir_all(&done_dir)?;
    let done_path = done_dir.join(format!("{id}.md"));
    if message.path.exists() {
        fs::rename(&message.path, &done_path)?;
    }

    if !status.success() {
        eprintln!(
            "Routine '{}' exited with status: {}",
            routine.name, status
        );
    }

    Ok(())
}

/// Load the default_routine from config, falling back to "develop".
fn load_default_routine() -> String {
    load_default_routine_from(Path::new(".decree/config.yml"))
}

fn load_default_routine_from(path: &Path) -> String {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return "develop".to_string(),
    };
    let value: serde_yaml::Value = match serde_yaml::from_str(&content) {
        Ok(v) => v,
        Err(_) => return "develop".to_string(),
    };
    value
        .get("default_routine")
        .and_then(|v| v.as_str())
        .unwrap_or("develop")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_default_routine_missing_file() {
        let result = load_default_routine_from(Path::new("/nonexistent/config.yml"));
        assert_eq!(result, "develop");
    }

    #[test]
    fn test_load_default_routine_from_yaml() {
        let tmp = std::env::temp_dir().join("decree_test_routine_config");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let config_path = tmp.join("config.yml");
        fs::write(&config_path, "default_routine: rust-develop\n").unwrap();

        let result = load_default_routine_from(&config_path);
        assert_eq!(result, "rust-develop");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_load_default_routine_invalid_yaml() {
        let tmp = std::env::temp_dir().join("decree_test_routine_config_bad");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let config_path = tmp.join("config.yml");
        fs::write(&config_path, "{{{{not yaml").unwrap();

        let result = load_default_routine_from(&config_path);
        assert_eq!(result, "develop");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_load_default_routine_missing_field() {
        let tmp = std::env::temp_dir().join("decree_test_routine_config_no_field");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let config_path = tmp.join("config.yml");
        fs::write(&config_path, "max_retries: 3\n").unwrap();

        let result = load_default_routine_from(&config_path);
        assert_eq!(result, "develop");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_calc_name_width() {
        use crate::routine::Routine;
        use std::path::PathBuf;

        let routines = vec![
            Routine {
                name: "develop".to_string(),
                path: PathBuf::from(".decree/routines/develop.sh"),
                title: "Develop".to_string(),
                short_description: "Default routine.".to_string(),
                description: "Default routine.".to_string(),
                custom_params: vec![],
            },
            Routine {
                name: "deploy/staging".to_string(),
                path: PathBuf::from(".decree/routines/deploy/staging.sh"),
                title: "Deploy Staging".to_string(),
                short_description: "Deploy to staging.".to_string(),
                description: "Deploy to staging.".to_string(),
                custom_params: vec![],
            },
        ];

        let width = calc_name_width(&routines);
        // "deploy/staging" is 14 chars, min is 14, + 2 = 16
        assert_eq!(width, 16);
    }

    #[test]
    fn test_calc_name_width_long_name() {
        use crate::routine::Routine;
        use std::path::PathBuf;

        let routines = vec![Routine {
            name: "very-long-routine-name".to_string(),
            path: PathBuf::from("test.sh"),
            title: "Test".to_string(),
            short_description: "Test.".to_string(),
            description: "Test.".to_string(),
            custom_params: vec![],
        }];

        let width = calc_name_width(&routines);
        // "very-long-routine-name" is 22 chars, + 2 = 24
        assert_eq!(width, 24);
    }
}
