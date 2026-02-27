use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::DecreeError;
use crate::message::InboxMessage;

/// Standard parameters injected into every routine.
pub const STANDARD_PARAMS: &[&str] = &[
    "spec_file",
    "message_file",
    "message_id",
    "message_dir",
    "chain",
    "seq",
];

/// The format of a routine file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutineFormat {
    Shell,
    Notebook,
}

/// A resolved routine ready for execution.
#[derive(Debug, Clone)]
pub struct ResolvedRoutine {
    pub name: String,
    pub path: PathBuf,
    pub format: RoutineFormat,
}

/// A discovered routine with its name and description.
#[derive(Debug, Clone)]
pub struct RoutineInfo {
    pub name: String,
    pub description: String,
}

/// Discover all available routines in `.decree/routines/`.
///
/// Scans for `*.sh` files, and also `*.ipynb` when `notebook_support` is true.
/// Deduplicates by name — if both `develop.sh` and `develop.ipynb` exist,
/// the routine appears once with the `.sh` description taking precedence.
pub fn discover_routines(
    project_root: &Path,
    notebook_support: bool,
) -> Result<Vec<RoutineInfo>, DecreeError> {
    let routines_dir = project_root.join(".decree/routines");
    if !routines_dir.is_dir() {
        return Ok(Vec::new());
    }

    // Collect: name -> (sh_description, ipynb_description)
    let mut map: BTreeMap<String, (Option<String>, Option<String>)> = BTreeMap::new();

    for entry in fs::read_dir(&routines_dir)? {
        let entry = entry?;
        let filename = entry.file_name().to_string_lossy().to_string();

        if let Some(name) = filename.strip_suffix(".sh") {
            let path = entry.path();
            let desc = extract_sh_description(&path).unwrap_or_default();
            map.entry(name.to_string())
                .or_insert((None, None))
                .0 = Some(desc);
        } else if notebook_support {
            if let Some(name) = filename.strip_suffix(".ipynb") {
                let path = entry.path();
                let desc = extract_ipynb_description(&path).unwrap_or_default();
                map.entry(name.to_string())
                    .or_insert((None, None))
                    .1 = Some(desc);
            }
        }
    }

    let routines = map
        .into_iter()
        .map(|(name, (sh_desc, ipynb_desc))| {
            // .sh description takes precedence
            let description = sh_desc.or(ipynb_desc).unwrap_or_default();
            RoutineInfo { name, description }
        })
        .collect();

    Ok(routines)
}

/// Resolve a routine name to a file path using discovery precedence rules.
///
/// If the name has an explicit extension (`.sh` or `.ipynb`), use it directly.
/// Otherwise, apply precedence based on `notebook_support`:
/// - `notebook_support: true`: check `.ipynb` first, then `.sh`
/// - `notebook_support: false`: check `.sh` only, ignore `.ipynb`
pub fn resolve_routine(
    project_root: &Path,
    name: &str,
    notebook_support: bool,
) -> Result<ResolvedRoutine, DecreeError> {
    let routines_dir = project_root.join(".decree/routines");

    // Check for explicit extension
    if let Some(stem) = name.strip_suffix(".sh") {
        let path = routines_dir.join(name);
        if path.is_file() {
            return Ok(ResolvedRoutine {
                name: stem.to_string(),
                path,
                format: RoutineFormat::Shell,
            });
        }
        return Err(DecreeError::RoutineNotFound(name.to_string()));
    }

    if let Some(stem) = name.strip_suffix(".ipynb") {
        if !notebook_support {
            return Err(DecreeError::RoutineNotFound(name.to_string()));
        }
        let path = routines_dir.join(name);
        if path.is_file() {
            return Ok(ResolvedRoutine {
                name: stem.to_string(),
                path,
                format: RoutineFormat::Notebook,
            });
        }
        return Err(DecreeError::RoutineNotFound(name.to_string()));
    }

    // No explicit extension — apply precedence rules
    if notebook_support {
        // Notebooks take precedence when enabled
        let ipynb_path = routines_dir.join(format!("{}.ipynb", name));
        if ipynb_path.is_file() {
            return Ok(ResolvedRoutine {
                name: name.to_string(),
                path: ipynb_path,
                format: RoutineFormat::Notebook,
            });
        }
        let sh_path = routines_dir.join(format!("{}.sh", name));
        if sh_path.is_file() {
            return Ok(ResolvedRoutine {
                name: name.to_string(),
                path: sh_path,
                format: RoutineFormat::Shell,
            });
        }
    } else {
        // Only check .sh when notebooks disabled
        let sh_path = routines_dir.join(format!("{}.sh", name));
        if sh_path.is_file() {
            return Ok(ResolvedRoutine {
                name: name.to_string(),
                path: sh_path,
                format: RoutineFormat::Shell,
            });
        }
    }

    Err(DecreeError::RoutineNotFound(name.to_string()))
}

