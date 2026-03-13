use crate::config::{self, AppConfig, RoutineEntry};
use crate::error::DecreeError;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Run `decree routine-sync [--source <dir>]`.
pub fn run(project_root: &Path, source: Option<&str>) -> Result<(), DecreeError> {
    let mut config = AppConfig::load_from_project(project_root)?;

    let source_override = source.map(|s| config::expand_tilde(s));
    let changed = discover(project_root, &mut config, source_override.as_deref())?;

    if changed {
        config.save(project_root)?;
    }

    // Display results
    print_status(project_root, &config, source_override.as_deref());

    Ok(())
}

/// Run discovery and register new routines into config.
///
/// Returns `true` if config was modified and should be saved.
pub fn discover(
    project_root: &Path,
    config: &mut AppConfig,
    source_override: Option<&Path>,
) -> Result<bool, DecreeError> {
    let mut changed = false;

    // Scan project-local routines
    let routines_dir = project_root
        .join(config::DECREE_DIR)
        .join(config::ROUTINES_DIR);

    if routines_dir.is_dir() {
        let local_names = scan_routine_names(&routines_dir)?;

        // Only create/modify the section if there are routines or it already exists
        if !local_names.is_empty() || config.routines.is_some() {
            let registry = config.routines.get_or_insert_with(Default::default);

            // Add new routines (project-local default to enabled)
            for name in &local_names {
                if !registry.contains_key(name) {
                    registry.insert(name.clone(), RoutineEntry::new(true));
                    changed = true;
                }
            }

            // Mark deprecated / un-deprecate
            for (name, entry) in registry.iter_mut() {
                let exists = local_names.contains(name);
                if !exists && !entry.deprecated {
                    entry.deprecated = true;
                    changed = true;
                } else if exists && entry.deprecated {
                    entry.deprecated = false;
                    changed = true;
                }
            }
        }
    }

    // Scan shared routines
    let shared_dir = source_override
        .map(PathBuf::from)
        .or_else(|| config.resolved_routine_source());

    if let Some(ref shared_dir) = shared_dir {
        if shared_dir.is_dir() {
            let shared_names = scan_routine_names(shared_dir)?;

            if !shared_names.is_empty() || config.shared_routines.is_some() {
                let registry = config.shared_routines.get_or_insert_with(Default::default);

                // Add new shared routines (default to disabled)
                for name in &shared_names {
                    if !registry.contains_key(name) {
                        registry.insert(name.clone(), RoutineEntry::new(false));
                        changed = true;
                    }
                }

                // Mark deprecated / un-deprecate
                for (name, entry) in registry.iter_mut() {
                    let exists = shared_names.contains(name);
                    if !exists && !entry.deprecated {
                        entry.deprecated = true;
                        changed = true;
                    } else if exists && entry.deprecated {
                        entry.deprecated = false;
                        changed = true;
                    }
                }
            }
        }
    }

    Ok(changed)
}

/// Scan a directory for `.sh` files and return their names (without extension).
pub fn scan_routine_names(dir: &Path) -> Result<HashSet<String>, DecreeError> {
    let mut names = HashSet::new();

    if !dir.exists() {
        return Ok(names);
    }

    for entry in WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "sh") {
            let rel = path
                .strip_prefix(dir)
                .map_err(|e| DecreeError::Other(e.to_string()))?;
            let name = rel.with_extension("").to_string_lossy().to_string();
            if !name.is_empty() {
                names.insert(name);
            }
        }
    }

    Ok(names)
}

/// Print the routine sync status to stdout.
fn print_status(_project_root: &Path, config: &AppConfig, source_override: Option<&Path>) {
    // Project routines
    println!("Project routines (.decree/routines/):");
    if let Some(ref registry) = config.routines {
        if registry.is_empty() {
            println!("  (none)");
        } else {
            for (name, entry) in registry {
                let status = entry_status(entry);
                println!("  {:<20} {}", name, status);
            }
        }
    } else {
        println!("  (legacy mode — no registry)");
    }

    // Shared routines
    let shared_dir = source_override
        .map(PathBuf::from)
        .or_else(|| config.resolved_routine_source());

    if let Some(shared_dir) = shared_dir {
        println!();
        println!("Shared routines ({}):", shared_dir.display());
        if let Some(ref registry) = config.shared_routines {
            if registry.is_empty() {
                println!("  (none)");
            } else {
                for (name, entry) in registry {
                    let status = entry_status(entry);
                    println!("  {:<20} {}", name, status);
                }
            }
        } else {
            println!("  (none)");
        }
    }
}

