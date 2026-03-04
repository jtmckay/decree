use std::path::Path;

use crate::error::{DecreeError, Result};
use crate::routine::discover_routines;

/// Run the `decree verify` command.
///
/// Returns `true` if all pre-checks pass, `false` if any fail.
pub fn run() -> Result<bool> {
    let routines_dir = Path::new(".decree/routines");
    let routines = discover_routines(routines_dir)?;

    if routines.is_empty() {
        eprintln!("No routines found in .decree/routines/");
        return Ok(true);
    }

    let name_width = routines
        .iter()
        .map(|r| r.name.len())
        .max()
        .unwrap_or(0)
        .max(14)
        + 2;

    println!();
    println!("Routine pre-checks:");

    let mut pass_count: u32 = 0;
    let mut fail_count: u32 = 0;

    for routine in &routines {
        match routine.run_pre_check() {
            Ok(()) => {
                println!(
                    "  {:<width$} PASS",
                    routine.name,
                    width = name_width
                );
                pass_count += 1;
            }
            Err(DecreeError::PreCheckFailed { reason, .. }) => {
                let reason = reason.lines().next().unwrap_or(&reason);
                println!(
                    "  {:<width$} FAIL: {reason}",
                    routine.name,
                    width = name_width
                );
                fail_count += 1;
            }
            Err(e) => {
                let msg = e.to_string();
                let msg = msg.lines().next().unwrap_or(&msg);
                println!(
                    "  {:<width$} FAIL: {msg}",
                    routine.name,
                    width = name_width
                );
                fail_count += 1;
            }
        }
    }

    let total = pass_count + fail_count;
    println!();
    println!("{pass_count} of {total} routines ready.");

    Ok(fail_count == 0)
}