/// Discover custom parameters declared in a routine file.
///
/// Returns the set of parameter names beyond the standard set.
pub fn discover_custom_params(resolved: &ResolvedRoutine) -> Result<Vec<String>, DecreeError> {
    match resolved.format {
        RoutineFormat::Shell => discover_custom_params_sh(&resolved.path),
        RoutineFormat::Notebook => discover_custom_params_ipynb(&resolved.path),
    }
}

/// Discover custom parameters from a shell script.
///
/// Reads the script and collects variable names from lines matching
/// `^[a-z_][a-z0-9_]*=` (simple assignment at start of line), stopping at
/// the first line that is not a comment, blank, shebang, `set` builtin, or
/// assignment. Standard parameter names are excluded.
fn discover_custom_params_sh(path: &Path) -> Result<Vec<String>, DecreeError> {
    let content = fs::read_to_string(path)?;
    let standard: BTreeSet<&str> = STANDARD_PARAMS.iter().copied().collect();
    let mut params = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip blank lines
        if trimmed.is_empty() {
            continue;
        }
        // Skip shebang
        if trimmed.starts_with("#!") {
            continue;
        }
        // Skip comments
        if trimmed.starts_with('#') {
            continue;
        }
        // Skip `set` builtins
        if trimmed.starts_with("set ") {
            continue;
        }

        // Check for variable assignment at start of line (use untrimmed line)
        if let Some(var_name) = parse_sh_assignment(line) {
            if !standard.contains(var_name.as_str()) {
                params.push(var_name);
            }
        } else {
            // First line that doesn't match allowed patterns — stop
            break;
        }
    }

    Ok(params)
}

/// Parse a shell variable assignment at the start of a line.
/// Matches `^[a-z_][a-z0-9_]*=`.
fn parse_sh_assignment(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    if bytes.is_empty() {
        return None;
    }

    let first = bytes[0];
    if !(first.is_ascii_lowercase() || first == b'_') {
        return None;
    }

    let mut end = 1;
    while end < bytes.len() {
        let b = bytes[end];
        if b == b'=' {
            return Some(line[..end].to_string());
        }
        if !(b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
            return None;
        }
        end += 1;
    }

    None
}

/// Discover custom parameters from a notebook's parameters cell.
///
/// Finds the cell tagged with `["parameters"]` and parses Python variable
/// assignments. Standard parameter names are excluded.
fn discover_custom_params_ipynb(path: &Path) -> Result<Vec<String>, DecreeError> {
    let content = fs::read_to_string(path)?;
    let notebook: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| DecreeError::Config(format!("invalid notebook {}: {}", path.display(), e)))?;

    let standard: BTreeSet<&str> = STANDARD_PARAMS.iter().copied().collect();
    let mut params = Vec::new();

    let cells = match notebook.get("cells").and_then(|c| c.as_array()) {
        Some(c) => c,
        None => return Ok(params),
    };

    for cell in cells {
        if !is_parameters_cell(cell) {
            continue;
        }

        let source = match cell.get("source").and_then(|s| s.as_array()) {
            Some(s) => s,
            None => continue,
        };

        let source_text: String = source
            .iter()
            .filter_map(|l| l.as_str())
            .collect::<Vec<_>>()
            .join("");

        for line in source_text.lines() {
            if let Some(var_name) = parse_python_assignment(line) {
                if !standard.contains(var_name.as_str()) {
                    params.push(var_name);
                }
            }
        }

        // Only process the first parameters cell
        break;
    }

    Ok(params)
}

/// Check if a notebook cell has the "parameters" tag.
fn is_parameters_cell(cell: &serde_json::Value) -> bool {
    let tags = cell
        .get("metadata")
        .and_then(|m| m.get("tags"))
        .and_then(|t| t.as_array());

    if let Some(tags) = tags {
        return tags.iter().any(|t| t.as_str() == Some("parameters"));
    }
    false
}

