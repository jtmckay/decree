pub mod check;
pub mod daemon;
pub mod emit;
pub mod event;
pub mod graph;
pub mod init;
pub mod process;
pub mod prune;
pub mod retry;
pub mod schema;
pub mod status;
pub mod tail;

use crate::error::DecreeError;

pub fn help() -> Result<(), DecreeError> {
    print!("{}", include_str!("../templates/help.txt"));
    Ok(())
}
