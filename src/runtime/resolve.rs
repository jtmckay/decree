//! Script resolution (docs/reference/scripts.md, Resolution): a script name used by a machine
//! resolves to exactly one executable file. Validation (V12) and the executor both call
//! `resolve_script`, so a machine that passes `decree check` runs the same files.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::machine::is_ident;

/// Directory holding scripts, relative to `.decree/`.
pub use crate::layout::SCRIPTS_DIR;

/// Why a script name does not resolve to exactly one executable file (V12).
#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    #[error("script name `{0}` does not match ^[a-z][a-z0-9_]*$")]
    InvalidName(String),

    #[error("script `{name}` not found; searched {}", join_paths(searched))]
    Missing {
        name: String,
        searched: Vec<PathBuf>,
    },

    #[error("script `{name}` is ambiguous: {}", join_paths(matches))]
    Ambiguous { name: String, matches: Vec<PathBuf> },

    #[error("script `{name}`: {} is not a regular file", path.display())]
    NotRegularFile { name: String, path: PathBuf },

    #[error("script `{name}`: {} is not executable", path.display())]
    NotExecutable { name: String, path: PathBuf },

    #[error("script `{name}`: cannot read {}: {source}", dir.display())]
    Io {
        name: String,
        dir: PathBuf,
        source: io::Error,
    },
}

fn join_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The directories script `name` is looked up in for `machine`: only its own
/// `machines/<machine>/scripts/`. A script another machine ships is reached by path, from a
/// script, never by name.
fn search_dirs(decree_dir: &Path, machine: &str) -> Vec<PathBuf> {
    vec![crate::layout::machine_dir(decree_dir, machine).join(SCRIPTS_DIR)]
}

/// Resolve script `name` used by `machine` (docs/reference/scripts.md, Resolution). The first directory from
/// `search_dirs` holding a match wins; a match is a file named `name` or `name.<ext>` with one
/// extension. The winner must be the only match in its directory, a regular file, and have
/// an execute bit set. Lower directories are not read once a match is found.
pub fn resolve_script(
    decree_dir: &Path,
    machine: &str,
    name: &str,
) -> Result<PathBuf, ScriptError> {
    if !is_ident(name) {
        return Err(ScriptError::InvalidName(name.to_string()));
    }
    let searched = search_dirs(decree_dir, machine);
    for dir in &searched {
        let mut matches = matches_in(dir, name).map_err(|source| ScriptError::Io {
            name: name.to_string(),
            dir: dir.clone(),
            source,
        })?;
        match matches.len() {
            0 => continue,
            1 => return check_executable(name, matches.remove(0)),
            _ => {
                return Err(ScriptError::Ambiguous {
                    name: name.to_string(),
                    matches,
                })
            }
        }
    }
    Err(ScriptError::Missing {
        name: name.to_string(),
        searched,
    })
}

/// Entries of `dir` named `name` or `name.<ext>`, sorted. Directories are skipped: they are
/// per-machine script directories, never scripts. A missing `dir` holds no matches, and so
/// does a `dir` that is not a directory: `scripts/<machine>` may be a script of that name.
fn matches_in(dir: &Path, name: &str) -> io::Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(Vec::new())
        }
        Err(e) => return Err(e),
    };
    let mut matches = Vec::new();
    for entry in entries {
        let path = entry?.path();
        let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if is_match(file_name, name) && !path.is_dir() {
            matches.push(path);
        }
    }
    matches.sort();
    Ok(matches)
}

/// `file_name` is `name`, or `name.<ext>` where `<ext>` is non-empty and has no dot.
fn is_match(file_name: &str, name: &str) -> bool {
    match file_name.strip_prefix(name) {
        Some("") => true,
        Some(rest) => rest
            .strip_prefix('.')
            .is_some_and(|ext| !ext.is_empty() && !ext.contains('.')),
        None => false,
    }
}

