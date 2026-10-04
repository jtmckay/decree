//! Helpers shared by the test files (each `mod common;`), per the Rust Book's
//! `tests/common/mod.rs` convention.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// Write `text` to `path` as an executable script (mode 755), from a `sh` child process.
///
/// Tests run on parallel threads. Had this process written the file, a process another
/// thread forks at that moment would inherit the open write handle until it execs, and
/// running the script meanwhile fails with ETXTBSY ("Text file busy").
pub fn write_script(path: &Path, text: &str) {
    let mut sh = Command::new("sh")
        .args(["-c", r#"cat > "$1" && chmod 755 "$1""#, "sh"])
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    sh.stdin.take().unwrap().write_all(text.as_bytes()).unwrap();
    assert!(sh.wait().unwrap().success(), "{}", path.display());
}
