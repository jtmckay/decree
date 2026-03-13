use crate::config::{self, AppConfig};
use crate::error::DecreeError;
use crate::message::RoutineInfo;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Standard parameters that are not custom user parameters.
const STANDARD_PARAMS: &[&str] = &[
    "message_file",
    "message_id",
    "message_dir",
    "chain",
    "seq",
    "spec_file",
];

/// A discovered custom parameter from a routine script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomParam {
    pub name: String,
    pub default: String,
}

/// Extended routine info with full description and custom parameters.
#[derive(Debug, Clone)]
pub struct RoutineDetail {
    pub info: RoutineInfo,
    pub long_description: String,
    pub script_path: String,
    pub custom_params: Vec<CustomParam>,
}

/// Extract the short (first line) and long (full block) descriptions from a routine script.
///
/// Rules from spec 05:
/// 1. Skip the shebang (`#!/...`)
/// 2. Next comment line is the title (e.g. `# Transcribe`)
/// 3. Skip `#`-only blank comment lines
/// 4. Collect subsequent comment lines, stripping `# `
/// 5. First line is the short description (list view)
/// 6. Full block is the long description (detail view)
pub fn extract_descriptions(content: &str) -> (String, String) {
    let lines: Vec<&str> = content.lines().collect();

    let start = if lines.first().is_some_and(|l| l.starts_with("#!")) {
        1
    } else {
        0
    };

    // Skip title line
    let after_title = start + 1;
    if after_title >= lines.len() {
        return (String::new(), String::new());
    }

    // Skip blank comment lines (lines that are just `#` with optional trailing space)
    let mut desc_start = after_title;
    while desc_start < lines.len() {
        let line = lines[desc_start].trim();
        if line == "#" {
            desc_start += 1;
        } else {
            break;
        }
    }

    let mut desc_lines = Vec::new();
    for line in &lines[desc_start..] {
        if let Some(text) = line.strip_prefix("# ") {
            desc_lines.push(text.to_string());
        } else {
            break;
        }
    }

    let short = desc_lines.first().cloned().unwrap_or_default();
    let long = desc_lines.join("\n");
    (short, long)
}

/// Discover custom parameters from a routine script.
///
/// Rules from spec 05:
/// 1. Skip shebang, comments, blanks, `set` builtins, pre-check block
/// 2. Match `var="${var:-default}"` assignments
/// 3. Stop at first non-matching line
/// 4. Exclude standard params (message_file, message_id, etc.)
/// 5. Remainder are custom parameters with defaults from `:-default`
pub fn discover_custom_params(content: &str) -> Vec<CustomParam> {
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    // Phase 1: Skip shebang, comments, blanks, `set` builtins, standard params, and pre-check block
    let mut in_precheck = false;
    while i < lines.len() {
        let trimmed = lines[i].trim();

        if trimmed.starts_with("#!") || trimmed.starts_with('#') || trimmed.is_empty() {
            i += 1;
            continue;
        }

        if trimmed.starts_with("set ") {
            i += 1;
            continue;
        }

        // Detect pre-check block start
        if trimmed.contains("DECREE_PRE_CHECK") {
            in_precheck = true;
            i += 1;
            continue;
        }

        if in_precheck {
            if trimmed == "fi" {
                in_precheck = false;
            }
            i += 1;
            continue;
        }

        // Skip standard parameter assignments
        if let Some(param) = parse_param_assignment(trimmed) {
            if STANDARD_PARAMS.contains(&param.name.as_str()) {
                i += 1;
                continue;
            }
        }

        // Hit a non-skippable line — start looking for param assignments
        break;
    }

    // Phase 2: Collect param assignments of the form var="${var:-default}"
    let mut params = Vec::new();
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if let Some(param) = parse_param_assignment(trimmed) {
            if !STANDARD_PARAMS.contains(&param.name.as_str()) {
                params.push(param);
            }
            i += 1;
        } else {
            break;
        }
    }

    params
}

/// Parse a line like `var="${var:-default}"` into a CustomParam.
fn parse_param_assignment(line: &str) -> Option<CustomParam> {
    // Match: name="${name:-default}" or name="${name:-}"
    let (name, rest) = line.split_once('=')?;
    let name = name.trim();

    // Validate name is a valid shell identifier
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return None;
    }

    let rest = rest.trim();

    // Must be quoted: "..." or '...'
    let inner = if rest.starts_with('"') && rest.ends_with('"') && rest.len() >= 2 {
        &rest[1..rest.len() - 1]
    } else if rest.starts_with('\'') && rest.ends_with('\'') && rest.len() >= 2 {
        &rest[1..rest.len() - 1]
    } else {
        return None;
    };

    // Must be ${name:-...}
    let expected_prefix = format!("${{{name}:-");
    if !inner.starts_with(&expected_prefix) || !inner.ends_with('}') {
        return None;
    }

    let default = &inner[expected_prefix.len()..inner.len() - 1];

    Some(CustomParam {
        name: name.to_string(),
        default: default.to_string(),
    })
}

