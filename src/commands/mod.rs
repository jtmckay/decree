pub mod check;
pub mod daemon;
pub mod emit;
pub mod event;
pub mod graph;
pub mod init;
pub mod process;
pub mod prune;
pub(crate) mod report;
pub mod schema;
pub mod skill;
pub mod status;
pub mod tail;

use crate::error::DecreeError;

pub fn help() -> Result<(), DecreeError> {
    print!("{}", include_str!("../templates/help.txt"));
    Ok(())
}

/// Print `value` as one JSON document on stdout (docs/reference/cli.md, Machine-readable
/// output).
pub(crate) fn print_json(value: &serde_json::Value) -> Result<(), DecreeError> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| DecreeError::Other(format!("cannot write JSON: {e}")))?;
    println!("{text}");
    Ok(())
}
