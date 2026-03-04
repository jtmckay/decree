use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_yaml::Value;

use crate::error::{DecreeError, Result};
use crate::message::Message;

/// Standard parameter names that are always injected by decree.
const STANDARD_PARAMS: &[&str] = &[
    "message_file",
    "message_id",
    "message_dir",
    "chain",
    "seq",
    "input_file",
];

/// A discovered routine shell script.
#[derive(Debug, Clone)]
pub struct Routine {
    /// Name derived from path relative to routines dir (e.g. "develop", "deploy/staging").
    pub name: String,
    /// Full path to the .sh file.
    pub path: PathBuf,
    /// Title from the comment header (e.g. "Develop").
    pub title: String,
    /// First line of the description block.
    pub short_description: String,
    /// Full description text (all collected comment lines joined).
    pub description: String,
    /// Custom parameters discovered from the script.
    pub custom_params: Vec<RoutineParam>,
}

/// A custom parameter discovered from a routine script.
#[derive(Debug, Clone)]
pub struct RoutineParam {
    /// Parameter name.
    pub name: String,
    /// Default value from `${var:-default}`.
    pub default: String,
    /// Whether the parameter is required (lacks "Optional" in doc comment).
    pub required: bool,
    /// Description from the Parameters comment block.
    pub description: String,
}

impl Routine {
    /// Load a routine from a shell script file.
    ///
    /// `routines_dir` is the base `.decree/routines/` directory used to
    /// derive the routine name from the relative path.
    pub fn load(routines_dir: &Path, path: &Path) -> Result<Self> {
        let name = path
            .strip_prefix(routines_dir)
            .map_err(|_| {
                DecreeError::Config(format!(
                    "routine path not under routines dir: {}",
                    path.display()
                ))
            })?
            .with_extension("")
            .to_string_lossy()
            .to_string();

        let content = fs::read_to_string(path)?;
        let (title, short_description, description) = extract_description(&content);
        let param_docs = extract_param_docs(&content);
        let custom_params = discover_custom_params(&content, &param_docs);

        Ok(Self {
            name,
            path: path.to_path_buf(),
            title,
            short_description,
            description,
            custom_params,
        })
    }

    /// Run the pre-check for this routine.
    ///
    /// Sets `DECREE_PRE_CHECK=true` and executes the script. Returns
    /// `Ok(())` on exit 0, or `Err(PreCheckFailed)` otherwise.
    pub fn run_pre_check(&self) -> Result<()> {
        let output = Command::new("bash")
            .arg(&self.path)
            .env("DECREE_PRE_CHECK", "true")
            .output()?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let reason = if stderr.is_empty() {
                String::from_utf8_lossy(&output.stdout).trim().to_string()
            } else {
                stderr
            };
            Err(DecreeError::PreCheckFailed {
                routine: self.name.clone(),
                reason,
            })
        }
    }

    /// Build a `Command` with all environment variables set for execution.
    ///
    /// Standard parameters are populated from the message. Custom parameters
    /// use values from message frontmatter, falling back to script defaults.
    pub fn build_command(&self, message: &Message, message_dir: &Path) -> Command {
        let mut cmd = Command::new("bash");
        cmd.arg(&self.path);

        // Standard parameters
        let message_file = message_dir.join("message.md");
        cmd.env("message_file", message_file.to_string_lossy().as_ref());
        cmd.env("message_id", &message.id);
        cmd.env("message_dir", message_dir.to_string_lossy().as_ref());
        cmd.env("chain", &message.chain);
        cmd.env("seq", message.seq.to_string());

        if let Some(ref input_file) = message.input_file {
            cmd.env("input_file", input_file);
        }

        // Custom parameters from message frontmatter
        for param in &self.custom_params {
            let value = message
                .custom_fields
                .get(&param.name)
                .map(value_to_string)
                .unwrap_or_else(|| param.default.clone());
            cmd.env(&param.name, &value);
        }

        cmd
    }

    /// Execute this routine with the given message context.
    ///
    /// Returns the captured process output.
    pub fn execute(
        &self,
        message: &Message,
        message_dir: &Path,
    ) -> Result<std::process::Output> {
        let mut cmd = self.build_command(message, message_dir);
        let output = cmd.output()?;
        Ok(output)
    }
}