/// Build the full detail for a routine, including description and custom params.
pub fn routine_detail(
    project_root: &Path,
    config: &AppConfig,
    info: &RoutineInfo,
) -> Result<RoutineDetail, DecreeError> {
    let script_path = find_routine_script_layered(project_root, config, &info.name)?;
    let content = std::fs::read_to_string(&script_path)?;

    let (_, long_description) = extract_descriptions(&content);
    let custom_params = discover_custom_params(&content);

    Ok(RoutineDetail {
        info: info.clone(),
        long_description,
        script_path: script_path.to_string_lossy().to_string(),
        custom_params,
    })
}

/// Find the actual script file for a routine name (tries .sh extension).
pub fn find_routine_script(
    routines_dir: &Path,
    name: &str,
) -> Result<std::path::PathBuf, DecreeError> {
    let with_sh = routines_dir.join(format!("{name}.sh"));
    if with_sh.is_file() {
        return Ok(with_sh);
    }

    // Try without extension (unlikely but possible)
    let bare = routines_dir.join(name);
    if bare.is_file() {
        return Ok(bare);
    }

    Err(DecreeError::RoutineNotFound(name.to_string()))
}

/// Find a routine script checking project-local first, then shared directory.
/// Does NOT check the registry — suitable for hooks which bypass the registry.
pub fn find_routine_script_layered(
    project_root: &Path,
    config: &AppConfig,
    name: &str,
) -> Result<PathBuf, DecreeError> {
    let project_dir = project_root
        .join(config::DECREE_DIR)
        .join(config::ROUTINES_DIR);

    if let Ok(path) = find_routine_script(&project_dir, name) {
        return Ok(path);
    }

    if let Some(shared_dir) = config.resolved_routine_source() {
        if let Ok(path) = find_routine_script(&shared_dir, name) {
            return Ok(path);
        }
    }

    Err(DecreeError::RoutineNotFound(name.to_string()))
}

/// Resolve a routine with registry check and layered directory lookup.
///
/// Returns error if the routine is disabled or not found.
/// Project-local `.decree/routines/` takes precedence over the shared directory.
pub fn resolve_routine(
    project_root: &Path,
    config: &AppConfig,
    name: &str,
) -> Result<PathBuf, DecreeError> {
    let project_dir = project_root
        .join(config::DECREE_DIR)
        .join(config::ROUTINES_DIR);

    // Check project-local
    if let Ok(path) = find_routine_script(&project_dir, name) {
        if let Some(ref routines) = config.routines {
            // Strict mode: must be registered and active
            match routines.get(name) {
                Some(entry) if entry.is_active() => return Ok(path),
                Some(_) => return Err(DecreeError::RoutineDisabled(name.to_string())),
                None => {} // Not in project registry — check shared
            }
        } else {
            // Legacy mode: all filesystem routines available
            return Ok(path);
        }
    }

    // Check shared directory
    if let Some(shared_dir) = config.resolved_routine_source() {
        if let Ok(path) = find_routine_script(&shared_dir, name) {
            // shared_routines is always strict
            if let Some(ref shared) = config.shared_routines {
                match shared.get(name) {
                    Some(entry) if entry.is_active() => return Ok(path),
                    Some(_) => return Err(DecreeError::RoutineDisabled(name.to_string())),
                    None => {}
                }
            }
        }
    }

    // Determine best error: disabled vs not found
    let is_disabled = config
        .routines
        .as_ref()
        .and_then(|r| r.get(name))
        .is_some_and(|e| !e.is_active())
        || config
            .shared_routines
            .as_ref()
            .and_then(|r| r.get(name))
            .is_some_and(|e| !e.is_active());

    if is_disabled {
        Err(DecreeError::RoutineDisabled(name.to_string()))
    } else {
        Err(DecreeError::RoutineNotFound(name.to_string()))
    }
}

/// Run the pre-check for a routine by executing it with DECREE_PRE_CHECK=true.
///
/// Returns Ok(None) on success, Ok(Some(reason)) on failure.
pub fn run_precheck(
    project_root: &Path,
    config: &AppConfig,
    routine_name: &str,
) -> Result<Option<String>, DecreeError> {
    let script_path = find_routine_script_layered(project_root, config, routine_name)?;

    let output = Command::new("bash")
        .arg(&script_path)
        .env("DECREE_PRE_CHECK", "true")
        .current_dir(project_root)
        .output()
        .map_err(|e| DecreeError::Io(e))?;

    if output.status.success() {
        Ok(None)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let reason = if stderr.is_empty() {
            "pre-check exited with non-zero status".to_string()
        } else {
            stderr
        };
        Ok(Some(reason))
    }
}