/// The single match must be a regular file (symlinks followed) with `mode & 0o111` non-zero.
fn check_executable(name: &str, path: PathBuf) -> Result<PathBuf, ScriptError> {
    let meta = match fs::metadata(&path) {
        Ok(meta) if meta.is_file() => meta,
        _ => {
            return Err(ScriptError::NotRegularFile {
                name: name.to_string(),
                path,
            })
        }
    };
    if meta.permissions().mode() & 0o111 == 0 {
        return Err(ScriptError::NotExecutable {
            name: name.to_string(),
            path,
        });
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A temp directory with an empty `project/.decree/`.
    struct Fixture {
        tmp: TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let tmp = TempDir::new().unwrap();
            fs::create_dir_all(tmp.path().join("project/.decree")).unwrap();
            Fixture { tmp }
        }

        fn decree_dir(&self) -> PathBuf {
            self.tmp.path().join("project/.decree")
        }

        /// Write a script at `rel` (relative to the temp root) with the given mode.
        fn script(&self, rel: &str, mode: u32) -> PathBuf {
            let path = self.tmp.path().join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "#!/usr/bin/env bash\nexit 0\n").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            path
        }

        fn resolve(&self, machine: &str, name: &str) -> Result<PathBuf, ScriptError> {
            resolve_script(&self.decree_dir(), machine, name)
        }
    }

    #[test]
    fn exact_name_resolves() {
        let f = Fixture::new();
        let x = f.script("project/.decree/machines/m/scripts/x", 0o755);
        assert_eq!(f.resolve("m", "x").unwrap(), x);
    }

    #[test]
    fn one_extension_resolves() {
        let f = Fixture::new();
        let x = f.script("project/.decree/machines/m/scripts/x.sh", 0o755);
        assert_eq!(f.resolve("m", "x").unwrap(), x);
    }

    #[test]
    fn another_machines_script_is_not_found() {
        let f = Fixture::new();
        f.script("project/.decree/machines/n/scripts/x.sh", 0o755);
        assert!(matches!(
            f.resolve("m", "x"),
            Err(ScriptError::Missing { .. })
        ));
    }

    #[test]
    fn two_matches_in_one_dir_fail_naming_both() {
        let f = Fixture::new();
        let sh = f.script("project/.decree/machines/m/scripts/x.sh", 0o755);
        let py = f.script("project/.decree/machines/m/scripts/x.py", 0o755);
        let err = f.resolve("m", "x").unwrap_err();
        match &err {
            ScriptError::Ambiguous { matches, .. } => assert_eq!(matches, &vec![py, sh]),
            other => panic!("expected Ambiguous, got {other:?}"),
        }
        let msg = err.to_string();
        assert!(msg.contains("x.sh") && msg.contains("x.py"), "{msg}");
    }

    #[test]
    fn not_executable_fails() {
        let f = Fixture::new();
        let x = f.script("project/.decree/machines/m/scripts/x.sh", 0o644);
        match f.resolve("m", "x").unwrap_err() {
            ScriptError::NotExecutable { path, .. } => assert_eq!(path, x),
            other => panic!("expected NotExecutable, got {other:?}"),
        }
    }

    #[test]
    fn any_execute_bit_is_enough() {
        let f = Fixture::new();
        let x = f.script("project/.decree/machines/m/scripts/x", 0o640 | 0o001);
        assert_eq!(f.resolve("m", "x").unwrap(), x);
    }

    #[test]
    fn missing_fails_listing_searched_dirs() {
        let f = Fixture::new();
        f.script("project/.decree/machines/m/scripts/y.sh", 0o755);
        let err = f.resolve("m", "x").unwrap_err();
        match &err {
            ScriptError::Missing { searched, .. } => {
                assert_eq!(searched, &search_dirs(&f.decree_dir(), "m"));
                assert_eq!(searched.len(), 1);
            }
            other => panic!("expected Missing, got {other:?}"),
        }
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn similar_names_do_not_match() {
        let f = Fixture::new();
        f.script("project/.decree/machines/m/scripts/xy.sh", 0o755);
        f.script("project/.decree/machines/m/scripts/x.tar.gz", 0o755);
        f.script("project/.decree/machines/m/scripts/x.", 0o755);
        f.script("project/.decree/machines/m/scripts/.x", 0o755);
        assert!(matches!(
            f.resolve("m", "x"),
            Err(ScriptError::Missing { .. })
        ));
    }

    #[test]
    fn non_regular_file_fails() {
        let f = Fixture::new();
        let dir = f.decree_dir().join("machines/m/scripts");
        fs::create_dir_all(&dir).unwrap();
        let link = dir.join("x");
        std::os::unix::fs::symlink(dir.join("nowhere"), &link).unwrap();
        match f.resolve("m", "x").unwrap_err() {
            ScriptError::NotRegularFile { path, .. } => assert_eq!(path, link),
            other => panic!("expected NotRegularFile, got {other:?}"),
        }
    }

    #[test]
    fn symlink_to_executable_resolves() {
        let f = Fixture::new();
        let target = f.script("elsewhere/real.sh", 0o755);
        let dir = f.decree_dir().join("machines/m/scripts");
        fs::create_dir_all(&dir).unwrap();
        let link = dir.join("x.sh");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert_eq!(f.resolve("m", "x").unwrap(), link);
    }

    #[test]
    fn invalid_names_fail() {
        let f = Fixture::new();
        f.script("project/.decree/machines/m/scripts/X.sh", 0o755);
        for name in ["", "X", "../x", "m/x", "x.sh", "1x", "x-y"] {
            assert!(
                matches!(f.resolve("m", name), Err(ScriptError::InvalidName(_))),
                "{name:?}"
            );
        }
    }
}
