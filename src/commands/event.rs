//! `decree event <wait id | run id> <event> [-m <note>]` (spec section 8).

use std::path::Path;

use crate::error::DecreeError;

pub fn run(
    _project_root: &Path,
    _target: &str,
    _event: &str,
    _note: Option<&str>,
) -> Result<(), DecreeError> {
    Err(DecreeError::Other("not implemented yet".into()))
}