/// Format the status string for a routine entry.
fn entry_status(entry: &RoutineEntry) -> &'static str {
    if entry.deprecated {
        "deprecated"
    } else if entry.enabled {
        "enabled"
    } else {
        "disabled"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_decree_dir(dir: &TempDir) {
        let decree = dir.path().join(".decree");
        std::fs::create_dir_all(decree.join("routines")).unwrap();
        std::fs::create_dir_all(decree.join("inbox")).unwrap();
        std::fs::write(
            decree.join("config.yml"),
            "commands:\n  ai_router: echo\n  ai_interactive: echo\n",
        )
        .unwrap();
    }

    #[test]
    fn test_scan_routine_names() {
        let dir = TempDir::new().unwrap();
        let routines = dir.path().join("routines");
        std::fs::create_dir_all(&routines).unwrap();
        std::fs::write(routines.join("develop.sh"), "#!/bin/bash\n").unwrap();
        std::fs::write(routines.join("deploy.sh"), "#!/bin/bash\n").unwrap();
        std::fs::write(routines.join("readme.md"), "Not a routine\n").unwrap();

        let names = scan_routine_names(&routines).unwrap();
        assert_eq!(names.len(), 2);
        assert!(names.contains("develop"));
        assert!(names.contains("deploy"));
    }

    #[test]
    fn test_scan_routine_names_empty_dir() {
        let dir = TempDir::new().unwrap();
        let routines = dir.path().join("routines");
        std::fs::create_dir_all(&routines).unwrap();

        let names = scan_routine_names(&routines).unwrap();
        assert!(names.is_empty());
    }

    #[test]
    fn test_scan_routine_names_nonexistent() {
        let dir = TempDir::new().unwrap();
        let names = scan_routine_names(&dir.path().join("nope")).unwrap();
        assert!(names.is_empty());
    }

    #[test]
    fn test_discover_project_routines() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/bin/bash\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".decree/routines/deploy.sh"),
            "#!/bin/bash\n",
        )
        .unwrap();

        let mut config = AppConfig::load_from_project(dir.path()).unwrap();
        assert!(config.routines.is_none());

        let changed = discover(dir.path(), &mut config, None).unwrap();
        assert!(changed);

        let routines = config.routines.as_ref().unwrap();
        assert_eq!(routines.len(), 2);
        assert!(routines["develop"].enabled);
        assert!(routines["deploy"].enabled);
    }

    #[test]
    fn test_discover_shared_routines() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        let shared = dir.path().join("shared-routines");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("common.sh"), "#!/bin/bash\n").unwrap();

        let mut config = AppConfig::load_from_project(dir.path()).unwrap();
        let changed = discover(dir.path(), &mut config, Some(&shared)).unwrap();
        assert!(changed);

        let shared_routines = config.shared_routines.as_ref().unwrap();
        assert_eq!(shared_routines.len(), 1);
        assert!(!shared_routines["common"].enabled); // shared default to disabled
    }

    #[test]
    fn test_discover_marks_deprecated() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        // Start with a routine registered but no file
        let mut config = AppConfig::load_from_project(dir.path()).unwrap();
        let mut routines = std::collections::BTreeMap::new();
        routines.insert("gone".to_string(), RoutineEntry::new(true));
        config.routines = Some(routines);

        let changed = discover(dir.path(), &mut config, None).unwrap();
        assert!(changed);
        assert!(config.routines.as_ref().unwrap()["gone"].deprecated);
    }

    #[test]
    fn test_discover_undeprecates() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        // File exists, but entry is deprecated
        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/bin/bash\n",
        )
        .unwrap();

        let mut config = AppConfig::load_from_project(dir.path()).unwrap();
        let mut routines = std::collections::BTreeMap::new();
        routines.insert(
            "develop".to_string(),
            RoutineEntry {
                enabled: true,
                deprecated: true,
            },
        );
        config.routines = Some(routines);

        let changed = discover(dir.path(), &mut config, None).unwrap();
        assert!(changed);
        assert!(!config.routines.as_ref().unwrap()["develop"].deprecated);
    }

    #[test]
    fn test_discover_preserves_enabled() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        std::fs::write(
            dir.path().join(".decree/routines/develop.sh"),
            "#!/bin/bash\n",
        )
        .unwrap();

        let mut config = AppConfig::load_from_project(dir.path()).unwrap();
        let mut routines = std::collections::BTreeMap::new();
        routines.insert("develop".to_string(), RoutineEntry::new(true));
        config.routines = Some(routines);

        // Running discovery again should not change anything
        let changed = discover(dir.path(), &mut config, None).unwrap();
        assert!(!changed);
        assert!(config.routines.as_ref().unwrap()["develop"].enabled);
    }

    #[test]
    fn test_discover_no_changes() {
        let dir = TempDir::new().unwrap();
        setup_decree_dir(&dir);

        let mut config = AppConfig::load_from_project(dir.path()).unwrap();
        // No routines on disk, no registry
        let changed = discover(dir.path(), &mut config, None).unwrap();
        assert!(!changed);
        assert!(config.routines.is_none());
    }

    #[test]
    fn test_entry_status() {
        assert_eq!(
            entry_status(&RoutineEntry {
                enabled: true,
                deprecated: false
            }),
            "enabled"
        );
        assert_eq!(
            entry_status(&RoutineEntry {
                enabled: false,
                deprecated: false
            }),
            "disabled"
        );
        assert_eq!(
            entry_status(&RoutineEntry {
                enabled: true,
                deprecated: true
            }),
            "deprecated"
        );
    }
}