/// Parse a Python variable assignment line like `var_name = "value"`.
fn parse_python_assignment(line: &str) -> Option<String> {
    let trimmed = line.trim();

    // Skip comments and blank lines
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }

    // Look for `name = ...` pattern
    let eq_pos = trimmed.find('=')?;
    // Make sure it's not `==`
    if trimmed.get(eq_pos + 1..eq_pos + 2) == Some("=") {
        return None;
    }

    let name = trimmed[..eq_pos].trim();

    // Validate Python identifier: [a-z_][a-z0-9_]*
    let bytes = name.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let first = bytes[0];
    if !(first.is_ascii_lowercase() || first == b'_') {
        return None;
    }
    for &b in &bytes[1..] {
        if !(b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
            return None;
        }
    }

    Some(name.to_string())
}

/// Build the standard parameters map for a message.
pub fn build_standard_params(msg: &InboxMessage, msg_dir: &Path) -> BTreeMap<String, String> {
    let mut params = BTreeMap::new();
    params.insert(
        "spec_file".to_string(),
        msg.input_file.clone().unwrap_or_default(),
    );
    params.insert(
        "message_file".to_string(),
        msg_dir.join("message.md").to_string_lossy().to_string(),
    );
    params.insert("message_id".to_string(), msg.id.clone());
    params.insert(
        "message_dir".to_string(),
        msg_dir.to_string_lossy().to_string(),
    );
    params.insert("chain".to_string(), msg.chain.clone());
    params.insert("seq".to_string(), msg.seq.to_string());
    params
}

/// Build the custom parameters map by matching message frontmatter fields
/// against declared routine parameters.
pub fn build_custom_params(
    resolved: &ResolvedRoutine,
    msg: &InboxMessage,
) -> Result<BTreeMap<String, String>, DecreeError> {
    let custom_param_names = discover_custom_params(resolved)?;
    let mut params = BTreeMap::new();

    for name in &custom_param_names {
        if let Some(value) = msg.custom_fields.get(name) {
            let val_str = match value {
                serde_yaml::Value::String(s) => s.clone(),
                serde_yaml::Value::Number(n) => n.to_string(),
                serde_yaml::Value::Bool(b) => b.to_string(),
                serde_yaml::Value::Null => String::new(),
                other => serde_yaml::to_string(other)
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
            };
            params.insert(name.clone(), val_str);
        }
    }

    Ok(params)
}

/// Result of routine execution.
#[derive(Debug)]
pub struct ExecutionResult {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub log_path: PathBuf,
}

/// Execute a resolved routine with the given parameters.
///
/// For shell scripts: runs via `bash` with parameters as environment variables,
/// captures stdout/stderr to `<msg_dir>/routine.log`.
///
/// For notebooks: runs via `papermill` with parameters as `-p` flags,
/// output to `<msg_dir>/output.ipynb`, log to `<msg_dir>/papermill.log`.
pub fn execute_routine(
    project_root: &Path,
    resolved: &ResolvedRoutine,
    msg: &InboxMessage,
    msg_dir: &Path,
) -> Result<ExecutionResult, DecreeError> {
    let standard = build_standard_params(msg, msg_dir);
    let custom = build_custom_params(resolved, msg)?;

    match resolved.format {
        RoutineFormat::Shell => execute_shell(project_root, resolved, &standard, &custom, msg_dir),
        RoutineFormat::Notebook => {
            execute_notebook(project_root, resolved, &standard, &custom, msg_dir)
        }
    }
}

/// Execute a shell script routine.
fn execute_shell(
    project_root: &Path,
    resolved: &ResolvedRoutine,
    standard: &BTreeMap<String, String>,
    custom: &BTreeMap<String, String>,
    msg_dir: &Path,
) -> Result<ExecutionResult, DecreeError> {
    let log_path = msg_dir.join("routine.log");
    let log_file = fs::File::create(&log_path)?;
    let log_stderr = log_file.try_clone()?;

    let mut cmd = Command::new("bash");
    cmd.arg(&resolved.path)
        .current_dir(project_root)
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_stderr));

    // Inject standard parameters as env vars
    for (key, value) in standard {
        cmd.env(key, value);
    }

    // Inject custom parameters as env vars
    for (key, value) in custom {
        cmd.env(key, value);
    }

    let status = cmd.status()?;

    Ok(ExecutionResult {
        success: status.success(),
        exit_code: status.code(),
        log_path,
    })
}

