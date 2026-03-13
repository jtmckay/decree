pub mod daemon;
pub mod init;
pub mod log;
pub mod process;
pub mod prompt;
pub mod routine;
pub mod routine_sync;
pub mod status;

use crate::error::DecreeError;

pub fn help() -> Result<(), DecreeError> {
    print!("{}", include_str!("../templates/help.txt"));
    Ok(())
}
