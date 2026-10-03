use crate::config::AppConfig;
use crate::error::{DecreeError, EXIT_PRECHECK};
use crate::hooks;
use crate::message::{self, RoutineInfo};
use crate::routine::{self, RoutineDetail};
use colored::Colorize;
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
        None => {
            print_list_view(&routines);
            Ok(())
        }
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

    let detail = routine::routine_detail(project_root, config, info)?;
    print_detail_view(&detail);
    Ok(())
}

/// Print the routine list.
fn print_list_view(routines: &[RoutineInfo]) {
    for r in routines {
        if r.description.is_empty() {
            println!("  {}", r.name);
        } else {
            println!("  {:<16} {}", r.name, r.description);
        }
    }
}

/// Print one routine's detail.
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
                println!("  {:<16} {}", r.name, "PASS".green());
                pass_count += 1;
            }
            Some(reason) => {
                println!("  {:<16} {}: {}", r.name, "FAIL".red(), reason);
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
                        println!("  {:<32} {}", label, "PASS".green());
                    }
                    Some(reason) => {
                        println!("  {:<32} {}: {}", label, "FAIL".red(), reason);
                        hook_fail = true;
                    }
                }
            } else {
                println!("  {:<32} {}: routine not found", label, "FAIL".red());
                hook_fail = true;
            }
        }
    }

    if pass_count < total || hook_fail {
        std::process::exit(EXIT_PRECHECK);
    }

    Ok(())
}