/// Execute a notebook routine via papermill.
fn execute_notebook(
    project_root: &Path,
    resolved: &ResolvedRoutine,
    standard: &BTreeMap<String, String>,
    custom: &BTreeMap<String, String>,
    msg_dir: &Path,
) -> Result<ExecutionResult, DecreeError> {
    let output_path = msg_dir.join("output.ipynb");
    let log_path = msg_dir.join("papermill.log");

    let venv_dir = resolve_venv_dir(project_root);
    let papermill_bin = venv_dir.join("bin/papermill");

    let mut cmd = Command::new(&papermill_bin);
    cmd.arg(&resolved.path)
        .arg(&output_path)
        .current_dir(project_root);

    // Add standard parameters as -p flags
    for (key, value) in standard {
        cmd.arg("-p").arg(key).arg(value);
    }

    // Add custom parameters as -p flags
    for (key, value) in custom {
        cmd.arg("-p").arg(key).arg(value);
    }

    let output = cmd.output()?;

    // Write combined stdout+stderr to papermill.log
    let mut log_file = fs::File::create(&log_path)?;
    log_file.write_all(&output.stdout)?;
    log_file.write_all(&output.stderr)?;

    Ok(ExecutionResult {
        success: output.status.success(),
        exit_code: output.status.code(),
        log_path,
    })
}

/// Resolve the venv directory path. Respects `DECREE_VENV` env var.
fn resolve_venv_dir(project_root: &Path) -> PathBuf {
    if let Ok(venv) = std::env::var("DECREE_VENV") {
        PathBuf::from(venv)
    } else {
        project_root.join(".decree/venv")
    }
}

/// Ensure a Python virtual environment exists with papermill and ipykernel.
///
/// Only relevant when `notebook_support: true`. Creates `.decree/venv/`
/// if missing. Shell script routines never require a venv.
pub fn ensure_venv(project_root: &Path) -> Result<(), DecreeError> {
    let venv_dir = resolve_venv_dir(project_root);

    if venv_dir.join("bin/papermill").is_file() {
        return Ok(());
    }

    // Create venv
    let status = Command::new("python3")
        .args(["-m", "venv"])
        .arg(&venv_dir)
        .status()
        .map_err(|e| DecreeError::Config(format!("failed to create venv: {}", e)))?;

    if !status.success() {
        return Err(DecreeError::Config(
            "python3 -m venv failed".to_string(),
        ));
    }

    // Install papermill and ipykernel
    let pip = venv_dir.join("bin/pip");
    let status = Command::new(&pip)
        .args(["install", "papermill", "ipykernel"])
        .status()
        .map_err(|e| DecreeError::Config(format!("pip install failed: {}", e)))?;

    if !status.success() {
        return Err(DecreeError::Config(
            "pip install papermill ipykernel failed".to_string(),
        ));
    }

    Ok(())
}

/// Extract the description from a shell script routine.
///
/// The description is the first block of contiguous `#` comment lines at the
/// top of the script (after the optional shebang). Leading `# ` or lone `#`
/// is stripped.
fn extract_sh_description(path: &Path) -> Result<String, DecreeError> {
    let content = fs::read_to_string(path)?;
    let mut lines: Vec<&str> = Vec::new();
    let mut in_comment_block = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip shebang
        if !in_comment_block && lines.is_empty() && trimmed.starts_with("#!") {
            continue;
        }

        if trimmed.starts_with('#') && !trimmed.starts_with("#!") {
            in_comment_block = true;
            // Strip leading "# " or lone "#"
            let text = trimmed.strip_prefix("# ").unwrap_or(
                trimmed.strip_prefix('#').unwrap_or(""),
            );
            lines.push(text);
        } else if in_comment_block {
            // End of contiguous comment block
            break;
        } else if trimmed.is_empty() {
            // Skip blank lines before first comment
            continue;
        } else {
            // Non-comment, non-blank before any comments
            break;
        }
    }

    Ok(lines.join("\n"))
}