/// Convert a serde_yaml Value to a string for environment variable injection.
fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        other => serde_yaml::to_string(other).unwrap_or_default(),
    }
}

/// Discover all routines in the routines directory recursively.
///
/// Returns routines sorted alphabetically by name.
pub fn discover_routines(routines_dir: &Path) -> Result<Vec<Routine>> {
    if !routines_dir.exists() {
        return Ok(Vec::new());
    }

    let mut routines = Vec::new();
    discover_recursive(routines_dir, routines_dir, &mut routines)?;
    routines.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(routines)
}

fn discover_recursive(
    base_dir: &Path,
    dir: &Path,
    routines: &mut Vec<Routine>,
) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();

        if path.is_dir() {
            discover_recursive(base_dir, &path, routines)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("sh") {
            routines.push(Routine::load(base_dir, &path)?);
        }
    }

    Ok(())
}

/// Find a routine by name.
///
/// Tries the direct path `<routines_dir>/<name>.sh` first, then falls
/// back to scanning all routines.
pub fn find_routine(routines_dir: &Path, name: &str) -> Result<Routine> {
    let path = routines_dir.join(format!("{name}.sh"));
    if path.exists() {
        return Routine::load(routines_dir, &path);
    }

    Err(DecreeError::RoutineNotFound(name.to_string()))
}

/// Extract title, short description, and full description from the script header.
///
/// Follows the convention:
/// 1. Skip shebang (`#!/...`)
/// 2. Next comment line is the title
/// 3. Skip blank comment lines (`#`)
/// 4. Collect subsequent comment lines as description, stripping `# `
/// 5. First collected line is the short description
/// 6. Stop at first non-comment line
fn extract_description(content: &str) -> (String, String, String) {
    let mut lines = content.lines();

    // 1. Skip shebang
    match lines.next() {
        Some(line) if line.starts_with("#!") => {}
        Some(line) if line.starts_with('#') => {
            // No shebang; treat this as the title line
            let title = line.trim_start_matches('#').trim().to_string();
            return collect_description(title, lines);
        }
        _ => return (String::new(), String::new(), String::new()),
    }

    // 2. Next comment line is the title
    match lines.next() {
        Some(line) if line.starts_with('#') => {
            let title = line.trim_start_matches('#').trim().to_string();
            collect_description(title, lines)
        }
        _ => (String::new(), String::new(), String::new()),
    }
}

/// After extracting the title, collect the description block.
fn collect_description(
    title: String,
    mut lines: std::str::Lines<'_>,
) -> (String, String, String) {
    let mut description_lines = Vec::new();
    let mut found_content = false;

    for line in &mut lines {
        if !line.starts_with('#') {
            break;
        }

        let stripped = line.trim_start_matches('#');
        let text = if stripped.starts_with(' ') {
            &stripped[1..]
        } else {
            stripped
        };

        // Step 3: skip blank comment lines before the description starts
        if !found_content && text.trim().is_empty() {
            continue;
        }

        found_content = true;
        description_lines.push(text.to_string());
    }

    let short = description_lines.first().cloned().unwrap_or_default();
    let full = description_lines.join("\n");

    (title, short, full)
}

/// Extract parameter documentation from the `# --- Parameters ---` block.
///
/// Returns a map of param_name → (required, description).
fn extract_param_docs(content: &str) -> BTreeMap<String, (bool, String)> {
    let mut docs = BTreeMap::new();
    let mut in_params = false;

    for line in content.lines() {
        if line.contains("--- Parameters ---") {
            in_params = true;
            continue;
        }

        if !in_params {
            continue;
        }

        if !line.starts_with('#') {
            break;
        }

        let text = line.trim_start_matches('#').trim();

        // Match: "var_name  - Description" or "var_name  - Optional. Description"
        if let Some((name_part, desc_part)) = text.split_once('-') {
            let name = name_part.trim();
            let desc = desc_part.trim();
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                let required = !desc.starts_with("Optional");
                docs.insert(name.to_string(), (required, desc.to_string()));
            }
        }
    }

    docs
}