/// Compute Levenshtein distance between two strings.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a_len = a.len();
    let b_len = b.len();
    let mut matrix = vec![vec![0usize; b_len + 1]; a_len + 1];

    for i in 0..=a_len {
        matrix[i][0] = i;
    }
    for j in 0..=b_len {
        matrix[0][j] = j;
    }

    for (i, ac) in a.chars().enumerate() {
        for (j, bc) in b.chars().enumerate() {
            let cost = if ac == bc { 0 } else { 1 };
            matrix[i + 1][j + 1] = (matrix[i][j + 1] + 1)
                .min(matrix[i + 1][j] + 1)
                .min(matrix[i][j] + cost);
        }
    }

    matrix[a_len][b_len]
}

/// Find the closest matching routine name within a Levenshtein distance threshold.
pub fn find_closest_routine(name: &str, routines: &[RoutineInfo], max_distance: usize) -> Option<String> {
    let mut best: Option<(usize, String)> = None;

    for r in routines {
        let dist = levenshtein(name, &r.name);
        if dist <= max_distance {
            if best.as_ref().is_none_or(|(d, _)| dist < *d) {
                best = Some((dist, r.name.clone()));
            }
        }
    }

    best.map(|(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_descriptions_standard() {
        let content = r#"#!/usr/bin/env bash
# Transcribe Audio
#
# Transcribes audio files using OpenAI Whisper.
# Supports multiple formats and output types.
set -euo pipefail
"#;
        let (short, long) = extract_descriptions(content);
        assert_eq!(short, "Transcribes audio files using OpenAI Whisper.");
        assert_eq!(
            long,
            "Transcribes audio files using OpenAI Whisper.\nSupports multiple formats and output types."
        );
    }

    #[test]
    fn test_extract_descriptions_no_shebang() {
        let content = "# Title\n#\n# Description line.\n";
        let (short, _) = extract_descriptions(content);
        assert_eq!(short, "Description line.");
    }

    #[test]
    fn test_extract_descriptions_empty() {
        let content = "#!/usr/bin/env bash\n# Title\n";
        let (short, long) = extract_descriptions(content);
        assert_eq!(short, "");
        assert_eq!(long, "");
    }

    #[test]
    fn test_discover_custom_params_basic() {
        let content = r#"#!/usr/bin/env bash
# Test
#
# A test routine.
set -euo pipefail

message_file="${message_file:-}"
message_id="${message_id:-}"

# Pre-check
if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    exit 0
fi

output_file="${output_file:-}"
model="${model:-large}"
"#;
        let params = discover_custom_params(content);
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "output_file");
        assert_eq!(params[0].default, "");
        assert_eq!(params[1].name, "model");
        assert_eq!(params[1].default, "large");
    }

    #[test]
    fn test_discover_custom_params_excludes_standard() {
        let content = r#"#!/usr/bin/env bash
# Test
set -euo pipefail

message_file="${message_file:-}"
chain="${chain:-}"
seq="${seq:-}"

if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    exit 0
fi

custom_one="${custom_one:-default_val}"
"#;
        let params = discover_custom_params(content);
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "custom_one");
        assert_eq!(params[0].default, "default_val");
    }

    #[test]
    fn test_discover_custom_params_stops_at_non_matching() {
        let content = r#"#!/usr/bin/env bash
# Test

if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    exit 0
fi

foo="${foo:-bar}"
baz="${baz:-}"
echo "hello"
qux="${qux:-quux}"
"#;
        let params = discover_custom_params(content);
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "foo");
        assert_eq!(params[1].name, "baz");
    }

    #[test]
    fn test_parse_param_assignment() {
        assert_eq!(
            parse_param_assignment(r#"foo="${foo:-bar}""#),
            Some(CustomParam {
                name: "foo".to_string(),
                default: "bar".to_string(),
            })
        );
        assert_eq!(
            parse_param_assignment(r#"foo="${foo:-}""#),
            Some(CustomParam {
                name: "foo".to_string(),
                default: "".to_string(),
            })
        );
        assert_eq!(parse_param_assignment("echo hello"), None);
        assert_eq!(parse_param_assignment(""), None);
    }

    #[test]
    fn test_levenshtein() {
        assert_eq!(levenshtein("develop", "develop"), 0);
        assert_eq!(levenshtein("devlop", "develop"), 1);
        assert_eq!(levenshtein("foo", "bar"), 3);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
    }

    #[test]
    fn test_find_closest_routine() {
        let routines = vec![
            RoutineInfo {
                name: "develop".to_string(),
                description: "Dev".to_string(),
            },
            RoutineInfo {
                name: "rust-develop".to_string(),
                description: "Rust".to_string(),
            },
        ];

        assert_eq!(
            find_closest_routine("devlop", &routines, 3),
            Some("develop".to_string())
        );
        assert_eq!(find_closest_routine("foo", &routines, 3), None);
        assert_eq!(
            find_closest_routine("develop", &routines, 3),
            Some("develop".to_string())
        );
    }
}