/// Extract the description from a notebook routine.
///
/// The description is the content of the first markdown cell.
fn extract_ipynb_description(path: &Path) -> Result<String, DecreeError> {
    let content = fs::read_to_string(path)?;
    let notebook: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| DecreeError::Config(format!("invalid notebook {}: {}", path.display(), e)))?;

    let cells = notebook
        .get("cells")
        .and_then(|c| c.as_array());

    let cells = match cells {
        Some(c) => c,
        None => return Ok(String::new()),
    };

    for cell in cells {
        let cell_type = cell.get("cell_type").and_then(|t| t.as_str());
        if cell_type == Some("markdown") {
            let source = cell.get("source").and_then(|s| s.as_array());
            if let Some(lines) = source {
                let text: String = lines
                    .iter()
                    .filter_map(|l| l.as_str())
                    .collect::<Vec<_>>()
                    .join("");
                return Ok(text);
            }
        }
    }

    Ok(String::new())
}

/// Build the router prompt for routine selection.
pub fn build_router_prompt(routines: &[RoutineInfo], task_body: &str) -> String {
    let mut prompt = String::from("Select the most appropriate routine for this task.\n\n## Available Routines\n");

    for r in routines {
        let first_line = r.description.lines().next().unwrap_or("");
        prompt.push_str(&format!("- {}: {}\n", r.name, first_line));
    }

    prompt.push_str(&format!("\n## Task\n{}\n\nRespond with ONLY the routine name, nothing else.\n", task_body));
    prompt
}

/// Check if a routine name is valid (exists in the discovered routines list).
pub fn is_valid_routine(routines: &[RoutineInfo], name: &str) -> bool {
    routines.iter().any(|r| r.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sh_assignment_simple() {
        assert_eq!(
            parse_sh_assignment(r#"target_branch="${target_branch:-main}""#),
            Some("target_branch".to_string())
        );
    }

    #[test]
    fn test_parse_sh_assignment_no_default() {
        assert_eq!(
            parse_sh_assignment(r#"spec_file="${spec_file:-}""#),
            Some("spec_file".to_string())
        );
    }

    #[test]
    fn test_parse_sh_assignment_plain_value() {
        assert_eq!(
            parse_sh_assignment(r#"foo=bar"#),
            Some("foo".to_string())
        );
    }

    #[test]
    fn test_parse_sh_assignment_uppercase_rejected() {
        assert_eq!(parse_sh_assignment("FOO=bar"), None);
    }

    #[test]
    fn test_parse_sh_assignment_space_before_eq_rejected() {
        assert_eq!(parse_sh_assignment("foo =bar"), None);
    }

    #[test]
    fn test_parse_python_assignment_string() {
        assert_eq!(
            parse_python_assignment(r#"target_branch = "main""#),
            Some("target_branch".to_string())
        );
    }

    #[test]
    fn test_parse_python_assignment_empty_string() {
        assert_eq!(
            parse_python_assignment(r#"spec_file = """#),
            Some("spec_file".to_string())
        );
    }

    #[test]
    fn test_parse_python_assignment_with_comment() {
        assert_eq!(
            parse_python_assignment(r#"target_branch = ""       # Custom: branch"#),
            Some("target_branch".to_string())
        );
    }

    #[test]
    fn test_parse_python_assignment_comment_line() {
        assert_eq!(parse_python_assignment("# this is a comment"), None);
    }

    #[test]
    fn test_parse_python_assignment_comparison() {
        assert_eq!(parse_python_assignment("if x == 5:"), None);
    }

    #[test]
    fn test_is_parameters_cell_true() {
        let cell: serde_json::Value = serde_json::from_str(
            r#"{"cell_type": "code", "source": ["x = 1"], "metadata": {"tags": ["parameters"]}}"#,
        )
        .unwrap();
        assert!(is_parameters_cell(&cell));
    }

    #[test]
    fn test_is_parameters_cell_false() {
        let cell: serde_json::Value = serde_json::from_str(
            r#"{"cell_type": "code", "source": ["x = 1"], "metadata": {}}"#,
        )
        .unwrap();
        assert!(!is_parameters_cell(&cell));
    }

    #[test]
    fn test_standard_params_constant() {
        assert_eq!(STANDARD_PARAMS.len(), 6);
        assert!(STANDARD_PARAMS.contains(&"spec_file"));
        assert!(STANDARD_PARAMS.contains(&"message_file"));
        assert!(STANDARD_PARAMS.contains(&"message_id"));
        assert!(STANDARD_PARAMS.contains(&"message_dir"));
        assert!(STANDARD_PARAMS.contains(&"chain"));
        assert!(STANDARD_PARAMS.contains(&"seq"));
    }
}