/// Discover custom parameters from `var="${var:-default}"` assignments.
///
/// Scans the script top-to-bottom:
/// 1. Skip shebang, comment lines, blank lines, `set` builtins
/// 2. Skip the `DECREE_PRE_CHECK` block (from `if` to `fi`)
/// 3. Match assignments of the form `var="${var:-default}"`
/// 4. Stop at the first non-matching line
/// 5. Exclude standard parameter names
fn discover_custom_params(
    content: &str,
    param_docs: &BTreeMap<String, (bool, String)>,
) -> Vec<RoutineParam> {
    let mut params = Vec::new();
    let mut precheck_depth: u32 = 0;

    for line in content.lines() {
        let trimmed = line.trim();

        // Inside pre-check block: track nesting and skip
        if precheck_depth > 0 {
            if trimmed.starts_with("if ") || trimmed.starts_with("if[") {
                precheck_depth += 1;
            }
            if trimmed == "fi" || trimmed.starts_with("fi ") || trimmed.starts_with("fi;") {
                precheck_depth -= 1;
            }
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

        // Skip blank lines
        if trimmed.is_empty() {
            continue;
        }

        // Skip set builtins
        if trimmed.starts_with("set ") {
            continue;
        }

        // Detect DECREE_PRE_CHECK block start
        if trimmed.starts_with("if ") && trimmed.contains("DECREE_PRE_CHECK") {
            precheck_depth = 1;
            continue;
        }

        // Try to match var="${var:-default}" pattern
        if let Some((name, default)) = parse_assignment(trimmed) {
            // Skip standard parameters
            if STANDARD_PARAMS.contains(&name.as_str()) {
                continue;
            }

            let (required, description) = param_docs
                .get(&name)
                .cloned()
                .unwrap_or((false, String::new()));

            params.push(RoutineParam {
                name,
                default,
                required,
                description,
            });
        } else {
            // Stop at first non-matching line
            break;
        }
    }

    params
}

/// Parse a line like `var="${var:-default}"` and return `(name, default)`.
///
/// The variable reference inside `${}` must match the assignment target.
fn parse_assignment(line: &str) -> Option<(String, String)> {
    let (name, rest) = line.split_once('=')?;
    let name = name.trim();

    // Validate variable name: alphanumeric + underscore, non-empty
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }

    let rest = rest.trim();

    // Must be quoted: "${var:-default}"
    let inner = rest.strip_prefix('"')?.strip_suffix('"')?;

    // Must be ${...}
    let inner = inner.strip_prefix("${")?.strip_suffix('}')?;

    // Must contain :-
    let (var_ref, default) = inner.split_once(":-")?;

    // Variable reference must match the assignment target
    if var_ref != name {
        return None;
    }

    Some((name.to_string(), default.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt;

    // --- parse_assignment tests ---

    #[test]
    fn test_parse_assignment_with_default() {
        let result = parse_assignment(r#"my_option="${my_option:-default_value}""#);
        assert_eq!(result, Some(("my_option".to_string(), "default_value".to_string())));
    }

    #[test]
    fn test_parse_assignment_empty_default() {
        let result = parse_assignment(r#"message_file="${message_file:-}""#);
        assert_eq!(result, Some(("message_file".to_string(), String::new())));
    }

    #[test]
    fn test_parse_assignment_default_with_spaces() {
        let result = parse_assignment(r#"greeting="${greeting:-hello world}""#);
        assert_eq!(result, Some(("greeting".to_string(), "hello world".to_string())));
    }

    #[test]
    fn test_parse_assignment_mismatched_name() {
        let result = parse_assignment(r#"foo="${bar:-default}""#);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_assignment_not_quoted() {
        let result = parse_assignment(r#"foo=${foo:-default}"#);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_assignment_no_default_marker() {
        let result = parse_assignment(r#"foo="${foo}""#);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_assignment_regular_assignment() {
        let result = parse_assignment(r#"FOO="bar""#);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_assignment_invalid_name() {
        let result = parse_assignment(r#"my-option="${my-option:-val}""#);
        assert_eq!(result, None);
    }

    // --- extract_description tests ---

    #[test]
    fn test_extract_description_develop() {
        let content = "\
#!/usr/bin/env bash
# Develop
#
# Default routine that delegates work to an AI assistant.
# Reads the spec file or task message, prompts the AI to implement all
# requirements, then verifies acceptance criteria are met.
set -euo pipefail
";
        let (title, short, desc) = extract_description(content);
        assert_eq!(title, "Develop");
        assert_eq!(short, "Default routine that delegates work to an AI assistant.");
        assert!(desc.contains("Default routine that delegates work"));
        assert!(desc.contains("requirements, then verifies acceptance criteria are met."));
    }

    #[test]
    fn test_extract_description_rust_develop() {
        let content = "\
#!/usr/bin/env bash
# Rust Develop
#
# Rust-specific development routine. Delegates implementation to an AI
# assistant, builds and tests the result, then hands build/test output
# to a QA engineer AI to diagnose and fix any failures.
set -euo pipefail
";
        let (title, short, _desc) = extract_description(content);
        assert_eq!(title, "Rust Develop");
        assert_eq!(
            short,
            "Rust-specific development routine. Delegates implementation to an AI"
        );
    }

    #[test]
    fn test_extract_description_no_shebang() {
        let content = "\
# My Routine
#
# Does something useful.
set -euo pipefail
";
        let (title, short, _desc) = extract_description(content);
        assert_eq!(title, "My Routine");
        assert_eq!(short, "Does something useful.");
    }

    #[test]
    fn test_extract_description_empty() {
        let content = "";
        let (title, short, desc) = extract_description(content);
        assert!(title.is_empty());
        assert!(short.is_empty());
        assert!(desc.is_empty());
    }

    #[test]
    fn test_extract_description_title_only() {
        let content = "\
#!/usr/bin/env bash
# Deploy
set -euo pipefail
";
        let (title, short, desc) = extract_description(content);
        assert_eq!(title, "Deploy");
        assert!(short.is_empty());
        assert!(desc.is_empty());
    }

    #[test]
    fn test_extract_description_no_blank_separator() {
        let content = "\
#!/usr/bin/env bash
# Deploy
# Deploys to production.
set -euo pipefail
";
        let (title, short, _desc) = extract_description(content);
        assert_eq!(title, "Deploy");
        assert_eq!(short, "Deploys to production.");
    }

    // --- extract_param_docs tests ---

    #[test]
    fn test_extract_param_docs() {
        let content = "\
#!/usr/bin/env bash
# Test
set -euo pipefail

# --- Parameters ---
# message_file  - Path to message.md in the run directory
# message_id    - Full message ID (<chain>-<seq>)
# input_file    - Optional. Path to input file
# my_option     - Optional. Custom parameter from frontmatter
message_file=\"${message_file:-}\"
";
        let docs = extract_param_docs(content);
        assert_eq!(docs.len(), 4);

        let (req, desc) = docs.get("message_file").expect("message_file");
        assert!(req);
        assert_eq!(desc, "Path to message.md in the run directory");

        let (req, desc) = docs.get("input_file").expect("input_file");
        assert!(!req);
        assert_eq!(desc, "Optional. Path to input file");

        let (req, _desc) = docs.get("my_option").expect("my_option");
        assert!(!req);
    }

    #[test]
    fn test_extract_param_docs_no_block() {
        let content = "\
#!/usr/bin/env bash
# Test
set -euo pipefail
message_file=\"${message_file:-}\"
";
        let docs = extract_param_docs(content);
        assert!(docs.is_empty());
    }

    // --- discover_custom_params tests ---

    #[test]
    fn test_discover_custom_params_with_precheck() {
        let content = r#"#!/usr/bin/env bash
# Test Routine
#
# A test routine.
set -euo pipefail

# --- Parameters ---
# message_file  - Path to message.md
# chain         - Chain ID
# seq           - Sequence number
# my_option     - Optional. Custom parameter
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
input_file="${input_file:-}"

if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    command -v mycommand >/dev/null 2>&1 || { echo "mycommand not found"; exit 1; }
    exit 0
fi

my_option="${my_option:-default_value}"

# --- Implementation starts here ---
echo "doing work"
"#;
        let param_docs = extract_param_docs(content);
        let params = discover_custom_params(content, &param_docs);

        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "my_option");
        assert_eq!(params[0].default, "default_value");
        assert!(!params[0].required);
    }

    #[test]
    fn test_discover_custom_params_no_precheck() {
        let content = r#"#!/usr/bin/env bash
# Simple Routine
set -euo pipefail

message_file="${message_file:-}"
chain="${chain:-}"
seq="${seq:-}"
my_param="${my_param:-foo}"
another="${another:-bar}"

echo "work"
"#;
        let param_docs = BTreeMap::new();
        let params = discover_custom_params(content, &param_docs);

        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "my_param");
        assert_eq!(params[0].default, "foo");
        assert_eq!(params[1].name, "another");
        assert_eq!(params[1].default, "bar");
    }

    #[test]
    fn test_discover_custom_params_stops_at_non_assignment() {
        let content = r#"#!/usr/bin/env bash
# Routine
set -euo pipefail

message_file="${message_file:-}"
my_param="${my_param:-val}"

WORK_FILE="something"
late_param="${late_param:-late}"
"#;
        let param_docs = BTreeMap::new();
        let params = discover_custom_params(content, &param_docs);

        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "my_param");
        // WORK_FILE="something" stops scanning, so late_param is not discovered
    }

    #[test]
    fn test_discover_custom_params_nested_precheck() {
        let content = r#"#!/usr/bin/env bash
# Nested Check
set -euo pipefail

message_file="${message_file:-}"

if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    if ! command -v opencode >/dev/null 2>&1; then
        echo "opencode not found" >&2
        exit 1
    fi
    exit 0
fi

my_option="${my_option:-val}"

echo "work"
"#;
        let param_docs = BTreeMap::new();
        let params = discover_custom_params(content, &param_docs);

        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "my_option");
        assert_eq!(params[0].default, "val");
    }

    #[test]
    fn test_discover_custom_params_develop_sh() {
        // Test against the actual develop.sh content
        let content = r#"#!/usr/bin/env bash
# Develop
#
# Default routine that delegates work to an AI assistant.
# Reads the spec file or task message, prompts the AI to implement all
# requirements, then verifies acceptance criteria are met.
set -euo pipefail

# Parameters (decree injects these as env vars)
spec_file="${spec_file:-}"
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

# Determine the work description
if [ -n "$spec_file" ] && [ -f "$spec_file" ]; then
    WORK_FILE="$spec_file"
else
    WORK_FILE="$message_file"
fi
"#;
        let param_docs = BTreeMap::new();
        let params = discover_custom_params(content, &param_docs);

        // spec_file is the only non-standard param before the first non-assignment line
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "spec_file");
        assert_eq!(params[0].default, "");
    }

    // --- discover_routines / find_routine tests ---

    fn make_routine_dir(name: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(format!("decree_test_{name}"));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("create tmp dir");
        tmp
    }

    fn write_script(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(&path, content).expect("write script");
        let mut perms = fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("set perms");
    }

    #[test]
    fn test_discover_routines_flat() {
        let tmp = make_routine_dir("discover_flat");

        write_script(
            &tmp,
            "develop.sh",
            "#!/usr/bin/env bash\n# Develop\n#\n# Default routine.\nset -euo pipefail\n",
        );
        write_script(
            &tmp,
            "test.sh",
            "#!/usr/bin/env bash\n# Test\n#\n# Runs tests.\nset -euo pipefail\n",
        );

        let routines = discover_routines(&tmp).expect("discover");
        assert_eq!(routines.len(), 2);
        assert_eq!(routines[0].name, "develop");
        assert_eq!(routines[0].title, "Develop");
        assert_eq!(routines[1].name, "test");
        assert_eq!(routines[1].title, "Test");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_discover_routines_nested() {
        let tmp = make_routine_dir("discover_nested");

        write_script(
            &tmp,
            "develop.sh",
            "#!/usr/bin/env bash\n# Develop\nset -euo pipefail\n",
        );
        write_script(
            &tmp,
            "deploy/staging.sh",
            "#!/usr/bin/env bash\n# Deploy Staging\n#\n# Deploys to staging.\nset -euo pipefail\n",
        );
        write_script(
            &tmp,
            "deploy/production.sh",
            "#!/usr/bin/env bash\n# Deploy Production\n#\n# Deploys to prod.\nset -euo pipefail\n",
        );

        let routines = discover_routines(&tmp).expect("discover");
        assert_eq!(routines.len(), 3);

        let names: Vec<&str> = routines.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"deploy/production"));
        assert!(names.contains(&"deploy/staging"));
        assert!(names.contains(&"develop"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_discover_routines_empty_dir() {
        let tmp = make_routine_dir("discover_empty");
        let routines = discover_routines(&tmp).expect("discover");
        assert!(routines.is_empty());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_discover_routines_nonexistent_dir() {
        let routines = discover_routines(Path::new("/tmp/decree_nonexistent_routines_dir"))
            .expect("discover");
        assert!(routines.is_empty());
    }

    #[test]
    fn test_discover_routines_skips_non_sh() {
        let tmp = make_routine_dir("discover_non_sh");

        write_script(
            &tmp,
            "develop.sh",
            "#!/usr/bin/env bash\n# Develop\nset -euo pipefail\n",
        );
        fs::write(tmp.join("readme.md"), "# Routines\n").expect("write");
        fs::write(tmp.join("notes.txt"), "some notes").expect("write");

        let routines = discover_routines(&tmp).expect("discover");
        assert_eq!(routines.len(), 1);
        assert_eq!(routines[0].name, "develop");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_find_routine_direct() {
        let tmp = make_routine_dir("find_direct");

        write_script(
            &tmp,
            "develop.sh",
            "#!/usr/bin/env bash\n# Develop\n#\n# Default routine.\nset -euo pipefail\n",
        );

        let routine = find_routine(&tmp, "develop").expect("find");
        assert_eq!(routine.name, "develop");
        assert_eq!(routine.title, "Develop");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_find_routine_nested() {
        let tmp = make_routine_dir("find_nested");

        write_script(
            &tmp,
            "deploy/staging.sh",
            "#!/usr/bin/env bash\n# Staging Deploy\nset -euo pipefail\n",
        );

        let routine = find_routine(&tmp, "deploy/staging").expect("find");
        assert_eq!(routine.name, "deploy/staging");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_find_routine_not_found() {
        let tmp = make_routine_dir("find_not_found");

        let result = find_routine(&tmp, "nonexistent");
        assert!(result.is_err());
        match result {
            Err(DecreeError::RoutineNotFound(name)) => assert_eq!(name, "nonexistent"),
            other => panic!("expected RoutineNotFound, got: {other:?}"),
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    // --- Routine::load integration test ---

    #[test]
    fn test_routine_load_full() {
        let tmp = make_routine_dir("load_full");

        let script = r#"#!/usr/bin/env bash
# Transcribe
#
# Transcribes audio files using whisper.
# Supports multiple formats and languages.
set -euo pipefail

# --- Parameters ---
# message_file  - Path to message.md in the run directory
# message_id    - Full message ID (<chain>-<seq>)
# message_dir   - Run directory path
# chain         - Chain ID
# seq           - Sequence number
# input_file    - Optional. Path to input file (e.g., migration file)
# model         - Optional. Whisper model to use
# language      - Optional. Target language
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
input_file="${input_file:-}"

if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    command -v whisper >/dev/null 2>&1 || { echo "whisper not found" >&2; exit 1; }
    exit 0
fi

model="${model:-base}"
language="${language:-en}"

# Implementation
whisper --model "$model" --language "$language" "$input_file"
"#;
        write_script(&tmp, "transcribe.sh", script);

        let routine = Routine::load(&tmp, &tmp.join("transcribe.sh")).expect("load");
        assert_eq!(routine.name, "transcribe");
        assert_eq!(routine.title, "Transcribe");
        assert_eq!(routine.short_description, "Transcribes audio files using whisper.");
        assert!(routine.description.contains("Supports multiple formats"));

        assert_eq!(routine.custom_params.len(), 2);
        assert_eq!(routine.custom_params[0].name, "model");
        assert_eq!(routine.custom_params[0].default, "base");
        assert!(!routine.custom_params[0].required);
        assert_eq!(routine.custom_params[1].name, "language");
        assert_eq!(routine.custom_params[1].default, "en");
    }

    // --- build_command tests ---

    #[test]
    fn test_build_command_sets_standard_env_vars() {
        let tmp = make_routine_dir("cmd_standard");

        write_script(
            &tmp,
            "test.sh",
            "#!/usr/bin/env bash\n# Test\nset -euo pipefail\nmessage_file=\"${message_file:-}\"\n",
        );

        let routine = Routine::load(&tmp, &tmp.join("test.sh")).expect("load");

        let message = Message {
            id: "chain123-0".to_string(),
            chain: "chain123".to_string(),
            seq: 0,
            message_type: crate::message::MessageType::Task,
            input_file: Some("specs/01.md".to_string()),
            routine: "test".to_string(),
            custom_fields: BTreeMap::new(),
            body: "Do work.".to_string(),
            path: PathBuf::from(".decree/inbox/chain123-0.md"),
        };

        let run_dir = tmp.join("run");
        fs::create_dir_all(&run_dir).expect("create run dir");

        let cmd = routine.build_command(&message, &run_dir);

        // Verify the command is configured (we can't easily inspect env vars
        // from a Command, but we can verify the program and args)
        let program = cmd.get_program().to_string_lossy().to_string();
        assert_eq!(program, "bash");

        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().to_string()).collect();
        assert_eq!(args.len(), 1);
        assert!(args[0].ends_with("test.sh"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_build_command_injects_custom_params_from_frontmatter() {
        let tmp = make_routine_dir("cmd_custom");

        let script = r#"#!/usr/bin/env bash
# Custom
set -euo pipefail
message_file="${message_file:-}"
chain="${chain:-}"
seq="${seq:-}"

if [ "${DECREE_PRE_CHECK:-}" = "true" ]; then
    exit 0
fi

model="${model:-base}"
"#;
        write_script(&tmp, "custom.sh", script);

        let routine = Routine::load(&tmp, &tmp.join("custom.sh")).expect("load");
        assert_eq!(routine.custom_params.len(), 1);
        assert_eq!(routine.custom_params[0].name, "model");

        let mut custom_fields = BTreeMap::new();
        custom_fields.insert("model".to_string(), Value::String("large-v3".to_string()));

        let message = Message {
            id: "test-0".to_string(),
            chain: "test".to_string(),
            seq: 0,
            message_type: crate::message::MessageType::Task,
            input_file: None,
            routine: "custom".to_string(),
            custom_fields,
            body: String::new(),
            path: PathBuf::from("test.md"),
        };

        let run_dir = tmp.join("run");
        fs::create_dir_all(&run_dir).expect("create run dir");

        let cmd = routine.build_command(&message, &run_dir);

        // Verify env vars are set by inspecting the command's env
        let envs: BTreeMap<String, String> = cmd
            .get_envs()
            .filter_map(|(k, v)| {
                Some((
                    k.to_string_lossy().to_string(),
                    v?.to_string_lossy().to_string(),
                ))
            })
            .collect();

        assert_eq!(envs.get("model").map(|s| s.as_str()), Some("large-v3"));
        assert_eq!(envs.get("message_id").map(|s| s.as_str()), Some("test-0"));
        assert_eq!(envs.get("chain").map(|s| s.as_str()), Some("test"));
        assert_eq!(envs.get("seq").map(|s| s.as_str()), Some("0"));

        let _ = fs::remove_dir_all(&tmp);
    }

    // --- value_to_string tests ---

    #[test]
    fn test_value_to_string_variants() {
        assert_eq!(value_to_string(&Value::String("hello".into())), "hello");
        assert_eq!(
            value_to_string(&Value::Number(serde_yaml::Number::from(42))),
            "42"
        );
        assert_eq!(value_to_string(&Value::Bool(true)), "true");
        assert_eq!(value_to_string(&Value::Null), "");
    }
}
