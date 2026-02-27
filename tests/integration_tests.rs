use std::fs;

use tempfile::TempDir;

/// Helper to set up a fake project root with .decree/ structure for testing.
fn setup_project(tmp: &TempDir) -> std::path::PathBuf {
    let root = tmp.path().to_path_buf();
    let decree = root.join(".decree");
    fs::create_dir_all(decree.join("routines")).unwrap();
    fs::create_dir_all(decree.join("plans")).unwrap();
    fs::create_dir_all(decree.join("cron")).unwrap();
    fs::create_dir_all(decree.join("inbox/done")).unwrap();
    fs::create_dir_all(decree.join("inbox/dead")).unwrap();
    fs::create_dir_all(decree.join("runs")).unwrap();
    fs::create_dir_all(decree.join("sessions")).unwrap();
    fs::create_dir_all(root.join("specs")).unwrap();
    fs::write(root.join("specs/processed-spec.md"), "").unwrap();
    root
}

mod config_tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = decree::config::Config::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.max_depth, 10);
        assert_eq!(config.default_routine, "develop");
        assert!(!config.notebook_support);
        assert_eq!(
            config.ai.model_path,
            "~/.decree/models/qwen2.5-1.5b-instruct-q5_k_m.gguf"
        );
    }

    #[test]
    fn test_config_save_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let config = decree::config::Config::default();
        config.save(&root).unwrap();

        let loaded = decree::config::Config::load(&root).unwrap();
        assert_eq!(loaded.max_retries, config.max_retries);
        assert_eq!(loaded.max_depth, config.max_depth);
        assert_eq!(loaded.default_routine, config.default_routine);
        assert_eq!(loaded.notebook_support, config.notebook_support);
        assert_eq!(loaded.commands.planning, config.commands.planning);
        assert_eq!(loaded.commands.router, config.commands.router);
    }

    #[test]
    fn test_resolved_model_path_expands_tilde() {
        let config = decree::config::Config::default();
        let resolved = config.resolved_model_path();
        // Should not start with ~/
        assert!(!resolved.to_string_lossy().starts_with("~/"));
        assert!(resolved
            .to_string_lossy()
            .contains(".decree/models/qwen2.5-1.5b-instruct-q5_k_m.gguf"));
    }

    #[test]
    fn test_config_load_missing_file() {
        let tmp = TempDir::new().unwrap();
        let result = decree::config::Config::load(tmp.path());
        assert!(result.is_err());
    }
}

mod message_tests {
    use super::*;

    #[test]
    fn test_message_id_parse() {
        let id = decree::message::MessageId::parse("2025022514320000-0").unwrap();
        assert_eq!(id.chain, "2025022514320000");
        assert_eq!(id.seq, 0);
    }

    #[test]
    fn test_message_id_parse_higher_seq() {
        let id = decree::message::MessageId::parse("2025022514320000-42").unwrap();
        assert_eq!(id.chain, "2025022514320000");
        assert_eq!(id.seq, 42);
    }

    #[test]
    fn test_message_id_display() {
        let id = decree::message::MessageId::new("2025022514320000", 3);
        assert_eq!(id.to_string(), "2025022514320000-3");
    }

    #[test]
    fn test_message_id_parse_invalid() {
        assert!(decree::message::MessageId::parse("not-a-valid-id").is_none());
        assert!(decree::message::MessageId::parse("abc").is_none());
    }

    #[test]
    fn test_new_chain_format() {
        let chain = decree::message::MessageId::new_chain(0);
        // Should be 16 chars: 14 timestamp + 2 counter
        assert_eq!(chain.len(), 16);
        assert!(chain.ends_with("00"));
    }

    #[test]
    fn test_new_chain_with_counter() {
        let chain = decree::message::MessageId::new_chain(5);
        assert_eq!(chain.len(), 16);
        assert!(chain.ends_with("05"));
    }

    #[test]
    fn test_resolve_id_exact_match() {
        let tmp = TempDir::new().unwrap();
        let runs = tmp.path().join("runs");
        fs::create_dir_all(runs.join("2025022514320000-0")).unwrap();
        fs::create_dir_all(runs.join("2025022514320000-1")).unwrap();

        let matches = decree::message::resolve_id(&runs, "2025022514320000-0").unwrap();
        assert_eq!(matches, vec!["2025022514320000-0"]);
    }

    #[test]
    fn test_resolve_id_chain_prefix() {
        let tmp = TempDir::new().unwrap();
        let runs = tmp.path().join("runs");
        fs::create_dir_all(runs.join("2025022514320000-0")).unwrap();
        fs::create_dir_all(runs.join("2025022514320000-1")).unwrap();
        fs::create_dir_all(runs.join("2025022514321500-0")).unwrap();

        let matches = decree::message::resolve_id(&runs, "2025022514320000").unwrap();
        assert_eq!(
            matches,
            vec!["2025022514320000-0", "2025022514320000-1"]
        );
    }

    #[test]
    fn test_resolve_id_unique_prefix() {
        let tmp = TempDir::new().unwrap();
        let runs = tmp.path().join("runs");
        fs::create_dir_all(runs.join("2025022514320000-0")).unwrap();
        fs::create_dir_all(runs.join("2025022514321500-0")).unwrap();

        let matches = decree::message::resolve_id(&runs, "202502251432000").unwrap();
        assert_eq!(matches, vec!["2025022514320000-0"]);
    }

    #[test]
    fn test_resolve_id_not_found() {
        let tmp = TempDir::new().unwrap();
        let runs = tmp.path().join("runs");
        fs::create_dir_all(&runs).unwrap();

        let result = decree::message::resolve_id(&runs, "9999999999999999");
        assert!(result.is_err());
    }

    #[test]
    fn test_most_recent() {
        let tmp = TempDir::new().unwrap();
        let runs = tmp.path().join("runs");
        fs::create_dir_all(runs.join("2025022514320000-0")).unwrap();
        fs::create_dir_all(runs.join("2025022514321500-0")).unwrap();
        fs::create_dir_all(runs.join("2025022514320000-1")).unwrap();

        let latest = decree::message::most_recent(&runs).unwrap();
        assert_eq!(latest, "2025022514321500-0");
    }

    #[test]
    fn test_most_recent_empty() {
        let tmp = TempDir::new().unwrap();
        let runs = tmp.path().join("runs");
        fs::create_dir_all(&runs).unwrap();

        let result = decree::message::most_recent(&runs);
        assert!(result.is_err());
    }
}

mod error_tests {
    use super::*;

    #[test]
    fn test_find_project_root_not_initialized() {
        let tmp = TempDir::new().unwrap();
        // Set current dir to an empty temp dir
        let _prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let result = decree::error::find_project_root();
        assert!(result.is_err());

        // Restore
        std::env::set_current_dir(&_prev).unwrap();
    }

    #[test]
    fn test_find_project_root_found() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let _prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(&root).unwrap();

        let found = decree::error::find_project_root().unwrap();
        assert_eq!(found, root);

        std::env::set_current_dir(&_prev).unwrap();
    }

    #[test]
    fn test_error_display() {
        let e = decree::error::DecreeError::NotInitialized;
        assert_eq!(
            e.to_string(),
            "not a decree project (no .decree/ directory found)"
        );

        let e = decree::error::DecreeError::NoSpecs;
        assert_eq!(e.to_string(), "no spec files found in specs/");
    }
}

mod ai_provider_tests {
    #[test]
    fn test_provider_commands() {
        use decree::config::AiProvider;

        assert_eq!(AiProvider::ClaudeCli.planning_command(), "claude -p {prompt}");
        assert_eq!(AiProvider::ClaudeCli.planning_continue_command(), "claude --continue");

        assert_eq!(AiProvider::CopilotCli.planning_command(), "copilot -p {prompt}");
        assert_eq!(AiProvider::CopilotCli.planning_continue_command(), "copilot --continue");

        assert_eq!(AiProvider::Embedded.planning_command(), "decree ai");
        assert_eq!(AiProvider::Embedded.planning_continue_command(), "");

        assert_eq!(AiProvider::Embedded.router_command(), "decree ai");
    }

    #[test]
    fn test_provider_display() {
        use decree::config::AiProvider;

        assert_eq!(format!("{}", AiProvider::ClaudeCli), "Claude CLI");
        assert_eq!(format!("{}", AiProvider::CopilotCli), "GitHub Copilot CLI");
        assert_eq!(format!("{}", AiProvider::Embedded), "Embedded (decree ai)");
    }
}

mod template_content_tests {
    use decree::commands::init::*;

    // --- SOW template ---

    #[test]
    fn sow_template_has_title_and_structure() {
        assert!(TEMPLATE_SOW.contains("# Statement of Work Template"));
        assert!(TEMPLATE_SOW.contains("## Structure"));
        assert!(TEMPLATE_SOW.contains("## Writing Guidelines"));
        assert!(TEMPLATE_SOW.contains("## Example"));
    }

    #[test]
    fn sow_template_has_structure_sections() {
        for section in &[
            "**Title**",
            "**Business Context**",
            "**Jobs to Be Done**",
            "**User Scenarios**",
            "**Scope**",
            "**Deliverables**",
            "**Acceptance Criteria**",
            "**Assumptions & Constraints**",
        ] {
            assert!(
                TEMPLATE_SOW.contains(section),
                "SOW template missing section: {section}"
            );
        }
    }

    #[test]
    fn sow_template_has_example() {
        assert!(TEMPLATE_SOW.contains("# SOW: Secure Account Access"));
        assert!(TEMPLATE_SOW.contains("## Business Context"));
        assert!(TEMPLATE_SOW.contains("## Jobs to Be Done"));
        assert!(TEMPLATE_SOW.contains("## User Scenarios"));
    }

    #[test]
    fn sow_template_mentions_decree_plan() {
        assert!(TEMPLATE_SOW.contains("decree plan"));
    }

    // --- Spec template ---

    #[test]
    fn spec_template_has_format_and_rules() {
        assert!(TEMPLATE_SPEC.contains("# Spec Template"));
        assert!(TEMPLATE_SPEC.contains("## Format"));
        assert!(TEMPLATE_SPEC.contains("## Rules"));
    }

    #[test]
    fn spec_template_has_bdd_guidelines() {
        assert!(TEMPLATE_SPEC.contains("**Given**"));
        assert!(TEMPLATE_SPEC.contains("**When**"));
        assert!(TEMPLATE_SPEC.contains("**Then**"));
        assert!(TEMPLATE_SPEC.contains("### Guidelines"));
        assert!(TEMPLATE_SPEC.contains("### Example"));
    }

    #[test]
    fn spec_template_has_rules() {
        for rule in &[
            "**Naming**",
            "**Frontmatter**",
            "**Ordering**",
            "**Immutability**",
            "**Self-contained**",
            "**Day-sized**",
            "**Testable**",
        ] {
            assert!(
                TEMPLATE_SPEC.contains(rule),
                "Spec template missing rule: {rule}"
            );
        }
    }

    #[test]
    fn spec_template_mentions_decree_plan() {
        assert!(TEMPLATE_SPEC.contains("decree plan"));
    }

    // --- Gitignore ---

    #[test]
    fn gitignore_has_required_entries() {
        for entry in &["venv/", "inbox/", "runs/", "sessions/", "last-run.yml"] {
            assert!(
                TEMPLATE_GITIGNORE.contains(entry),
                "Gitignore missing entry: {entry}"
            );
        }
    }
}

mod shell_routine_tests {
    use decree::commands::init::*;

    // --- develop.sh ---

    #[test]
    fn develop_sh_has_shebang_and_strict_mode() {
        assert!(TEMPLATE_DEVELOP_SH.starts_with("#!/usr/bin/env bash"));
        assert!(TEMPLATE_DEVELOP_SH.contains("set -euo pipefail"));
    }

    #[test]
    fn develop_sh_has_description_comment() {
        assert!(TEMPLATE_DEVELOP_SH.contains("# Develop\n"));
        assert!(TEMPLATE_DEVELOP_SH.contains("# Default routine that delegates work to an AI assistant."));
    }

    #[test]
    fn develop_sh_has_parameter_declarations() {
        for param in &[
            "spec_file=\"${spec_file:-}\"",
            "message_file=\"${message_file:-}\"",
            "message_id=\"${message_id:-}\"",
            "message_dir=\"${message_dir:-}\"",
            "chain=\"${chain:-}\"",
            "seq=\"${seq:-}\"",
        ] {
            assert!(
                TEMPLATE_DEVELOP_SH.contains(param),
                "develop.sh missing parameter: {param}"
            );
        }
    }

    #[test]
    fn develop_sh_has_work_file_determination() {
        assert!(TEMPLATE_DEVELOP_SH.contains("if [ -n \"$spec_file\" ] && [ -f \"$spec_file\" ]"));
        assert!(TEMPLATE_DEVELOP_SH.contains("WORK_FILE=\"$spec_file\""));
        assert!(TEMPLATE_DEVELOP_SH.contains("WORK_FILE=\"$message_file\""));
    }

    #[test]
    fn develop_sh_has_implementation_and_verification() {
        assert!(TEMPLATE_DEVELOP_SH.contains("# Implementation"));
        assert!(TEMPLATE_DEVELOP_SH.contains("# Verification"));
    }

    #[test]
    fn develop_sh_uses_placeholders() {
        assert!(TEMPLATE_DEVELOP_SH.contains("{AI_CMD}"));
        assert!(TEMPLATE_DEVELOP_SH.contains("{ALLOWED_TOOLS}"));
    }

    // --- rust-develop.sh ---

    #[test]
    fn rust_develop_sh_has_shebang_and_strict_mode() {
        assert!(TEMPLATE_RUST_DEVELOP_SH.starts_with("#!/usr/bin/env bash"));
        assert!(TEMPLATE_RUST_DEVELOP_SH.contains("set -euo pipefail"));
    }

    #[test]
    fn rust_develop_sh_has_description_comment() {
        assert!(TEMPLATE_RUST_DEVELOP_SH.contains("# Rust Develop\n"));
        assert!(TEMPLATE_RUST_DEVELOP_SH.contains("# Rust-specific development routine."));
    }

    #[test]
    fn rust_develop_sh_has_parameter_declarations() {
        for param in &[
            "spec_file=\"${spec_file:-}\"",
            "message_file=\"${message_file:-}\"",
            "message_id=\"${message_id:-}\"",
            "message_dir=\"${message_dir:-}\"",
            "chain=\"${chain:-}\"",
            "seq=\"${seq:-}\"",
        ] {
            assert!(
                TEMPLATE_RUST_DEVELOP_SH.contains(param),
                "rust-develop.sh missing parameter: {param}"
            );
        }
    }

    #[test]
    fn rust_develop_sh_has_three_steps() {
        assert!(TEMPLATE_RUST_DEVELOP_SH.contains("# Step 1: Implementation"));
        assert!(TEMPLATE_RUST_DEVELOP_SH.contains("# Step 2: Build and test"));
        assert!(TEMPLATE_RUST_DEVELOP_SH.contains("# Step 3: QA"));
    }

    #[test]
    fn rust_develop_sh_has_cargo_commands() {
        assert!(TEMPLATE_RUST_DEVELOP_SH.contains("cargo build --release"));
        assert!(TEMPLATE_RUST_DEVELOP_SH.contains("cargo test"));
        assert!(TEMPLATE_RUST_DEVELOP_SH.contains("build.log"));
        assert!(TEMPLATE_RUST_DEVELOP_SH.contains("test-output.log"));
    }

    #[test]
    fn rust_develop_sh_uses_placeholders() {
        assert!(TEMPLATE_RUST_DEVELOP_SH.contains("{AI_CMD}"));
        assert!(TEMPLATE_RUST_DEVELOP_SH.contains("{ALLOWED_TOOLS}"));
    }
}

mod notebook_structure_tests {
    use decree::commands::init::*;

    #[test]
    fn develop_ipynb_is_valid_json() {
        let v: serde_json::Value = serde_json::from_str(TEMPLATE_DEVELOP_IPYNB)
            .expect("develop.ipynb must be valid JSON");
        assert_eq!(v["nbformat"], 4);
    }

    #[test]
    fn develop_ipynb_has_four_cells() {
        let v: serde_json::Value = serde_json::from_str(TEMPLATE_DEVELOP_IPYNB).unwrap();
        let cells = v["cells"].as_array().unwrap();
        assert_eq!(cells.len(), 4, "develop.ipynb should have 4 cells");
    }

    #[test]
    fn develop_ipynb_cell_types() {
        let v: serde_json::Value = serde_json::from_str(TEMPLATE_DEVELOP_IPYNB).unwrap();
        let cells = v["cells"].as_array().unwrap();
        assert_eq!(cells[0]["cell_type"], "markdown");
        assert_eq!(cells[1]["cell_type"], "code");
        assert_eq!(cells[2]["cell_type"], "code");
        assert_eq!(cells[3]["cell_type"], "code");
    }

    #[test]
    fn develop_ipynb_markdown_has_description() {
        let v: serde_json::Value = serde_json::from_str(TEMPLATE_DEVELOP_IPYNB).unwrap();
        let cells = v["cells"].as_array().unwrap();
        let source: String = cells[0]["source"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        assert!(source.contains("# Develop"));
        assert!(source.contains("Default routine that delegates work to an AI assistant."));
    }

    #[test]
    fn develop_ipynb_parameters_cell_tagged() {
        let v: serde_json::Value = serde_json::from_str(TEMPLATE_DEVELOP_IPYNB).unwrap();
        let cells = v["cells"].as_array().unwrap();
        let tags = cells[1]["metadata"]["tags"].as_array().unwrap();
        assert!(
            tags.iter().any(|t| t == "parameters"),
            "Parameters cell must have 'parameters' tag"
        );
    }

    #[test]
    fn develop_ipynb_parameters_cell_declares_all_params() {
        let v: serde_json::Value = serde_json::from_str(TEMPLATE_DEVELOP_IPYNB).unwrap();
        let cells = v["cells"].as_array().unwrap();
        let source: String = cells[1]["source"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        for param in &[
            "input_file",
            "message_file",
            "message_id",
            "message_dir",
            "chain",
            "seq",
        ] {
            assert!(
                source.contains(&format!("{param} = \"\"")),
                "develop.ipynb parameters missing: {param}"
            );
        }
    }

    #[test]
    fn develop_ipynb_uses_placeholders() {
        assert!(TEMPLATE_DEVELOP_IPYNB.contains("{AI_CMD}"));
        assert!(TEMPLATE_DEVELOP_IPYNB.contains("{ALLOWED_TOOLS}"));
    }

    // --- rust-develop.ipynb ---

    #[test]
    fn rust_develop_ipynb_is_valid_json() {
        let v: serde_json::Value = serde_json::from_str(TEMPLATE_RUST_DEVELOP_IPYNB)
            .expect("rust-develop.ipynb must be valid JSON");
        assert_eq!(v["nbformat"], 4);
    }

    #[test]
    fn rust_develop_ipynb_has_six_cells() {
        let v: serde_json::Value = serde_json::from_str(TEMPLATE_RUST_DEVELOP_IPYNB).unwrap();
        let cells = v["cells"].as_array().unwrap();
        assert_eq!(cells.len(), 6, "rust-develop.ipynb should have 6 cells");
    }

    #[test]
    fn rust_develop_ipynb_cell_types() {
        let v: serde_json::Value = serde_json::from_str(TEMPLATE_RUST_DEVELOP_IPYNB).unwrap();
        let cells = v["cells"].as_array().unwrap();
        assert_eq!(cells[0]["cell_type"], "markdown");
        assert_eq!(cells[1]["cell_type"], "code"); // parameters
        assert_eq!(cells[2]["cell_type"], "code"); // work file determination
        assert_eq!(cells[3]["cell_type"], "code"); // implementation
        assert_eq!(cells[4]["cell_type"], "code"); // build/test
        assert_eq!(cells[5]["cell_type"], "code"); // QA
    }

    #[test]
    fn rust_develop_ipynb_parameters_cell_tagged() {
        let v: serde_json::Value = serde_json::from_str(TEMPLATE_RUST_DEVELOP_IPYNB).unwrap();
        let cells = v["cells"].as_array().unwrap();
        let tags = cells[1]["metadata"]["tags"].as_array().unwrap();
        assert!(
            tags.iter().any(|t| t == "parameters"),
            "Parameters cell must have 'parameters' tag"
        );
    }

    #[test]
    fn rust_develop_ipynb_parameters_cell_declares_all_params() {
        let v: serde_json::Value = serde_json::from_str(TEMPLATE_RUST_DEVELOP_IPYNB).unwrap();
        let cells = v["cells"].as_array().unwrap();
        let source: String = cells[1]["source"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        for param in &[
            "spec_file",
            "message_file",
            "message_id",
            "message_dir",
            "chain",
            "seq",
        ] {
            assert!(
                source.contains(&format!("{param} = \"\"")),
                "rust-develop.ipynb parameters missing: {param}"
            );
        }
    }

    #[test]
    fn rust_develop_ipynb_has_cargo_commands() {
        assert!(TEMPLATE_RUST_DEVELOP_IPYNB.contains("cargo build --release"));
        assert!(TEMPLATE_RUST_DEVELOP_IPYNB.contains("cargo test"));
        assert!(TEMPLATE_RUST_DEVELOP_IPYNB.contains("build.log"));
        assert!(TEMPLATE_RUST_DEVELOP_IPYNB.contains("test-output.log"));
    }

    #[test]
    fn rust_develop_ipynb_has_work_file_determination() {
        let v: serde_json::Value = serde_json::from_str(TEMPLATE_RUST_DEVELOP_IPYNB).unwrap();
        let cells = v["cells"].as_array().unwrap();
        let source: String = cells[2]["source"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        assert!(source.contains("import os"));
        assert!(source.contains("os.path.isfile(spec_file)"));
    }

    #[test]
    fn rust_develop_ipynb_uses_placeholders() {
        assert!(TEMPLATE_RUST_DEVELOP_IPYNB.contains("{AI_CMD}"));
        assert!(TEMPLATE_RUST_DEVELOP_IPYNB.contains("{ALLOWED_TOOLS}"));
    }
}

mod render_tests {
    use decree::commands::init::*;
    use decree::config::AiProvider;

    #[test]
    fn render_template_replaces_ai_cmd() {
        let result = render_template("run {AI_CMD} here", "claude -p", "--tools");
        assert_eq!(result, "run claude -p here");
    }

    #[test]
    fn render_template_replaces_allowed_tools() {
        let result = render_template("tools: {ALLOWED_TOOLS}", "cmd", "--allowedTools 'Edit'");
        assert_eq!(result, "tools: --allowedTools 'Edit'");
    }

    #[test]
    fn render_template_replaces_both_placeholders() {
        let template = "{AI_CMD} \"prompt\" \\\n  {ALLOWED_TOOLS}";
        let result = render_template(template, "claude -p", "--allowedTools 'Edit,Write'");
        assert_eq!(result, "claude -p \"prompt\" \\\n  --allowedTools 'Edit,Write'");
    }

    #[test]
    fn render_template_handles_empty_allowed_tools() {
        let template = "{AI_CMD} \"prompt\" \\\n  {ALLOWED_TOOLS}";
        let result = render_template(template, "decree ai -p", "");
        assert_eq!(result, "decree ai -p \"prompt\" \\\n  ");
    }

    #[test]
    fn ai_cmd_and_tools_claude_cli() {
        let (cmd, tools) = ai_cmd_and_tools(AiProvider::ClaudeCli);
        assert_eq!(cmd, "claude -p");
        assert!(tools.contains("--allowedTools"));
        assert!(tools.contains("Edit"));
        assert!(tools.contains("Write"));
        assert!(tools.contains("Bash(cargo*)"));
    }

    #[test]
    fn ai_cmd_and_tools_copilot_cli() {
        let (cmd, tools) = ai_cmd_and_tools(AiProvider::CopilotCli);
        assert_eq!(cmd, "copilot -p");
        assert!(tools.contains("--allowedTools"));
    }

    #[test]
    fn ai_cmd_and_tools_embedded() {
        let (cmd, tools) = ai_cmd_and_tools(AiProvider::Embedded);
        assert_eq!(cmd, "decree ai -p");
        assert_eq!(tools, "");
    }

    #[test]
    fn develop_sh_renders_for_claude() {
        let (cmd, tools) = ai_cmd_and_tools(AiProvider::ClaudeCli);
        let rendered = render_template(TEMPLATE_DEVELOP_SH, cmd, tools);
        assert!(rendered.contains("claude -p \"You are a senior software engineer."));
        assert!(rendered.contains("--allowedTools"));
        assert!(!rendered.contains("{AI_CMD}"));
        assert!(!rendered.contains("{ALLOWED_TOOLS}"));
    }

    #[test]
    fn rust_develop_sh_renders_for_claude() {
        let (cmd, tools) = ai_cmd_and_tools(AiProvider::ClaudeCli);
        let rendered = render_template(TEMPLATE_RUST_DEVELOP_SH, cmd, tools);
        assert!(rendered.contains("claude -p \"You are a senior Rust engineer."));
        assert!(rendered.contains("claude -p \"You are an expert Quality Assurance Engineer"));
        assert!(!rendered.contains("{AI_CMD}"));
        assert!(!rendered.contains("{ALLOWED_TOOLS}"));
    }

    #[test]
    fn develop_ipynb_renders_for_claude() {
        let (cmd, tools) = ai_cmd_and_tools(AiProvider::ClaudeCli);
        let rendered = render_template(TEMPLATE_DEVELOP_IPYNB, cmd, tools);
        assert!(rendered.contains("claude -p"));
        assert!(!rendered.contains("{AI_CMD}"));
        assert!(!rendered.contains("{ALLOWED_TOOLS}"));
        // Must still be valid JSON after rendering
        let _: serde_json::Value =
            serde_json::from_str(&rendered).expect("rendered develop.ipynb must be valid JSON");
    }
}

mod write_behavior_tests {
    use super::*;
    use decree::commands::init::*;

    #[test]
    fn write_if_absent_creates_new_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");
        let written = write_if_absent(&path, "hello").unwrap();
        assert!(written);
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn write_if_absent_skips_existing_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");
        fs::write(&path, "original").unwrap();
        let written = write_if_absent(&path, "new content").unwrap();
        assert!(!written);
        assert_eq!(fs::read_to_string(&path).unwrap(), "original");
    }

    #[test]
    fn write_executable_creates_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("script.sh");
        let written = write_executable(&path, "#!/bin/bash\necho hi").unwrap();
        assert!(written);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "#!/bin/bash\necho hi"
        );
    }

    #[cfg(unix)]
    #[test]
    fn write_executable_sets_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("script.sh");
        write_executable(&path, "#!/bin/bash").unwrap();
        let perms = fs::metadata(&path).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o755);
    }

    #[test]
    fn write_executable_skips_existing_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("script.sh");
        fs::write(&path, "original").unwrap();
        let written = write_executable(&path, "new").unwrap();
        assert!(!written);
        assert_eq!(fs::read_to_string(&path).unwrap(), "original");
    }
}

mod cli_tests {
    use assert_cmd::Command;
    use predicates::prelude::*;

    #[test]
    fn test_help_flag() {
        Command::cargo_bin("decree")
            .unwrap()
            .arg("--help")
            .assert()
            .success()
            .stdout(predicate::str::contains("Specification-driven project execution framework"));
    }

    #[test]
    fn test_all_subcommands_recognized() {
        for cmd in &[
            "plan", "run", "process", "daemon", "diff", "apply", "sow", "ai", "bench", "status",
            "log",
        ] {
            Command::cargo_bin("decree")
                .unwrap()
                .arg(cmd)
                .arg("--help")
                .assert()
                .success();
        }
    }

    #[test]
    fn test_init_help() {
        Command::cargo_bin("decree")
            .unwrap()
            .args(["init", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("model-path"));
    }

    #[test]
    fn test_unknown_subcommand() {
        Command::cargo_bin("decree")
            .unwrap()
            .arg("nonexistent")
            .assert()
            .failure();
    }

    #[test]
    fn test_no_args_outside_project() {
        Command::cargo_bin("decree")
            .unwrap()
            .current_dir(std::env::temp_dir())
            .assert()
            .success()
            .stdout(predicate::str::contains("decree init"));
    }

    #[test]
    fn test_diff_no_messages() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".decree/runs")).unwrap();

        Command::cargo_bin("decree")
            .unwrap()
            .arg("diff")
            .current_dir(tmp.path())
            .assert()
            .failure();
    }

    #[test]
    fn test_apply_no_args_lists_messages() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".decree/runs")).unwrap();

        Command::cargo_bin("decree")
            .unwrap()
            .arg("apply")
            .current_dir(tmp.path())
            .assert()
            .success()
            .stdout(predicate::str::contains("No messages found"));
    }

    #[test]
    fn test_ai_help_shows_options() {
        Command::cargo_bin("decree")
            .unwrap()
            .args(["ai", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--resume"))
            .stdout(predicate::str::contains("--json"))
            .stdout(predicate::str::contains("--max-tokens"))
            .stdout(predicate::str::contains("-p"));
    }

    #[test]
    fn test_bench_help_shows_options() {
        Command::cargo_bin("decree")
            .unwrap()
            .args(["bench", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--runs"))
            .stdout(predicate::str::contains("--max-tokens"))
            .stdout(predicate::str::contains("--ctx"))
            .stdout(predicate::str::contains("-v"));
    }

    #[test]
    fn test_ai_outside_project_fails() {
        Command::cargo_bin("decree")
            .unwrap()
            .arg("ai")
            .current_dir(std::env::temp_dir())
            .assert()
            .failure()
            .stderr(predicate::str::contains("not a decree project"));
    }

    #[test]
    fn test_bench_outside_project_fails() {
        Command::cargo_bin("decree")
            .unwrap()
            .arg("bench")
            .current_dir(std::env::temp_dir())
            .assert()
            .failure()
            .stderr(predicate::str::contains("not a decree project"));
    }

    #[test]
    fn test_bench_default_runs_is_three() {
        // Verify --help shows default of 3 for --runs
        Command::cargo_bin("decree")
            .unwrap()
            .args(["bench", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("[default: 3]"));
    }
}

mod session_tests {
    use super::*;

    #[test]
    fn test_session_new_has_valid_id() {
        let session = decree::session::Session::new();
        assert_eq!(session.id.len(), 14);
        assert!(session.id.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn test_session_save_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let mut session = decree::session::Session::new();
        session.history.push(decree::llm::ChatMessage {
            role: "user".to_string(),
            content: "What is Rust?".to_string(),
        });
        session.history.push(decree::llm::ChatMessage {
            role: "assistant".to_string(),
            content: "Rust is a systems programming language.".to_string(),
        });
        session.save(&root).unwrap();

        let loaded = decree::session::Session::load(&root, &session.id).unwrap();
        assert_eq!(loaded.id, session.id);
        assert_eq!(loaded.history.len(), 2);
        assert_eq!(loaded.history[0].role, "user");
        assert_eq!(loaded.history[0].content, "What is Rust?");
        assert_eq!(loaded.history[1].role, "assistant");
    }

    #[test]
    fn test_session_atomic_write() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let mut session = decree::session::Session::new();
        session.save(&root).unwrap();

        // .tmp should not linger
        let tmp_path = root
            .join(".decree/sessions")
            .join(format!("{}.yml.tmp", session.id));
        assert!(!tmp_path.exists());

        // actual file exists
        let path = root
            .join(".decree/sessions")
            .join(format!("{}.yml", session.id));
        assert!(path.exists());
    }

    #[test]
    fn test_session_file_valid_yaml_format() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let mut session = decree::session::Session::new();
        session.history.push(decree::llm::ChatMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
        });
        session.history.push(decree::llm::ChatMessage {
            role: "assistant".to_string(),
            content: "Hi!".to_string(),
        });
        session.save(&root).unwrap();

        let path = root
            .join(".decree/sessions")
            .join(format!("{}.yml", session.id));
        let content = fs::read_to_string(&path).unwrap();
        let val: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
        assert!(val["id"].is_string());
        assert!(val["created"].is_string());
        assert!(val["updated"].is_string());
        assert!(val["history"].is_sequence());
        let history = val["history"].as_sequence().unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0]["role"].as_str().unwrap(), "user");
        assert_eq!(history[1]["role"].as_str().unwrap(), "assistant");
    }

    #[test]
    fn test_session_load_latest() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let mut s1 = decree::session::Session::new();
        s1.id = "20260226140000".to_string();
        s1.save(&root).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(50));

        let mut s2 = decree::session::Session::new();
        s2.id = "20260226150000".to_string();
        s2.save(&root).unwrap();

        let latest = decree::session::Session::load_latest(&root).unwrap();
        assert_eq!(latest.id, "20260226150000");
    }

    #[test]
    fn test_session_load_nonexistent_errors() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let result = decree::session::Session::load(&root, "99999999999999");
        assert!(result.is_err());
    }

    #[test]
    fn test_session_load_latest_empty_errors() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let result = decree::session::Session::load_latest(&root);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("no sessions found"));
    }

    #[test]
    fn test_session_list() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let mut s1 = decree::session::Session::new();
        s1.id = "20260226140000".to_string();
        s1.save(&root).unwrap();

        let mut s2 = decree::session::Session::new();
        s2.id = "20260226150000".to_string();
        s2.save(&root).unwrap();

        let ids = decree::session::list_sessions(&root).unwrap();
        assert_eq!(ids, vec!["20260226140000", "20260226150000"]);
    }

    #[test]
    fn test_session_history_alternating_roles() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let mut session = decree::session::Session::new();
        for i in 0..4 {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            session.history.push(decree::llm::ChatMessage {
                role: role.to_string(),
                content: format!("Message {i}"),
            });
        }
        session.save(&root).unwrap();

        let loaded = decree::session::Session::load(&root, &session.id).unwrap();
        assert_eq!(loaded.history.len(), 4);
        assert_eq!(loaded.history[0].role, "user");
        assert_eq!(loaded.history[1].role, "assistant");
        assert_eq!(loaded.history[2].role, "user");
        assert_eq!(loaded.history[3].role, "assistant");
    }

    #[test]
    fn test_session_preserves_full_history_on_save() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let mut session = decree::session::Session::new();

        // First exchange
        session.history.push(decree::llm::ChatMessage {
            role: "user".to_string(),
            content: "Q1".to_string(),
        });
        session.history.push(decree::llm::ChatMessage {
            role: "assistant".to_string(),
            content: "A1".to_string(),
        });
        session.save(&root).unwrap();

        // Second exchange
        session.history.push(decree::llm::ChatMessage {
            role: "user".to_string(),
            content: "Q2".to_string(),
        });
        session.history.push(decree::llm::ChatMessage {
            role: "assistant".to_string(),
            content: "A2".to_string(),
        });
        session.save(&root).unwrap();

        // File should have all 4 messages
        let loaded = decree::session::Session::load(&root, &session.id).unwrap();
        assert_eq!(loaded.history.len(), 4);
    }
}

mod llm_tests {
    #[test]
    fn test_build_chatml_basic() {
        let messages = vec![
            decree::llm::ChatMessage {
                role: "system".to_string(),
                content: "You are helpful.".to_string(),
            },
            decree::llm::ChatMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
            },
        ];
        let prompt = decree::llm::build_chatml(&messages, true);
        assert!(prompt.contains("<|im_start|>system\nYou are helpful.<|im_end|>"));
        assert!(prompt.contains("<|im_start|>user\nHello<|im_end|>"));
        assert!(prompt.ends_with("<|im_start|>assistant\n"));
    }

    #[test]
    fn test_build_chatml_no_generation_prompt() {
        let messages = vec![decree::llm::ChatMessage {
            role: "user".to_string(),
            content: "Hi".to_string(),
        }];
        let prompt = decree::llm::build_chatml(&messages, false);
        assert!(prompt.contains("<|im_start|>user\nHi<|im_end|>"));
        assert!(!prompt.contains("<|im_start|>assistant"));
    }

    #[test]
    fn test_build_chatml_empty() {
        let prompt = decree::llm::build_chatml(&[], true);
        assert_eq!(prompt, "<|im_start|>assistant\n");
    }

    #[test]
    fn test_build_chatml_multi_turn() {
        let messages = vec![
            decree::llm::ChatMessage {
                role: "user".to_string(),
                content: "Q1".to_string(),
            },
            decree::llm::ChatMessage {
                role: "assistant".to_string(),
                content: "A1".to_string(),
            },
            decree::llm::ChatMessage {
                role: "user".to_string(),
                content: "Q2".to_string(),
            },
        ];
        let prompt = decree::llm::build_chatml(&messages, true);
        assert!(prompt.contains("Q1"));
        assert!(prompt.contains("A1"));
        assert!(prompt.contains("Q2"));
        // Should end with assistant prompt
        assert!(prompt.ends_with("<|im_start|>assistant\n"));
    }

    #[test]
    fn test_detect_build_backend_cpu() {
        // Default build (no GPU features) should be CPU
        let backend = decree::llm::detect_build_backend();
        assert_eq!(backend, "CPU");
    }

    #[test]
    fn test_detect_gpu_cpu_only() {
        let gpu = decree::llm::detect_gpu(0);
        assert!(
            gpu.contains("none"),
            "CPU-only build should report 'none', got: {gpu}"
        );
    }
}

mod checkpoint_tests {
    use super::*;
    use decree::checkpoint;

    /// Helper: create a minimal project with some source files.
    fn setup_checkpoint_project(tmp: &TempDir) -> std::path::PathBuf {
        let root = setup_project(tmp);
        // Create some source files.
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn hello() -> &'static str { \"hello\" }\n").unwrap();
        fs::write(root.join("README.md"), "# My Project\n").unwrap();
        root
    }

    // --- SHA-256 ---

    #[test]
    fn test_sha256_hex_known_value() {
        // SHA-256 of empty string.
        let hash = checkpoint::sha256_hex(b"");
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_sha256_hex_hello() {
        let hash = checkpoint::sha256_hex(b"hello");
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    // --- Tree walking ---

    #[test]
    fn test_walk_tree_excludes_decree_dir() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);

        let paths = checkpoint::walk_tree(&root).unwrap();
        let rel_paths: Vec<String> = paths
            .iter()
            .map(|p| {
                p.strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();

        // Should NOT contain anything under .decree/
        for p in &rel_paths {
            assert!(
                !p.starts_with(".decree/") && !p.starts_with(".decree"),
                "walk_tree should exclude .decree paths, found: {p}"
            );
        }
    }

    #[test]
    fn test_walk_tree_includes_source_files() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);

        let paths = checkpoint::walk_tree(&root).unwrap();
        let rel_paths: Vec<String> = paths
            .iter()
            .map(|p| {
                p.strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();

        assert!(rel_paths.contains(&"src/main.rs".to_string()));
        assert!(rel_paths.contains(&"src/lib.rs".to_string()));
        assert!(rel_paths.contains(&"README.md".to_string()));
    }

    #[test]
    fn test_walk_tree_respects_gitignore() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);

        // Create a .gitignore that ignores *.log files.
        fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        fs::write(root.join("build.log"), "some log content").unwrap();
        fs::write(root.join("output.log"), "another log").unwrap();

        let paths = checkpoint::walk_tree(&root).unwrap();
        let rel_paths: Vec<String> = paths
            .iter()
            .map(|p| {
                p.strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();

        assert!(
            !rel_paths.contains(&"build.log".to_string()),
            "build.log should be excluded by .gitignore"
        );
        assert!(
            !rel_paths.contains(&"output.log".to_string()),
            "output.log should be excluded by .gitignore"
        );
        // Source files should still be included.
        assert!(rel_paths.contains(&"src/main.rs".to_string()));
    }

    #[test]
    fn test_walk_tree_respects_decreeignore() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);

        fs::write(root.join(".decreeignore"), "*.tmp\n").unwrap();
        fs::write(root.join("scratch.tmp"), "temp content").unwrap();

        let paths = checkpoint::walk_tree(&root).unwrap();
        let rel_paths: Vec<String> = paths
            .iter()
            .map(|p| {
                p.strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();

        assert!(
            !rel_paths.contains(&"scratch.tmp".to_string()),
            "scratch.tmp should be excluded by .decreeignore"
        );
    }

    #[test]
    fn test_walk_tree_excludes_git_dir() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);

        // Create a fake .git directory.
        fs::create_dir_all(root.join(".git/objects")).unwrap();
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/main").unwrap();

        let paths = checkpoint::walk_tree(&root).unwrap();
        let rel_paths: Vec<String> = paths
            .iter()
            .map(|p| {
                p.strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();

        for p in &rel_paths {
            assert!(
                !p.starts_with(".git/") && *p != ".git",
                "walk_tree should exclude .git paths, found: {p}"
            );
        }
    }

    // --- Manifest ---

    #[test]
    fn test_create_manifest_records_all_files() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);

        let manifest = checkpoint::create_manifest(&root).unwrap();

        assert!(manifest.files.contains_key("src/main.rs"));
        assert!(manifest.files.contains_key("src/lib.rs"));
        assert!(manifest.files.contains_key("README.md"));
    }

    #[test]
    fn test_create_manifest_correct_hash() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);

        let manifest = checkpoint::create_manifest(&root).unwrap();
        let entry = &manifest.files["src/main.rs"];

        let expected_hash = checkpoint::sha256_hex(b"fn main() {}\n");
        assert_eq!(entry.sha256, expected_hash);
    }

    #[test]
    fn test_create_manifest_correct_size() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);

        let manifest = checkpoint::create_manifest(&root).unwrap();
        let entry = &manifest.files["src/main.rs"];

        assert_eq!(entry.size, b"fn main() {}\n".len() as u64);
    }

    #[test]
    fn test_create_manifest_excludes_decree() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);

        let manifest = checkpoint::create_manifest(&root).unwrap();

        for key in manifest.files.keys() {
            assert!(
                !key.starts_with(".decree/"),
                "manifest should exclude .decree paths, found: {key}"
            );
        }
    }

    #[test]
    fn test_manifest_save_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        let manifest_path = root.join(".decree/runs/test-0/manifest.json");

        let manifest = checkpoint::create_manifest(&root).unwrap();
        checkpoint::save_manifest(&manifest, &manifest_path).unwrap();

        let loaded = checkpoint::load_manifest(&manifest_path).unwrap();
        assert_eq!(manifest, loaded);
    }

    #[test]
    fn test_manifest_json_format() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        let manifest_path = root.join(".decree/runs/test-0/manifest.json");

        let manifest = checkpoint::create_manifest(&root).unwrap();
        checkpoint::save_manifest(&manifest, &manifest_path).unwrap();

        let content = fs::read_to_string(&manifest_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert!(parsed["files"].is_object());
        let files = parsed["files"].as_object().unwrap();
        // Check that each file entry has the required fields.
        for (_name, entry) in files {
            assert!(entry["sha256"].is_string());
            assert!(entry["size"].is_number());
            assert!(entry["mode"].is_string());
        }
    }

    // --- Checkpoint creation ---

    #[test]
    fn test_save_checkpoint_creates_manifest_file() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        let msg_dir = root.join(".decree/runs/2025022514320000-0");

        let _manifest = checkpoint::save_checkpoint(&root, &msg_dir).unwrap();

        assert!(msg_dir.join("manifest.json").exists());
    }

    #[test]
    fn test_create_checkpoint_returns_manifest_and_cache() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        let msg_dir = root.join(".decree/runs/2025022514320000-0");

        let cp = checkpoint::create_checkpoint(&root, &msg_dir).unwrap();

        assert!(cp.manifest.files.contains_key("src/main.rs"));
        assert!(cp.content_cache.contains_key("src/main.rs"));
        assert_eq!(
            cp.content_cache["src/main.rs"],
            b"fn main() {}\n"
        );
    }

    // --- Diff generation ---

    #[test]
    fn test_diff_no_changes_produces_empty_diff() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        let msg_dir = root.join(".decree/runs/test-0");

        let cp = checkpoint::create_checkpoint(&root, &msg_dir).unwrap();
        // No changes made.
        let diff = checkpoint::finalize_diff(&root, &cp, &msg_dir).unwrap();

        assert!(diff.is_empty(), "no changes should produce empty diff");
        assert!(msg_dir.join("changes.diff").exists());
    }

    #[test]
    fn test_diff_modified_file() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        let msg_dir = root.join(".decree/runs/test-0");

        let cp = checkpoint::create_checkpoint(&root, &msg_dir).unwrap();

        // Modify a file.
        fs::write(root.join("src/main.rs"), "fn main() { println!(\"hello\"); }\n").unwrap();

        let diff = checkpoint::finalize_diff(&root, &cp, &msg_dir).unwrap();

        assert!(diff.contains("src/main.rs"), "diff should mention modified file");
        assert!(diff.contains("---"), "diff should have --- header");
        assert!(diff.contains("+++"), "diff should have +++ header");
    }

    #[test]
    fn test_diff_new_file() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        let msg_dir = root.join(".decree/runs/test-0");

        let cp = checkpoint::create_checkpoint(&root, &msg_dir).unwrap();

        // Add a new file.
        fs::write(root.join("src/utils.rs"), "pub fn add(a: i32, b: i32) -> i32 { a + b }\n").unwrap();

        let diff = checkpoint::finalize_diff(&root, &cp, &msg_dir).unwrap();

        assert!(diff.contains("src/utils.rs"), "diff should mention new file");
        assert!(diff.contains("/dev/null"), "new file diff should reference /dev/null");
    }

    #[test]
    fn test_diff_deleted_file() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        let msg_dir = root.join(".decree/runs/test-0");

        let cp = checkpoint::create_checkpoint(&root, &msg_dir).unwrap();

        // Delete a file.
        fs::remove_file(root.join("README.md")).unwrap();

        let diff = checkpoint::finalize_diff(&root, &cp, &msg_dir).unwrap();

        assert!(diff.contains("README.md"), "diff should mention deleted file");
        assert!(diff.contains("/dev/null"), "deleted file diff should reference /dev/null");
        // The old content should appear in the diff.
        assert!(diff.contains("# My Project"), "diff should include old content of deleted file");
    }

    #[test]
    fn test_diff_multiple_changes() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        let msg_dir = root.join(".decree/runs/test-0");

        let cp = checkpoint::create_checkpoint(&root, &msg_dir).unwrap();

        // Modify, add, delete.
        fs::write(root.join("src/main.rs"), "fn main() { println!(\"changed\"); }\n").unwrap();
        fs::write(root.join("new_file.txt"), "brand new\n").unwrap();
        fs::remove_file(root.join("README.md")).unwrap();

        let diff = checkpoint::finalize_diff(&root, &cp, &msg_dir).unwrap();

        assert!(diff.contains("src/main.rs"));
        assert!(diff.contains("new_file.txt"));
        assert!(diff.contains("README.md"));
    }

    #[test]
    fn test_diff_binary_file() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        let msg_dir = root.join(".decree/runs/test-0");

        let cp = checkpoint::create_checkpoint(&root, &msg_dir).unwrap();

        // Add a binary file (contains null bytes).
        let binary_content: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 0x00, 0x01, 0x02];
        fs::write(root.join("image.png"), &binary_content).unwrap();

        let diff = checkpoint::finalize_diff(&root, &cp, &msg_dir).unwrap();

        assert!(diff.contains("Binary files"), "diff should note binary files");
        assert!(diff.contains("Base64-Content:"), "diff should include base64-encoded content");
    }

    #[test]
    fn test_diff_is_human_readable_unified_format() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        let msg_dir = root.join(".decree/runs/test-0");

        let cp = checkpoint::create_checkpoint(&root, &msg_dir).unwrap();

        // Modify file.
        fs::write(
            root.join("src/lib.rs"),
            "pub fn hello() -> &'static str { \"world\" }\n",
        ).unwrap();

        let diff = checkpoint::finalize_diff(&root, &cp, &msg_dir).unwrap();

        // Standard unified diff indicators.
        assert!(diff.contains("@@"), "unified diff should contain @@ hunk headers");
    }

    // --- Revert ---

    #[test]
    fn test_revert_modified_file() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        let msg_dir = root.join(".decree/runs/test-0");

        let cp = checkpoint::create_checkpoint(&root, &msg_dir).unwrap();

        // Modify.
        fs::write(root.join("src/main.rs"), "fn main() { panic!(); }\n").unwrap();

        // Revert.
        checkpoint::revert_to_checkpoint(&root, &cp).unwrap();

        let content = fs::read_to_string(root.join("src/main.rs")).unwrap();
        assert_eq!(content, "fn main() {}\n");
    }

    #[test]
    fn test_revert_new_file_deleted() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        let msg_dir = root.join(".decree/runs/test-0");

        let cp = checkpoint::create_checkpoint(&root, &msg_dir).unwrap();

        // Add a new file.
        fs::write(root.join("src/extra.rs"), "// extra\n").unwrap();

        // Revert.
        checkpoint::revert_to_checkpoint(&root, &cp).unwrap();

        assert!(
            !root.join("src/extra.rs").exists(),
            "new file should be deleted after revert"
        );
    }

    #[test]
    fn test_revert_deleted_file_restored() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        let msg_dir = root.join(".decree/runs/test-0");

        let cp = checkpoint::create_checkpoint(&root, &msg_dir).unwrap();

        // Delete a file.
        fs::remove_file(root.join("README.md")).unwrap();

        // Revert.
        checkpoint::revert_to_checkpoint(&root, &cp).unwrap();

        assert!(root.join("README.md").exists(), "deleted file should be restored");
        let content = fs::read_to_string(root.join("README.md")).unwrap();
        assert_eq!(content, "# My Project\n");
    }

    #[test]
    fn test_revert_complex_changes() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        let msg_dir = root.join(".decree/runs/test-0");

        let cp = checkpoint::create_checkpoint(&root, &msg_dir).unwrap();

        // Simulate a routine that modifies, adds, and deletes.
        fs::write(root.join("src/main.rs"), "fn main() { unreachable!() }\n").unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn goodbye() {}\n").unwrap();
        fs::write(root.join("new_module.rs"), "mod new;\n").unwrap();
        fs::create_dir_all(root.join("generated")).unwrap();
        fs::write(root.join("generated/output.txt"), "generated content").unwrap();
        fs::remove_file(root.join("README.md")).unwrap();

        // Revert.
        checkpoint::revert_to_checkpoint(&root, &cp).unwrap();

        // Modified files restored.
        assert_eq!(
            fs::read_to_string(root.join("src/main.rs")).unwrap(),
            "fn main() {}\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("src/lib.rs")).unwrap(),
            "pub fn hello() -> &'static str { \"hello\" }\n"
        );

        // New files removed.
        assert!(!root.join("new_module.rs").exists());
        assert!(!root.join("generated/output.txt").exists());

        // Deleted file restored.
        assert_eq!(
            fs::read_to_string(root.join("README.md")).unwrap(),
            "# My Project\n"
        );
    }

    #[test]
    fn test_revert_no_changes_is_noop() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        let msg_dir = root.join(".decree/runs/test-0");

        let cp = checkpoint::create_checkpoint(&root, &msg_dir).unwrap();

        // No changes — revert should succeed silently.
        checkpoint::revert_to_checkpoint(&root, &cp).unwrap();

        assert_eq!(
            fs::read_to_string(root.join("src/main.rs")).unwrap(),
            "fn main() {}\n"
        );
    }

    // --- Integrity verification ---

    #[test]
    fn test_verify_integrity_passes_after_revert() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        let msg_dir = root.join(".decree/runs/test-0");

        let cp = checkpoint::create_checkpoint(&root, &msg_dir).unwrap();

        // Modify then revert.
        fs::write(root.join("src/main.rs"), "fn main() { panic!(); }\n").unwrap();
        checkpoint::revert_to_checkpoint(&root, &cp).unwrap();

        // Explicit integrity check.
        let affected = vec!["src/main.rs"];
        let result = checkpoint::verify_integrity(&root, &cp.manifest, &affected);
        assert!(result.is_ok());
    }

    #[test]
    fn test_verify_integrity_fails_on_mismatch() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);

        let manifest = checkpoint::create_manifest(&root).unwrap();

        // Corrupt a file.
        fs::write(root.join("src/main.rs"), "CORRUPTED").unwrap();

        let affected = vec!["src/main.rs"];
        let result = checkpoint::verify_integrity(&root, &manifest, &affected);
        assert!(result.is_err());

        match result.unwrap_err() {
            decree::error::DecreeError::CheckpointIntegrity(mismatches) => {
                assert_eq!(mismatches.len(), 1);
                assert!(mismatches[0].contains("src/main.rs"));
            }
            other => panic!("expected CheckpointIntegrity, got: {other}"),
        }
    }

    #[test]
    fn test_verify_integrity_fails_missing_file() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);

        let manifest = checkpoint::create_manifest(&root).unwrap();

        // Remove a file that should exist.
        fs::remove_file(root.join("src/main.rs")).unwrap();

        let affected = vec!["src/main.rs"];
        let result = checkpoint::verify_integrity(&root, &manifest, &affected);
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_integrity_fails_unexpected_file() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);

        let manifest = checkpoint::create_manifest(&root).unwrap();

        // Create a file that shouldn't exist (not in manifest).
        fs::write(root.join("rogue.txt"), "unexpected").unwrap();

        let affected = vec!["rogue.txt"];
        let result = checkpoint::verify_integrity(&root, &manifest, &affected);
        assert!(result.is_err());
    }

    // --- Content cache ---

    #[test]
    fn test_content_cache_captures_all_files() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);

        let cache = checkpoint::capture_content_cache(&root).unwrap();

        assert!(cache.contains_key("src/main.rs"));
        assert!(cache.contains_key("src/lib.rs"));
        assert!(cache.contains_key("README.md"));
        assert_eq!(cache["src/main.rs"], b"fn main() {}\n");
    }

    #[test]
    fn test_content_cache_excludes_decree() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);

        let cache = checkpoint::capture_content_cache(&root).unwrap();

        for key in cache.keys() {
            assert!(
                !key.starts_with(".decree/"),
                "cache should exclude .decree paths, found: {key}"
            );
        }
    }

    // --- Full workflow: checkpoint -> execute -> diff -> revert ---

    #[test]
    fn test_full_workflow_success() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        let msg_dir = root.join(".decree/runs/2025022514320000-0");

        // 1. Create checkpoint before execution.
        let cp = checkpoint::create_checkpoint(&root, &msg_dir).unwrap();
        assert!(msg_dir.join("manifest.json").exists());

        // 2. Simulate routine execution: modify files.
        fs::write(root.join("src/main.rs"), "fn main() { println!(\"done\"); }\n").unwrap();

        // 3. Generate diff after execution.
        let diff = checkpoint::finalize_diff(&root, &cp, &msg_dir).unwrap();
        assert!(msg_dir.join("changes.diff").exists());
        assert!(!diff.is_empty());
    }

    #[test]
    fn test_full_workflow_failure_revert() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        let msg_dir = root.join(".decree/runs/2025022514320000-0");

        // 1. Checkpoint.
        let cp = checkpoint::create_checkpoint(&root, &msg_dir).unwrap();

        // 2. Simulate routine that fails after partial work.
        fs::write(root.join("src/main.rs"), "fn main() { BROKEN }\n").unwrap();
        fs::write(root.join("half_done.rs"), "// incomplete\n").unwrap();
        fs::remove_file(root.join("README.md")).unwrap();

        // 3. Generate diff (partial work).
        let _diff = checkpoint::finalize_diff(&root, &cp, &msg_dir).unwrap();

        // 4. Revert on final failure.
        checkpoint::revert_to_checkpoint(&root, &cp).unwrap();

        // Verify everything is back to original.
        assert_eq!(
            fs::read_to_string(root.join("src/main.rs")).unwrap(),
            "fn main() {}\n"
        );
        assert!(!root.join("half_done.rs").exists());
        assert_eq!(
            fs::read_to_string(root.join("README.md")).unwrap(),
            "# My Project\n"
        );
    }

    #[test]
    fn test_works_without_git_directory() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        // Explicitly no .git/ directory.
        let msg_dir = root.join(".decree/runs/test-0");

        let cp = checkpoint::create_checkpoint(&root, &msg_dir).unwrap();

        fs::write(root.join("src/main.rs"), "fn main() { changed(); }\n").unwrap();

        let diff = checkpoint::finalize_diff(&root, &cp, &msg_dir).unwrap();
        assert!(!diff.is_empty());

        checkpoint::revert_to_checkpoint(&root, &cp).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("src/main.rs")).unwrap(),
            "fn main() {}\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_manifest_records_file_mode() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);

        // Make a file executable.
        let script = root.join("run.sh");
        fs::write(&script, "#!/bin/bash\necho hi\n").unwrap();
        fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let manifest = checkpoint::create_manifest(&root).unwrap();
        let entry = &manifest.files["run.sh"];

        assert_eq!(entry.mode, "755");
    }

    #[cfg(unix)]
    #[test]
    fn test_revert_restores_file_mode() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        let msg_dir = root.join(".decree/runs/test-0");

        // Create an executable file before checkpoint.
        let script = root.join("run.sh");
        fs::write(&script, "#!/bin/bash\necho original\n").unwrap();
        fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let cp = checkpoint::create_checkpoint(&root, &msg_dir).unwrap();

        // Modify the file (content change).
        fs::write(&script, "#!/bin/bash\necho modified\n").unwrap();

        // Revert.
        checkpoint::revert_to_checkpoint(&root, &cp).unwrap();

        let content = fs::read_to_string(&script).unwrap();
        assert_eq!(content, "#!/bin/bash\necho original\n");

        let mode = fs::metadata(&script).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
    }

    #[test]
    fn test_manifest_excludes_gitignored_files() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);

        fs::write(root.join(".gitignore"), "target/\n*.o\n").unwrap();
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::write(root.join("target/debug/main"), "binary").unwrap();
        fs::write(root.join("module.o"), "object file").unwrap();

        let manifest = checkpoint::create_manifest(&root).unwrap();

        assert!(!manifest.files.contains_key("target/debug/main"));
        assert!(!manifest.files.contains_key("module.o"));
        assert!(manifest.files.contains_key("src/main.rs"));
    }

    #[test]
    fn test_diff_written_to_correct_path() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        let msg_dir = root.join(".decree/runs/2025022514320000-1");

        let cp = checkpoint::create_checkpoint(&root, &msg_dir).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() { changed(); }\n").unwrap();
        checkpoint::finalize_diff(&root, &cp, &msg_dir).unwrap();

        let diff_path = msg_dir.join("changes.diff");
        assert!(diff_path.exists());
        let diff_content = fs::read_to_string(&diff_path).unwrap();
        assert!(diff_content.contains("src/main.rs"));
    }

    #[test]
    fn test_new_file_in_new_directory_reverted() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);
        let msg_dir = root.join(".decree/runs/test-0");

        let cp = checkpoint::create_checkpoint(&root, &msg_dir).unwrap();

        // Create a file in a new directory.
        fs::create_dir_all(root.join("deep/nested/dir")).unwrap();
        fs::write(root.join("deep/nested/dir/file.txt"), "content").unwrap();

        checkpoint::revert_to_checkpoint(&root, &cp).unwrap();

        assert!(!root.join("deep/nested/dir/file.txt").exists());
    }

    #[test]
    fn test_checkpoint_spec_fields_per_file() {
        let tmp = TempDir::new().unwrap();
        let root = setup_checkpoint_project(&tmp);

        let manifest = checkpoint::create_manifest(&root).unwrap();

        for (path, entry) in &manifest.files {
            // sha256 must be 64 hex chars.
            assert_eq!(
                entry.sha256.len(),
                64,
                "sha256 for {path} should be 64 chars, got {}",
                entry.sha256.len()
            );
            assert!(
                entry.sha256.chars().all(|c| c.is_ascii_hexdigit()),
                "sha256 for {path} should be hex"
            );

            // size must be > 0 (our test files aren't empty).
            // mode must be a valid octal string.
            assert!(
                u32::from_str_radix(&entry.mode, 8).is_ok(),
                "mode for {path} should be valid octal, got: {}",
                entry.mode
            );
        }
    }
}

// ==========================================================================
// Spec 05: Message Format and Normalization
// ==========================================================================

mod spec_processing_tests {
    use super::*;

    #[test]
    fn test_list_specs_alphabetical() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        fs::write(root.join("specs/03-third.spec.md"), "third").unwrap();
        fs::write(root.join("specs/01-first.spec.md"), "first").unwrap();
        fs::write(root.join("specs/02-second.spec.md"), "second").unwrap();

        let specs = decree::spec::list_specs(&root).unwrap();
        assert_eq!(
            specs,
            vec![
                "01-first.spec.md",
                "02-second.spec.md",
                "03-third.spec.md"
            ]
        );
    }

    #[test]
    fn test_read_processed_creates_file() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        // Remove the processed file that setup_project creates
        let path = root.join("specs/processed-spec.md");
        fs::remove_file(&path).unwrap();
        assert!(!path.exists());

        let processed = decree::spec::read_processed(&root).unwrap();
        assert!(processed.is_empty());
        assert!(path.exists());
    }

    #[test]
    fn test_read_processed_existing() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        fs::write(
            root.join("specs/processed-spec.md"),
            "01-first.spec.md\n02-second.spec.md\n",
        )
        .unwrap();

        let processed = decree::spec::read_processed(&root).unwrap();
        assert_eq!(processed.len(), 2);
        assert!(processed.contains("01-first.spec.md"));
        assert!(processed.contains("02-second.spec.md"));
    }

    #[test]
    fn test_next_unprocessed_skips_processed() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        fs::write(root.join("specs/01-first.spec.md"), "first").unwrap();
        fs::write(root.join("specs/02-second.spec.md"), "second").unwrap();
        fs::write(root.join("specs/03-third.spec.md"), "third").unwrap();
        fs::write(
            root.join("specs/processed-spec.md"),
            "01-first.spec.md\n",
        )
        .unwrap();

        let next = decree::spec::next_unprocessed(&root).unwrap();
        assert_eq!(next, Some("02-second.spec.md".to_string()));
    }

    #[test]
    fn test_next_unprocessed_none_left() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        fs::write(root.join("specs/01-first.spec.md"), "first").unwrap();
        fs::write(
            root.join("specs/processed-spec.md"),
            "01-first.spec.md\n",
        )
        .unwrap();

        let next = decree::spec::next_unprocessed(&root).unwrap();
        assert!(next.is_none());
    }

    #[test]
    fn test_mark_processed() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        decree::spec::mark_processed(&root, "01-first.spec.md").unwrap();
        decree::spec::mark_processed(&root, "02-second.spec.md").unwrap();

        let processed = decree::spec::read_processed(&root).unwrap();
        assert!(processed.contains("01-first.spec.md"));
        assert!(processed.contains("02-second.spec.md"));
    }

    #[test]
    fn test_new_spec_picked_up_after_previous_run() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        fs::write(root.join("specs/01-first.spec.md"), "first").unwrap();
        fs::write(
            root.join("specs/processed-spec.md"),
            "01-first.spec.md\n",
        )
        .unwrap();

        // No more specs to process
        assert!(decree::spec::next_unprocessed(&root).unwrap().is_none());

        // Add a new spec
        fs::write(root.join("specs/02-second.spec.md"), "second").unwrap();

        // Now it's picked up
        let next = decree::spec::next_unprocessed(&root).unwrap();
        assert_eq!(next, Some("02-second.spec.md".to_string()));
    }

    #[test]
    fn test_parse_spec_frontmatter_with_routine() {
        let content = "---\nroutine: custom\n---\n# My Spec\nDo things.";
        let fm = decree::spec::parse_spec_frontmatter(content);
        assert_eq!(fm.routine, Some("custom".to_string()));
    }

    #[test]
    fn test_parse_spec_frontmatter_no_frontmatter() {
        let content = "# My Spec\nNo frontmatter here.";
        let fm = decree::spec::parse_spec_frontmatter(content);
        assert!(fm.routine.is_none());
    }
}

mod message_format_tests {
    #[test]
    fn test_parse_fully_structured_message() {
        let content = "\
---
id: 2025022514320000-0
chain: 2025022514320000
seq: 0
type: spec
input_file: specs/01-add-auth.spec.md
routine: develop
---
";
        let (fm, body) = decree::message::parse_message_file(content);
        assert_eq!(fm.id.as_deref(), Some("2025022514320000-0"));
        assert_eq!(fm.chain.as_deref(), Some("2025022514320000"));
        assert_eq!(fm.seq, Some(0));
        assert_eq!(fm.msg_type.as_deref(), Some("spec"));
        assert_eq!(
            fm.input_file.as_deref(),
            Some("specs/01-add-auth.spec.md")
        );
        assert_eq!(fm.routine.as_deref(), Some("develop"));
        assert!(body.is_empty());
    }

    #[test]
    fn test_parse_minimal_message() {
        let content = "\
---
chain: 2025022514320000
seq: 1
---
Fix type errors in src/auth.rs introduced by the auth implementation.
";
        let (fm, body) = decree::message::parse_message_file(content);
        assert_eq!(fm.chain.as_deref(), Some("2025022514320000"));
        assert_eq!(fm.seq, Some(1));
        assert!(fm.id.is_none());
        assert!(fm.msg_type.is_none());
        assert!(fm.routine.is_none());
        assert!(body.contains("Fix type errors"));
    }

    #[test]
    fn test_parse_bare_message() {
        let content = "Fix type errors in src/auth.rs introduced by the auth implementation.\n";
        let (fm, body) = decree::message::parse_message_file(content);
        assert!(fm.id.is_none());
        assert!(fm.chain.is_none());
        assert!(fm.seq.is_none());
        assert!(fm.msg_type.is_none());
        assert!(fm.routine.is_none());
        assert!(body.contains("Fix type errors"));
    }

    #[test]
    fn test_parse_message_with_custom_fields() {
        let content = "\
---
chain: 2025022514320000
seq: 0
type: task
routine: develop
custom_var: hello
---
Some body text.
";
        let (fm, body) = decree::message::parse_message_file(content);
        assert_eq!(fm.chain.as_deref(), Some("2025022514320000"));
        assert!(fm.custom_fields.contains_key("custom_var"));
        assert!(body.contains("Some body text"));
    }

    #[test]
    fn test_chain_seq_from_filename() {
        let result = decree::message::chain_seq_from_filename("2025022514320000-0.md");
        assert_eq!(result, Some(("2025022514320000".to_string(), 0)));
    }

    #[test]
    fn test_chain_seq_from_filename_higher_seq() {
        let result = decree::message::chain_seq_from_filename("2025022514320000-5.md");
        assert_eq!(result, Some(("2025022514320000".to_string(), 5)));
    }

    #[test]
    fn test_chain_seq_from_invalid_filename() {
        assert!(decree::message::chain_seq_from_filename("random-name.md").is_none());
        assert!(decree::message::chain_seq_from_filename("notes.txt").is_none());
    }

    #[test]
    fn test_serialize_message_roundtrip() {
        let msg = decree::message::InboxMessage {
            id: "2025022514320000-0".to_string(),
            chain: "2025022514320000".to_string(),
            seq: 0,
            msg_type: decree::message::MessageType::Task,
            input_file: None,
            routine: "develop".to_string(),
            body: "Fix type errors in src/auth.rs\n".to_string(),
            custom_fields: std::collections::BTreeMap::new(),
        };

        let serialized = decree::message::serialize_message(&msg);
        let (fm, body) = decree::message::parse_message_file(&serialized);
        assert_eq!(fm.id.as_deref(), Some("2025022514320000-0"));
        assert_eq!(fm.chain.as_deref(), Some("2025022514320000"));
        assert_eq!(fm.seq, Some(0));
        assert_eq!(fm.msg_type.as_deref(), Some("task"));
        assert_eq!(fm.routine.as_deref(), Some("develop"));
        assert!(body.contains("Fix type errors"));
    }

    #[test]
    fn test_serialize_message_with_input_file() {
        let msg = decree::message::InboxMessage {
            id: "2025022514320000-0".to_string(),
            chain: "2025022514320000".to_string(),
            seq: 0,
            msg_type: decree::message::MessageType::Spec,
            input_file: Some("specs/01-add-auth.spec.md".to_string()),
            routine: "develop".to_string(),
            body: String::new(),
            custom_fields: std::collections::BTreeMap::new(),
        };

        let serialized = decree::message::serialize_message(&msg);
        assert!(serialized.contains("input_file: specs/01-add-auth.spec.md"));
        assert!(serialized.contains("type: spec"));
    }
}

mod normalization_tests {
    use super::*;

    fn setup_config() -> decree::config::Config {
        decree::config::Config::default()
    }

    #[test]
    fn test_normalize_fully_structured_is_noop() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let content = "\
---
id: 2025022514320000-0
chain: 2025022514320000
seq: 0
type: spec
input_file: specs/01-add-auth.spec.md
routine: develop
---
";
        let inbox_dir = root.join(".decree/inbox");
        let file_path = inbox_dir.join("2025022514320000-0.md");
        fs::write(&file_path, content).unwrap();

        let config = setup_config();
        let routines = vec![];
        let msg = decree::message::normalize_message(
            &file_path,
            &config,
            &routines,
            None,
            None,
        )
        .unwrap();

        assert_eq!(msg.id, "2025022514320000-0");
        assert_eq!(msg.chain, "2025022514320000");
        assert_eq!(msg.seq, 0);
        assert_eq!(msg.msg_type, decree::message::MessageType::Spec);
        assert_eq!(msg.routine, "develop");

        // File should NOT be rewritten — content identical
        let after = fs::read_to_string(&file_path).unwrap();
        assert_eq!(after, content);
    }

    #[test]
    fn test_normalize_bare_message_derives_from_filename() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let content = "Fix type errors in src/auth.rs\n";
        let inbox_dir = root.join(".decree/inbox");
        let file_path = inbox_dir.join("2025022514320000-1.md");
        fs::write(&file_path, content).unwrap();

        let config = setup_config();
        let routines = vec![];
        let msg = decree::message::normalize_message(
            &file_path,
            &config,
            &routines,
            None,
            None,
        )
        .unwrap();

        assert_eq!(msg.chain, "2025022514320000");
        assert_eq!(msg.seq, 1);
        assert_eq!(msg.id, "2025022514320000-1");
        assert_eq!(msg.msg_type, decree::message::MessageType::Task);
        assert_eq!(msg.routine, "develop"); // fallback default
        assert!(msg.body.contains("Fix type errors"));

        // File should be rewritten with frontmatter
        let after = fs::read_to_string(&file_path).unwrap();
        assert!(after.starts_with("---\n"));
        assert!(after.contains("id: 2025022514320000-1"));
        assert!(after.contains("type: task"));
        assert!(after.contains("Fix type errors"));
    }

    #[test]
    fn test_normalize_partial_frontmatter_fills_missing() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let content = "\
---
chain: 2025022514320000
seq: 1
---
Fix type errors in src/auth.rs
";
        let inbox_dir = root.join(".decree/inbox");
        let file_path = inbox_dir.join("2025022514320000-1.md");
        fs::write(&file_path, content).unwrap();

        let config = setup_config();
        let routines = vec![];
        let msg = decree::message::normalize_message(
            &file_path,
            &config,
            &routines,
            None,
            None,
        )
        .unwrap();

        // Existing fields preserved
        assert_eq!(msg.chain, "2025022514320000");
        assert_eq!(msg.seq, 1);
        // Derived fields filled in
        assert_eq!(msg.id, "2025022514320000-1");
        assert_eq!(msg.msg_type, decree::message::MessageType::Task);
        assert_eq!(msg.routine, "develop");
    }

    #[test]
    fn test_normalize_with_input_file_infers_spec_type() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let content = "\
---
chain: 2025022514320000
seq: 0
input_file: specs/01-add-auth.spec.md
---
";
        let inbox_dir = root.join(".decree/inbox");
        let file_path = inbox_dir.join("2025022514320000-0.md");
        fs::write(&file_path, content).unwrap();

        let config = setup_config();
        let routines = vec![];
        let msg = decree::message::normalize_message(
            &file_path,
            &config,
            &routines,
            None,
            None,
        )
        .unwrap();

        assert_eq!(msg.msg_type, decree::message::MessageType::Spec);
        assert_eq!(
            msg.input_file.as_deref(),
            Some("specs/01-add-auth.spec.md")
        );
    }

    #[test]
    fn test_normalize_router_selects_routine() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let content = "Deploy the application to staging\n";
        let inbox_dir = root.join(".decree/inbox");
        let file_path = inbox_dir.join("2025022514320000-0.md");
        fs::write(&file_path, content).unwrap();

        let config = setup_config();
        let routines = vec![
            decree::routine::RoutineInfo {
                name: "develop".to_string(),
                description: "Default development routine".to_string(),
            },
            decree::routine::RoutineInfo {
                name: "deploy".to_string(),
                description: "Deploy to staging/production".to_string(),
            },
        ];

        // Router returns "deploy"
        let router_fn: Box<decree::message::RouterFn> =
            Box::new(|_prompt: &str| Ok("deploy".to_string()));

        let msg = decree::message::normalize_message(
            &file_path,
            &config,
            &routines,
            Some(router_fn),
            None,
        )
        .unwrap();

        assert_eq!(msg.routine, "deploy");
    }

    #[test]
    fn test_normalize_router_failure_falls_back() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let content = "Do something\n";
        let inbox_dir = root.join(".decree/inbox");
        let file_path = inbox_dir.join("2025022514320000-0.md");
        fs::write(&file_path, content).unwrap();

        let config = setup_config();
        let routines = vec![decree::routine::RoutineInfo {
            name: "develop".to_string(),
            description: "Default development routine".to_string(),
        }];

        // Router returns unrecognized name
        let router_fn: Box<decree::message::RouterFn> =
            Box::new(|_prompt: &str| Ok("nonexistent".to_string()));

        let msg = decree::message::normalize_message(
            &file_path,
            &config,
            &routines,
            Some(router_fn),
            Some("custom"),
        )
        .unwrap();

        // Falls back to spec frontmatter routine
        assert_eq!(msg.routine, "custom");
    }

    #[test]
    fn test_normalize_router_error_falls_back_to_config_default() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let content = "Do something\n";
        let inbox_dir = root.join(".decree/inbox");
        let file_path = inbox_dir.join("2025022514320000-0.md");
        fs::write(&file_path, content).unwrap();

        let mut config = setup_config();
        config.default_routine = "my-default".to_string();
        let routines = vec![decree::routine::RoutineInfo {
            name: "develop".to_string(),
            description: "Default development routine".to_string(),
        }];

        // Router returns error
        let router_fn: Box<decree::message::RouterFn> = Box::new(|_prompt: &str| {
            Err(decree::error::DecreeError::Config("router failed".into()))
        });

        let msg = decree::message::normalize_message(
            &file_path,
            &config,
            &routines,
            Some(router_fn),
            None, // no spec routine
        )
        .unwrap();

        // Falls back to config default
        assert_eq!(msg.routine, "my-default");
    }

    #[test]
    fn test_normalize_non_standard_filename_generates_chain() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let content = "Fix something\n";
        let inbox_dir = root.join(".decree/inbox");
        let file_path = inbox_dir.join("random-task.md");
        fs::write(&file_path, content).unwrap();

        let config = setup_config();
        let routines = vec![];
        let msg = decree::message::normalize_message(
            &file_path,
            &config,
            &routines,
            None,
            None,
        )
        .unwrap();

        // Chain should be generated (16 chars), seq defaults to 0
        assert_eq!(msg.chain.len(), 16);
        assert_eq!(msg.seq, 0);
        assert_eq!(msg.id, format!("{}-0", msg.chain));
    }

    #[test]
    fn test_normalize_preserves_custom_fields() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let content = "\
---
chain: 2025022514320000
seq: 0
custom_var: hello
---
Task body.
";
        let inbox_dir = root.join(".decree/inbox");
        let file_path = inbox_dir.join("2025022514320000-0.md");
        fs::write(&file_path, content).unwrap();

        let config = setup_config();
        let routines = vec![];
        let msg = decree::message::normalize_message(
            &file_path,
            &config,
            &routines,
            None,
            None,
        )
        .unwrap();

        assert!(msg.custom_fields.contains_key("custom_var"));
        // Verify custom field is in the rewritten file
        let after = fs::read_to_string(&file_path).unwrap();
        assert!(after.contains("custom_var: hello"));
    }

    #[test]
    fn test_normalize_preserves_body() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let body_text = "Fix type errors in src/auth.rs\n\nAlso update the docs.\n";
        let content = body_text;
        let inbox_dir = root.join(".decree/inbox");
        let file_path = inbox_dir.join("2025022514320000-0.md");
        fs::write(&file_path, content).unwrap();

        let config = setup_config();
        let routines = vec![];
        let msg = decree::message::normalize_message(
            &file_path,
            &config,
            &routines,
            None,
            None,
        )
        .unwrap();

        assert!(msg.body.contains("Fix type errors"));
        assert!(msg.body.contains("Also update the docs"));

        // Rewritten file preserves body
        let after = fs::read_to_string(&file_path).unwrap();
        assert!(after.contains("Fix type errors"));
        assert!(after.contains("Also update the docs"));
    }
}

mod routine_discovery_tests {
    use super::*;

    #[test]
    fn test_discover_routines_sh_only() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        fs::write(
            root.join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\n# Develop\n#\n# Default development routine.\nset -euo pipefail\n",
        )
        .unwrap();
        fs::write(
            root.join(".decree/routines/deploy.sh"),
            "#!/usr/bin/env bash\n# Deploy\n#\n# Deploy to staging.\nset -euo pipefail\n",
        )
        .unwrap();

        let routines =
            decree::routine::discover_routines(&root, false).unwrap();
        assert_eq!(routines.len(), 2);
        let names: Vec<&str> = routines.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"develop"));
        assert!(names.contains(&"deploy"));
    }

    #[test]
    fn test_discover_routines_ignores_ipynb_when_disabled() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        fs::write(
            root.join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\n# Develop\nset -euo pipefail\n",
        )
        .unwrap();
        fs::write(
            root.join(".decree/routines/notebook.ipynb"),
            r##"{"cells":[{"cell_type":"markdown","source":["# Notebook"]}]}"##,
        )
        .unwrap();

        let routines =
            decree::routine::discover_routines(&root, false).unwrap();
        assert_eq!(routines.len(), 1);
        assert_eq!(routines[0].name, "develop");
    }

    #[test]
    fn test_discover_routines_dedup_with_notebook() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        fs::write(
            root.join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\n# Shell Develop\nset -euo pipefail\n",
        )
        .unwrap();
        fs::write(
            root.join(".decree/routines/develop.ipynb"),
            r##"{"cells":[{"cell_type":"markdown","source":["# Notebook Develop"]}]}"##,
        )
        .unwrap();

        let routines =
            decree::routine::discover_routines(&root, true).unwrap();
        // Should be deduplicated — one entry
        assert_eq!(routines.len(), 1);
        assert_eq!(routines[0].name, "develop");
        // .sh description takes precedence
        assert!(routines[0].description.contains("Shell Develop"));
    }

    #[test]
    fn test_extract_sh_description() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        let script = "\
#!/usr/bin/env bash
# Deploy
#
# Deploy to staging/production environments.
# Handles blue-green deployment strategy.
set -euo pipefail
";
        fs::write(root.join(".decree/routines/deploy.sh"), script).unwrap();

        let routines =
            decree::routine::discover_routines(&root, false).unwrap();
        let deploy = routines.iter().find(|r| r.name == "deploy").unwrap();
        assert!(deploy.description.contains("Deploy"));
        assert!(deploy.description.contains("blue-green"));
    }

    #[test]
    fn test_extract_ipynb_description() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        let notebook = r##"{
            "cells": [
                {
                    "cell_type": "markdown",
                    "source": ["# Analyze\n", "Data analysis routine."],
                    "metadata": {}
                },
                {
                    "cell_type": "code",
                    "source": ["print('hello')"],
                    "metadata": {}
                }
            ],
            "metadata": {},
            "nbformat": 4,
            "nbformat_minor": 5
        }"##;
        fs::write(root.join(".decree/routines/analyze.ipynb"), notebook).unwrap();

        let routines =
            decree::routine::discover_routines(&root, true).unwrap();
        let analyze = routines.iter().find(|r| r.name == "analyze").unwrap();
        assert!(analyze.description.contains("Analyze"));
        assert!(analyze.description.contains("Data analysis"));
    }

    #[test]
    fn test_build_router_prompt() {
        let routines = vec![
            decree::routine::RoutineInfo {
                name: "develop".to_string(),
                description: "Default development routine.\nMore details.".to_string(),
            },
            decree::routine::RoutineInfo {
                name: "deploy".to_string(),
                description: "Deploy to staging.".to_string(),
            },
        ];

        let prompt = decree::routine::build_router_prompt(&routines, "Deploy the app");
        assert!(prompt.contains("## Available Routines"));
        assert!(prompt.contains("- develop: Default development routine."));
        assert!(prompt.contains("- deploy: Deploy to staging."));
        assert!(prompt.contains("## Task"));
        assert!(prompt.contains("Deploy the app"));
        assert!(prompt.contains("Respond with ONLY the routine name"));
    }

    #[test]
    fn test_is_valid_routine() {
        let routines = vec![
            decree::routine::RoutineInfo {
                name: "develop".to_string(),
                description: String::new(),
            },
            decree::routine::RoutineInfo {
                name: "deploy".to_string(),
                description: String::new(),
            },
        ];

        assert!(decree::routine::is_valid_routine(&routines, "develop"));
        assert!(decree::routine::is_valid_routine(&routines, "deploy"));
        assert!(!decree::routine::is_valid_routine(&routines, "nonexistent"));
    }
}

mod routine_resolution_tests {
    use super::*;

    #[test]
    fn test_resolve_sh_routine() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        fs::write(
            root.join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\n# Develop\nset -euo pipefail\n",
        )
        .unwrap();

        let resolved =
            decree::routine::resolve_routine(&root, "develop", false).unwrap();
        assert_eq!(resolved.name, "develop");
        assert_eq!(resolved.format, decree::routine::RoutineFormat::Shell);
        assert!(resolved.path.ends_with("develop.sh"));
    }

    #[test]
    fn test_resolve_ipynb_when_notebook_support_true() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        fs::write(
            root.join(".decree/routines/develop.ipynb"),
            r#"{"cells":[],"metadata":{},"nbformat":4,"nbformat_minor":5}"#,
        )
        .unwrap();

        let resolved =
            decree::routine::resolve_routine(&root, "develop", true).unwrap();
        assert_eq!(resolved.name, "develop");
        assert_eq!(resolved.format, decree::routine::RoutineFormat::Notebook);
        assert!(resolved.path.ends_with("develop.ipynb"));
    }

    #[test]
    fn test_resolve_notebook_precedence_when_both_exist() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        fs::write(
            root.join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\n# Shell Develop\n",
        )
        .unwrap();
        fs::write(
            root.join(".decree/routines/develop.ipynb"),
            r#"{"cells":[],"metadata":{},"nbformat":4,"nbformat_minor":5}"#,
        )
        .unwrap();

        // With notebook support: .ipynb takes precedence
        let resolved =
            decree::routine::resolve_routine(&root, "develop", true).unwrap();
        assert_eq!(resolved.format, decree::routine::RoutineFormat::Notebook);
    }

    #[test]
    fn test_resolve_sh_when_both_exist_notebook_disabled() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        fs::write(
            root.join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\n# Shell Develop\n",
        )
        .unwrap();
        fs::write(
            root.join(".decree/routines/develop.ipynb"),
            r#"{"cells":[],"metadata":{},"nbformat":4,"nbformat_minor":5}"#,
        )
        .unwrap();

        // Without notebook support: .sh is used, .ipynb ignored
        let resolved =
            decree::routine::resolve_routine(&root, "develop", false).unwrap();
        assert_eq!(resolved.format, decree::routine::RoutineFormat::Shell);
    }

    #[test]
    fn test_resolve_not_found_only_ipynb_with_notebooks_disabled() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        fs::write(
            root.join(".decree/routines/develop.ipynb"),
            r#"{"cells":[],"metadata":{},"nbformat":4,"nbformat_minor":5}"#,
        )
        .unwrap();

        let result =
            decree::routine::resolve_routine(&root, "develop", false);
        assert!(result.is_err());
        match result.unwrap_err() {
            decree::error::DecreeError::RoutineNotFound(name) => {
                assert_eq!(name, "develop");
            }
            other => panic!("expected RoutineNotFound, got: {:?}", other),
        }
    }

    #[test]
    fn test_resolve_not_found_neither_exists() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let result =
            decree::routine::resolve_routine(&root, "develop", true);
        assert!(result.is_err());
        match result.unwrap_err() {
            decree::error::DecreeError::RoutineNotFound(name) => {
                assert_eq!(name, "develop");
            }
            other => panic!("expected RoutineNotFound, got: {:?}", other),
        }
    }

    #[test]
    fn test_resolve_explicit_sh_extension() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        fs::write(
            root.join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\n# Develop\n",
        )
        .unwrap();
        fs::write(
            root.join(".decree/routines/develop.ipynb"),
            r#"{"cells":[],"metadata":{},"nbformat":4,"nbformat_minor":5}"#,
        )
        .unwrap();

        // Explicit .sh extension skips precedence
        let resolved =
            decree::routine::resolve_routine(&root, "develop.sh", true).unwrap();
        assert_eq!(resolved.name, "develop");
        assert_eq!(resolved.format, decree::routine::RoutineFormat::Shell);
    }

    #[test]
    fn test_resolve_explicit_ipynb_extension() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        fs::write(
            root.join(".decree/routines/develop.ipynb"),
            r#"{"cells":[],"metadata":{},"nbformat":4,"nbformat_minor":5}"#,
        )
        .unwrap();

        let resolved =
            decree::routine::resolve_routine(&root, "develop.ipynb", true).unwrap();
        assert_eq!(resolved.name, "develop");
        assert_eq!(resolved.format, decree::routine::RoutineFormat::Notebook);
    }

    #[test]
    fn test_resolve_explicit_ipynb_extension_rejected_when_disabled() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        fs::write(
            root.join(".decree/routines/develop.ipynb"),
            r#"{"cells":[],"metadata":{},"nbformat":4,"nbformat_minor":5}"#,
        )
        .unwrap();

        let result =
            decree::routine::resolve_routine(&root, "develop.ipynb", false);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_sh_only_when_no_ipynb() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        fs::write(
            root.join(".decree/routines/develop.sh"),
            "#!/usr/bin/env bash\n# Develop\n",
        )
        .unwrap();

        // With notebook support but only .sh exists
        let resolved =
            decree::routine::resolve_routine(&root, "develop", true).unwrap();
        assert_eq!(resolved.format, decree::routine::RoutineFormat::Shell);
    }
}

mod custom_params_tests {
    use super::*;

    #[test]
    fn test_discover_custom_params_sh_basic() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        let script = r#"#!/usr/bin/env bash
# PR Review routine
set -euo pipefail

spec_file="${spec_file:-}"
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
target_branch="${target_branch:-main}"
"#;
        fs::write(root.join(".decree/routines/pr-review.sh"), script).unwrap();

        let resolved =
            decree::routine::resolve_routine(&root, "pr-review", false).unwrap();
        let params = decree::routine::discover_custom_params(&resolved).unwrap();
        assert_eq!(params, vec!["target_branch"]);
    }

    #[test]
    fn test_discover_custom_params_sh_multiple() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        let script = r#"#!/usr/bin/env bash
# Multi-param routine
set -euo pipefail

spec_file="${spec_file:-}"
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
target_branch="${target_branch:-main}"
deploy_env="${deploy_env:-staging}"
"#;
        fs::write(root.join(".decree/routines/multi.sh"), script).unwrap();

        let resolved =
            decree::routine::resolve_routine(&root, "multi", false).unwrap();
        let params = decree::routine::discover_custom_params(&resolved).unwrap();
        assert_eq!(params.len(), 2);
        assert!(params.contains(&"target_branch".to_string()));
        assert!(params.contains(&"deploy_env".to_string()));
    }

    #[test]
    fn test_discover_custom_params_sh_no_custom() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        let script = r#"#!/usr/bin/env bash
# Standard routine
set -euo pipefail

spec_file="${spec_file:-}"
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
"#;
        fs::write(root.join(".decree/routines/standard.sh"), script).unwrap();

        let resolved =
            decree::routine::resolve_routine(&root, "standard", false).unwrap();
        let params = decree::routine::discover_custom_params(&resolved).unwrap();
        assert!(params.is_empty());
    }

    #[test]
    fn test_discover_custom_params_sh_stops_at_non_assignment() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        let script = r#"#!/usr/bin/env bash
# Test routine
set -euo pipefail

spec_file="${spec_file:-}"
target_branch="${target_branch:-main}"

echo "hello"
another_var="${another_var:-}"
"#;
        fs::write(root.join(".decree/routines/test.sh"), script).unwrap();

        let resolved =
            decree::routine::resolve_routine(&root, "test", false).unwrap();
        let params = decree::routine::discover_custom_params(&resolved).unwrap();
        // Should only find target_branch — parsing stops at `echo`
        assert_eq!(params, vec!["target_branch"]);
    }

    #[test]
    fn test_discover_custom_params_ipynb() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        let notebook = r##"{
            "cells": [
                {
                    "cell_type": "markdown",
                    "source": ["# Review\n"],
                    "metadata": {}
                },
                {
                    "cell_type": "code",
                    "source": [
                        "spec_file = \"\"\n",
                        "message_file = \"\"\n",
                        "message_id = \"\"\n",
                        "message_dir = \"\"\n",
                        "chain = \"\"\n",
                        "seq = \"\"\n",
                        "target_branch = \"\"       # Custom: branch to target for PRs\n"
                    ],
                    "metadata": {"tags": ["parameters"]}
                }
            ],
            "metadata": {},
            "nbformat": 4,
            "nbformat_minor": 5
        }"##;
        fs::write(root.join(".decree/routines/review.ipynb"), notebook).unwrap();

        let resolved =
            decree::routine::resolve_routine(&root, "review", true).unwrap();
        let params = decree::routine::discover_custom_params(&resolved).unwrap();
        assert_eq!(params, vec!["target_branch"]);
    }

    #[test]
    fn test_discover_custom_params_ipynb_no_parameters_cell() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        let notebook = r##"{
            "cells": [
                {
                    "cell_type": "markdown",
                    "source": ["# No params\n"],
                    "metadata": {}
                },
                {
                    "cell_type": "code",
                    "source": ["print('hello')"],
                    "metadata": {}
                }
            ],
            "metadata": {},
            "nbformat": 4,
            "nbformat_minor": 5
        }"##;
        fs::write(root.join(".decree/routines/noparams.ipynb"), notebook).unwrap();

        let resolved =
            decree::routine::resolve_routine(&root, "noparams", true).unwrap();
        let params = decree::routine::discover_custom_params(&resolved).unwrap();
        assert!(params.is_empty());
    }

    #[test]
    fn test_build_custom_params_from_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        let script = r#"#!/usr/bin/env bash
# PR Review
set -euo pipefail

spec_file="${spec_file:-}"
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
target_branch="${target_branch:-main}"
"#;
        fs::write(root.join(".decree/routines/pr-review.sh"), script).unwrap();

        let resolved =
            decree::routine::resolve_routine(&root, "pr-review", false).unwrap();

        let mut custom_fields = std::collections::BTreeMap::new();
        custom_fields.insert(
            "target_branch".to_string(),
            serde_yaml::Value::String("develop".to_string()),
        );
        custom_fields.insert(
            "unknown_field".to_string(),
            serde_yaml::Value::String("ignored".to_string()),
        );

        let msg = decree::message::InboxMessage {
            id: "2025022514320000-0".to_string(),
            chain: "2025022514320000".to_string(),
            seq: 0,
            msg_type: decree::message::MessageType::Task,
            input_file: None,
            routine: "pr-review".to_string(),
            body: "Review changes".to_string(),
            custom_fields,
        };

        let params = decree::routine::build_custom_params(&resolved, &msg).unwrap();
        // target_branch should be populated from frontmatter
        assert_eq!(params.get("target_branch").unwrap(), "develop");
        // unknown_field should NOT be in params (not declared in routine)
        assert!(!params.contains_key("unknown_field"));
    }

    #[test]
    fn test_build_custom_params_missing_uses_default() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);
        let script = r#"#!/usr/bin/env bash
# PR Review
set -euo pipefail

spec_file="${spec_file:-}"
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
target_branch="${target_branch:-main}"
"#;
        fs::write(root.join(".decree/routines/pr-review.sh"), script).unwrap();

        let resolved =
            decree::routine::resolve_routine(&root, "pr-review", false).unwrap();

        let msg = decree::message::InboxMessage {
            id: "2025022514320000-0".to_string(),
            chain: "2025022514320000".to_string(),
            seq: 0,
            msg_type: decree::message::MessageType::Task,
            input_file: None,
            routine: "pr-review".to_string(),
            body: "Review changes".to_string(),
            custom_fields: std::collections::BTreeMap::new(),
        };

        let params = decree::routine::build_custom_params(&resolved, &msg).unwrap();
        // No frontmatter field for target_branch — params map is empty,
        // the routine will use its own default
        assert!(!params.contains_key("target_branch"));
    }

    #[test]
    fn test_build_standard_params() {
        let msg = decree::message::InboxMessage {
            id: "2025022514320000-0".to_string(),
            chain: "2025022514320000".to_string(),
            seq: 0,
            msg_type: decree::message::MessageType::Spec,
            input_file: Some("specs/01-auth.spec.md".to_string()),
            routine: "develop".to_string(),
            body: "Implement auth".to_string(),
            custom_fields: std::collections::BTreeMap::new(),
        };

        let msg_dir = std::path::Path::new("/tmp/runs/2025022514320000-0");
        let params = decree::routine::build_standard_params(&msg, msg_dir);

        assert_eq!(params.get("spec_file").unwrap(), "specs/01-auth.spec.md");
        assert_eq!(
            params.get("message_file").unwrap(),
            "/tmp/runs/2025022514320000-0/message.md"
        );
        assert_eq!(params.get("message_id").unwrap(), "2025022514320000-0");
        assert_eq!(
            params.get("message_dir").unwrap(),
            "/tmp/runs/2025022514320000-0"
        );
        assert_eq!(params.get("chain").unwrap(), "2025022514320000");
        assert_eq!(params.get("seq").unwrap(), "0");
    }
}

mod routine_execution_tests {
    use super::*;

    #[test]
    fn test_execute_shell_routine_success() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let script = r#"#!/usr/bin/env bash
# Echo routine
set -euo pipefail

spec_file="${spec_file:-}"
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

echo "routine executed: $message_id"
echo "chain=$chain seq=$seq"
"#;
        fs::write(root.join(".decree/routines/echo-test.sh"), script).unwrap();

        let msg_dir = root.join(".decree/runs/2025022514320000-0");
        fs::create_dir_all(&msg_dir).unwrap();
        fs::write(msg_dir.join("message.md"), "test body").unwrap();

        let msg = decree::message::InboxMessage {
            id: "2025022514320000-0".to_string(),
            chain: "2025022514320000".to_string(),
            seq: 0,
            msg_type: decree::message::MessageType::Task,
            input_file: None,
            routine: "echo-test".to_string(),
            body: "test body".to_string(),
            custom_fields: std::collections::BTreeMap::new(),
        };

        let resolved =
            decree::routine::resolve_routine(&root, "echo-test", false).unwrap();
        let result =
            decree::routine::execute_routine(&root, &resolved, &msg, &msg_dir).unwrap();

        assert!(result.success);
        assert_eq!(result.exit_code, Some(0));
        assert!(result.log_path.ends_with("routine.log"));

        // Verify log contents
        let log = fs::read_to_string(&result.log_path).unwrap();
        assert!(log.contains("routine executed: 2025022514320000-0"));
        assert!(log.contains("chain=2025022514320000 seq=0"));
    }

    #[test]
    fn test_execute_shell_routine_with_custom_params() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let script = r#"#!/usr/bin/env bash
# Custom param routine
set -euo pipefail

spec_file="${spec_file:-}"
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"
target_branch="${target_branch:-fallback}"

echo "branch=$target_branch"
"#;
        fs::write(root.join(".decree/routines/custom.sh"), script).unwrap();

        let msg_dir = root.join(".decree/runs/2025022514320000-0");
        fs::create_dir_all(&msg_dir).unwrap();
        fs::write(msg_dir.join("message.md"), "test").unwrap();

        let mut custom_fields = std::collections::BTreeMap::new();
        custom_fields.insert(
            "target_branch".to_string(),
            serde_yaml::Value::String("develop".to_string()),
        );

        let msg = decree::message::InboxMessage {
            id: "2025022514320000-0".to_string(),
            chain: "2025022514320000".to_string(),
            seq: 0,
            msg_type: decree::message::MessageType::Task,
            input_file: None,
            routine: "custom".to_string(),
            body: "test".to_string(),
            custom_fields,
        };

        let resolved =
            decree::routine::resolve_routine(&root, "custom", false).unwrap();
        let result =
            decree::routine::execute_routine(&root, &resolved, &msg, &msg_dir).unwrap();

        assert!(result.success);
        let log = fs::read_to_string(&result.log_path).unwrap();
        assert!(log.contains("branch=develop"));
    }

    #[test]
    fn test_execute_shell_routine_no_output_ipynb() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let script = "#!/usr/bin/env bash\necho ok\n";
        fs::write(root.join(".decree/routines/simple.sh"), script).unwrap();

        let msg_dir = root.join(".decree/runs/2025022514320000-0");
        fs::create_dir_all(&msg_dir).unwrap();
        fs::write(msg_dir.join("message.md"), "test").unwrap();

        let msg = decree::message::InboxMessage {
            id: "2025022514320000-0".to_string(),
            chain: "2025022514320000".to_string(),
            seq: 0,
            msg_type: decree::message::MessageType::Task,
            input_file: None,
            routine: "simple".to_string(),
            body: "test".to_string(),
            custom_fields: std::collections::BTreeMap::new(),
        };

        let resolved =
            decree::routine::resolve_routine(&root, "simple", false).unwrap();
        decree::routine::execute_routine(&root, &resolved, &msg, &msg_dir).unwrap();

        // Shell routines should NOT create output.ipynb or papermill.log
        assert!(!msg_dir.join("output.ipynb").exists());
        assert!(!msg_dir.join("papermill.log").exists());
        // Should create routine.log
        assert!(msg_dir.join("routine.log").exists());
    }

    #[test]
    fn test_execute_shell_routine_failure() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let script = "#!/usr/bin/env bash\nexit 1\n";
        fs::write(root.join(".decree/routines/fail.sh"), script).unwrap();

        let msg_dir = root.join(".decree/runs/2025022514320000-0");
        fs::create_dir_all(&msg_dir).unwrap();
        fs::write(msg_dir.join("message.md"), "test").unwrap();

        let msg = decree::message::InboxMessage {
            id: "2025022514320000-0".to_string(),
            chain: "2025022514320000".to_string(),
            seq: 0,
            msg_type: decree::message::MessageType::Task,
            input_file: None,
            routine: "fail".to_string(),
            body: "test".to_string(),
            custom_fields: std::collections::BTreeMap::new(),
        };

        let resolved =
            decree::routine::resolve_routine(&root, "fail", false).unwrap();
        let result =
            decree::routine::execute_routine(&root, &resolved, &msg, &msg_dir).unwrap();

        assert!(!result.success);
        assert_eq!(result.exit_code, Some(1));
    }

    #[test]
    fn test_execute_shell_routine_captures_stderr() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let script = "#!/usr/bin/env bash\necho 'stdout line'\necho 'stderr line' >&2\n";
        fs::write(root.join(".decree/routines/both.sh"), script).unwrap();

        let msg_dir = root.join(".decree/runs/2025022514320000-0");
        fs::create_dir_all(&msg_dir).unwrap();
        fs::write(msg_dir.join("message.md"), "test").unwrap();

        let msg = decree::message::InboxMessage {
            id: "2025022514320000-0".to_string(),
            chain: "2025022514320000".to_string(),
            seq: 0,
            msg_type: decree::message::MessageType::Task,
            input_file: None,
            routine: "both".to_string(),
            body: "test".to_string(),
            custom_fields: std::collections::BTreeMap::new(),
        };

        let resolved =
            decree::routine::resolve_routine(&root, "both", false).unwrap();
        decree::routine::execute_routine(&root, &resolved, &msg, &msg_dir).unwrap();

        let log = fs::read_to_string(msg_dir.join("routine.log")).unwrap();
        // Both stdout and stderr should appear in the log
        assert!(log.contains("stdout line"));
        assert!(log.contains("stderr line"));
    }

    #[test]
    fn test_execute_shell_routine_unknown_frontmatter_ignored() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let script = r#"#!/usr/bin/env bash
# Simple routine
set -euo pipefail

spec_file="${spec_file:-}"
message_file="${message_file:-}"
message_id="${message_id:-}"
message_dir="${message_dir:-}"
chain="${chain:-}"
seq="${seq:-}"

echo "ok"
"#;
        fs::write(root.join(".decree/routines/simple2.sh"), script).unwrap();

        let msg_dir = root.join(".decree/runs/2025022514320000-0");
        fs::create_dir_all(&msg_dir).unwrap();
        fs::write(msg_dir.join("message.md"), "test").unwrap();

        let mut custom_fields = std::collections::BTreeMap::new();
        custom_fields.insert(
            "unknown_field".to_string(),
            serde_yaml::Value::String("value".to_string()),
        );

        let msg = decree::message::InboxMessage {
            id: "2025022514320000-0".to_string(),
            chain: "2025022514320000".to_string(),
            seq: 0,
            msg_type: decree::message::MessageType::Task,
            input_file: None,
            routine: "simple2".to_string(),
            body: "test".to_string(),
            custom_fields,
        };

        let resolved =
            decree::routine::resolve_routine(&root, "simple2", false).unwrap();
        let result =
            decree::routine::execute_routine(&root, &resolved, &msg, &msg_dir).unwrap();

        // Should succeed — unknown fields are ignored
        assert!(result.success);
    }
}

// ---------------------------------------------------------------------------
// Spec 07: Pipeline, Run, and Process tests
// ---------------------------------------------------------------------------

mod pipeline_tests {
    use super::*;

    /// Helper: set up a project with config and a simple succeed/fail routine.
    fn setup_with_config(tmp: &TempDir) -> std::path::PathBuf {
        let root = setup_project(tmp);
        let config = decree::config::Config::default();
        config.save(&root).unwrap();

        // A routine that always succeeds
        let succeed_sh = "#!/usr/bin/env bash\necho 'routine ok'\n";
        fs::write(root.join(".decree/routines/develop.sh"), succeed_sh).unwrap();

        root
    }

    /// Helper: set up with a routine that always fails.
    fn setup_with_failing_routine(tmp: &TempDir) -> std::path::PathBuf {
        let root = setup_project(tmp);
        let mut config = decree::config::Config::default();
        config.max_retries = 2; // fewer retries for faster tests
        config.save(&root).unwrap();

        let fail_sh = "#!/usr/bin/env bash\necho 'routine failed' >&2\nexit 1\n";
        fs::write(root.join(".decree/routines/develop.sh"), fail_sh).unwrap();

        root
    }

    #[test]
    fn test_create_inbox_message_task_type() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_config(&tmp);

        let vars = vec![("routine".to_string(), "develop".to_string())];
        let path = decree::pipeline::create_inbox_message(
            &root,
            "fix-types",
            "20260226143200",
            "Fix the type errors",
            &vars,
        )
        .unwrap();

        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("type: task"));
        assert!(content.contains("chain: \"20260226143200\""));
        assert!(content.contains("seq: 0"));
        assert!(content.contains("routine: develop"));
        assert!(content.contains("Fix the type errors"));
    }

    #[test]
    fn test_create_inbox_message_spec_type() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_config(&tmp);

        let vars = vec![
            ("input_file".to_string(), "specs/01-add-auth.spec.md".to_string()),
        ];
        let path = decree::pipeline::create_inbox_message(
            &root,
            "add-auth",
            "20260226143200",
            "",
            &vars,
        )
        .unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("type: spec"));
        assert!(content.contains("input_file: specs/01-add-auth.spec.md"));
    }

    #[test]
    fn test_create_inbox_message_with_multiple_vars() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_config(&tmp);

        let vars = vec![
            ("routine".to_string(), "pr-review".to_string()),
            ("target_branch".to_string(), "main".to_string()),
            ("reviewer".to_string(), "alice".to_string()),
        ];
        let path = decree::pipeline::create_inbox_message(
            &root,
            "pr-review",
            "20260226143200",
            "Review the current branch",
            &vars,
        )
        .unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("routine: pr-review"));
        assert!(content.contains("target_branch: main"));
        assert!(content.contains("reviewer: alice"));
        assert!(content.contains("Review the current branch"));
    }

    #[test]
    fn test_process_chain_success() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_config(&tmp);
        let config = decree::config::Config::load(&root).unwrap();

        // Create an inbox message
        let vars = vec![("routine".to_string(), "develop".to_string())];
        let msg_path = decree::pipeline::create_inbox_message(
            &root,
            "test-run",
            "20260226143200",
            "test body",
            &vars,
        )
        .unwrap();

        let result = decree::pipeline::process_chain(&root, &config, &msg_path, None).unwrap();

        // Should succeed
        assert!(matches!(result, decree::pipeline::ProcessResult::Success));

        // Message should be in done/
        assert!(!msg_path.exists());
        assert!(root.join(".decree/inbox/done/test-run.md").exists());

        // Run directory should exist with artifacts
        let runs = fs::read_dir(root.join(".decree/runs")).unwrap();
        let run_dirs: Vec<_> = runs
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().unwrap().is_dir())
            .collect();
        assert_eq!(run_dirs.len(), 1);

        let run_dir = run_dirs[0].path();
        assert!(run_dir.join("message.md").exists());
        assert!(run_dir.join("manifest.json").exists());
        assert!(run_dir.join("routine.log").exists());
    }

    #[test]
    fn test_process_chain_failure_dead_letters() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_failing_routine(&tmp);
        let config = decree::config::Config::load(&root).unwrap();

        let vars = vec![("routine".to_string(), "develop".to_string())];
        let msg_path = decree::pipeline::create_inbox_message(
            &root,
            "fail-run",
            "20260226143200",
            "this will fail",
            &vars,
        )
        .unwrap();

        let result = decree::pipeline::process_chain(&root, &config, &msg_path, None).unwrap();

        // Should be dead-lettered
        assert!(matches!(
            result,
            decree::pipeline::ProcessResult::DeadLettered(_)
        ));

        // Message should be in dead/
        assert!(!msg_path.exists());
        assert!(root.join(".decree/inbox/dead/fail-run.md").exists());
    }

    #[test]
    fn test_process_chain_failure_context_written_on_final_retry() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_failing_routine(&tmp);
        let config = decree::config::Config::load(&root).unwrap();

        let vars = vec![("routine".to_string(), "develop".to_string())];
        let msg_path = decree::pipeline::create_inbox_message(
            &root,
            "ctx-run",
            "20260226143200",
            "failure context test",
            &vars,
        )
        .unwrap();

        decree::pipeline::process_chain(&root, &config, &msg_path, None).unwrap();

        // Find the run directory
        let runs: Vec<_> = fs::read_dir(root.join(".decree/runs"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().unwrap().is_dir())
            .collect();
        assert_eq!(runs.len(), 1);

        let run_dir = runs[0].path();
        // failure-context.md should exist (written before final attempt)
        assert!(
            run_dir.join("failure-context.md").exists(),
            "failure-context.md should be written for final retry"
        );

        let ctx = fs::read_to_string(run_dir.join("failure-context.md")).unwrap();
        assert!(ctx.contains("# Failure Context"));
        assert!(ctx.contains("Attempt 1"));
    }

    #[test]
    fn test_depth_limit_dead_letters() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_config(&tmp);
        let mut config = decree::config::Config::load(&root).unwrap();
        config.max_depth = 2;
        config.save(&root).unwrap();
        let config = decree::config::Config::load(&root).unwrap();

        // Create a message with seq >= max_depth
        let inbox = root.join(".decree/inbox");
        let msg_content = "---\nchain: \"20260226143200\"\nseq: 5\ntype: task\nroutine: develop\n---\ndeep message\n";
        let msg_path = inbox.join("deep-msg.md");
        fs::write(&msg_path, msg_content).unwrap();

        let result = decree::pipeline::process_chain(&root, &config, &msg_path, None).unwrap();

        assert!(matches!(
            result,
            decree::pipeline::ProcessResult::DeadLettered(_)
        ));
        assert!(root.join(".decree/inbox/dead/deep-msg.md").exists());
    }

    #[test]
    fn test_depth_first_follow_up_processing() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_config(&tmp);

        let chain = "20260226999900";

        // Create a routine that spawns a follow-up message
        let spawn_sh = format!(
            "#!/usr/bin/env bash\n\
             echo 'spawning follow-up'\n\
             # Create a follow-up message in the inbox\n\
             cat > .decree/inbox/followup.md << 'ENDMSG'\n\
             ---\n\
             chain: \"{chain}\"\n\
             seq: 1\n\
             type: task\n\
             routine: develop\n\
             ---\n\
             follow-up work\n\
             ENDMSG\n"
        );
        fs::write(root.join(".decree/routines/spawn.sh"), spawn_sh).unwrap();

        let config = decree::config::Config::load(&root).unwrap();

        let vars = vec![("routine".to_string(), "spawn".to_string())];
        let msg_path = decree::pipeline::create_inbox_message(
            &root,
            "spawn-test",
            chain,
            "initial message",
            &vars,
        )
        .unwrap();

        let result = decree::pipeline::process_chain(&root, &config, &msg_path, None).unwrap();

        assert!(matches!(result, decree::pipeline::ProcessResult::Success));

        // Both messages should be processed (in done/)
        assert!(root.join(".decree/inbox/done/spawn-test.md").exists());
        assert!(root.join(".decree/inbox/done/followup.md").exists());

        // Both run directories should exist
        let run_dirs: Vec<String> = fs::read_dir(root.join(".decree/runs"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().unwrap().is_dir())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(run_dirs.len(), 2, "should have 2 run directories");
    }

    #[test]
    fn test_to_kebab_case() {
        assert_eq!(decree::pipeline::to_kebab_case("Fix Auth Types"), "fix-auth-types");
        assert_eq!(decree::pipeline::to_kebab_case("fix_auth--types!!"), "fix-auth-types");
        assert_eq!(decree::pipeline::to_kebab_case("fix-auth-types"), "fix-auth-types");
        assert_eq!(decree::pipeline::to_kebab_case("  --fix--  "), "fix");
        assert_eq!(decree::pipeline::to_kebab_case("CamelCase"), "camelcase");
        assert_eq!(decree::pipeline::to_kebab_case(""), "");
    }

    #[test]
    fn test_last_run_persistence() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_config(&tmp);

        let lr = decree::pipeline::LastRun {
            routine: "develop".into(),
            message_name: "fix-auth".into(),
            input_file: Some("specs/01.spec.md".into()),
            custom: [("target_branch".into(), "main".into())].into(),
        };
        lr.save(&root).unwrap();

        let loaded = decree::pipeline::LastRun::load(&root).unwrap();
        assert_eq!(loaded.routine, "develop");
        assert_eq!(loaded.message_name, "fix-auth");
        assert_eq!(loaded.input_file, Some("specs/01.spec.md".into()));
        assert_eq!(loaded.custom.get("target_branch").unwrap(), "main");
    }

    #[test]
    fn test_last_run_missing_returns_none() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_config(&tmp);
        assert!(decree::pipeline::LastRun::load(&root).is_none());
    }
}

mod process_command_tests {
    use super::*;

    fn setup_with_config(tmp: &TempDir) -> std::path::PathBuf {
        let root = setup_project(tmp);
        let config = decree::config::Config::default();
        config.save(&root).unwrap();

        let succeed_sh = "#!/usr/bin/env bash\necho 'routine ok'\n";
        fs::write(root.join(".decree/routines/develop.sh"), succeed_sh).unwrap();

        root
    }

    #[test]
    fn test_process_marks_spec_as_processed() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_config(&tmp);

        // Create a spec file
        fs::write(
            root.join("specs/01-add-auth.spec.md"),
            "---\nroutine: develop\n---\n# Add Authentication\n",
        )
        .unwrap();

        let config = decree::config::Config::load(&root).unwrap();
        let all_specs = decree::spec::list_specs(&root).unwrap();
        assert_eq!(all_specs.len(), 1);

        // Process the spec through the pipeline
        let chain = decree::message::MessageId::new_chain(0);
        let vars = vec![
            ("input_file".to_string(), "specs/01-add-auth.spec.md".to_string()),
        ];
        let msg_path = decree::pipeline::create_inbox_message(
            &root,
            "01-add-auth",
            &chain,
            "",
            &vars,
        )
        .unwrap();

        let result = decree::pipeline::process_chain(
            &root,
            &config,
            &msg_path,
            Some("develop"),
        )
        .unwrap();

        assert!(matches!(result, decree::pipeline::ProcessResult::Success));

        // Spec should now be marked as processed
        let processed = decree::spec::read_processed(&root).unwrap();
        assert!(processed.contains("01-add-auth.spec.md"));
    }

    #[test]
    fn test_process_skips_already_processed_specs() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_config(&tmp);

        fs::write(
            root.join("specs/01-done.spec.md"),
            "# Already done\n",
        )
        .unwrap();
        fs::write(
            root.join("specs/02-pending.spec.md"),
            "# Pending\n",
        )
        .unwrap();

        // Mark first as processed
        decree::spec::mark_processed(&root, "01-done.spec.md").unwrap();

        // Check that next_unprocessed returns the second spec
        let next = decree::spec::next_unprocessed(&root).unwrap();
        assert_eq!(next, Some("02-pending.spec.md".to_string()));
    }
}

mod run_command_tests {
    use super::*;
    use assert_cmd::Command;
    use predicates::prelude::*;

    fn setup_with_config(tmp: &TempDir) -> std::path::PathBuf {
        let root = setup_project(tmp);
        let config = decree::config::Config::default();
        config.save(&root).unwrap();

        let succeed_sh = "#!/usr/bin/env bash\necho 'routine ok'\n";
        fs::write(root.join(".decree/routines/develop.sh"), succeed_sh).unwrap();

        root
    }

    #[test]
    fn test_run_with_prompt_and_name() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_config(&tmp);

        Command::cargo_bin("decree")
            .unwrap()
            .args([
                "run",
                "-m", "fix-types",
                "-p", "fix the auth types",
                "-v", "routine=develop",
            ])
            .current_dir(&root)
            .assert()
            .success()
            .stdout(predicate::str::contains("creating message: fix-types"))
            .stdout(predicate::str::contains("done"));

        // Message should be in done/
        assert!(root.join(".decree/inbox/done/fix-types.md").exists());

        // Run directory should exist
        let runs: Vec<_> = fs::read_dir(root.join(".decree/runs"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().unwrap().is_dir())
            .collect();
        assert_eq!(runs.len(), 1);
    }

    #[test]
    fn test_run_with_spec_file() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_config(&tmp);

        fs::write(
            root.join("specs/01-add-auth.spec.md"),
            "---\nroutine: develop\n---\n# Add Auth\n",
        )
        .unwrap();

        Command::cargo_bin("decree")
            .unwrap()
            .args([
                "run",
                "-v", "input_file=specs/01-add-auth.spec.md",
            ])
            .current_dir(&root)
            .assert()
            .success()
            .stdout(predicate::str::contains("done"));

        // Spec should be marked as processed
        let processed = decree::spec::read_processed(&root).unwrap();
        assert!(processed.contains("01-add-auth.spec.md"));
    }

    #[test]
    fn test_run_with_piped_input() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_config(&tmp);

        Command::cargo_bin("decree")
            .unwrap()
            .args(["run", "-m", "piped-task", "-v", "routine=develop"])
            .write_stdin("this is piped input\n")
            .current_dir(&root)
            .assert()
            .success()
            .stdout(predicate::str::contains("done"));

        assert!(root.join(".decree/inbox/done/piped-task.md").exists());
    }

    #[test]
    fn test_run_with_multiple_vars() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_config(&tmp);

        Command::cargo_bin("decree")
            .unwrap()
            .args([
                "run",
                "-m", "multi-var",
                "-p", "test",
                "-v", "routine=develop",
                "-v", "target_branch=main",
                "-v", "reviewer=alice",
            ])
            .current_dir(&root)
            .assert()
            .success();

        // Check the message in done/ has all vars
        let done_content =
            fs::read_to_string(root.join(".decree/inbox/done/multi-var.md")).unwrap();
        assert!(done_content.contains("target_branch: main"));
        assert!(done_content.contains("reviewer: alice"));
    }

    #[test]
    fn test_run_routine_not_found_dead_letters() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_config(&tmp);

        Command::cargo_bin("decree")
            .unwrap()
            .args([
                "run",
                "-m", "bad-routine",
                "-p", "test",
                "-v", "routine=nonexistent",
            ])
            .current_dir(&root)
            .assert()
            .success() // command itself succeeds, but message is dead-lettered
            .stderr(predicate::str::contains("dead-letter"));

        assert!(root.join(".decree/inbox/dead/bad-routine.md").exists());
    }

    #[test]
    fn test_run_no_body_no_input_file() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_config(&tmp);

        // Running with only -m and -v routine= but no -p and no input_file
        // should still work (empty body is valid for task messages)
        Command::cargo_bin("decree")
            .unwrap()
            .args([
                "run",
                "-m", "no-body",
                "-v", "routine=develop",
            ])
            .current_dir(&root)
            .assert()
            .success();

        assert!(root.join(".decree/inbox/done/no-body.md").exists());
    }

    #[test]
    fn test_run_creates_run_directory_with_artifacts() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_config(&tmp);

        Command::cargo_bin("decree")
            .unwrap()
            .args([
                "run",
                "-m", "artifacts-test",
                "-p", "check artifacts",
                "-v", "routine=develop",
            ])
            .current_dir(&root)
            .assert()
            .success();

        // Find the run directory
        let runs: Vec<_> = fs::read_dir(root.join(".decree/runs"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().unwrap().is_dir())
            .collect();
        assert_eq!(runs.len(), 1);

        let run_dir = runs[0].path();
        assert!(run_dir.join("message.md").exists(), "message.md missing");
        assert!(run_dir.join("manifest.json").exists(), "manifest.json missing");
        assert!(run_dir.join("routine.log").exists(), "routine.log missing");
        assert!(run_dir.join("changes.diff").exists(), "changes.diff missing");
    }

    #[test]
    fn test_run_message_name_kebab_cased() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_config(&tmp);

        Command::cargo_bin("decree")
            .unwrap()
            .args([
                "run",
                "-m", "Fix Auth Types",
                "-p", "test",
                "-v", "routine=develop",
            ])
            .current_dir(&root)
            .assert()
            .success();

        // Name should be kebab-cased
        assert!(root.join(".decree/inbox/done/fix-auth-types.md").exists());
    }

    #[test]
    fn test_process_no_specs() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_config(&tmp);

        Command::cargo_bin("decree")
            .unwrap()
            .arg("process")
            .current_dir(&root)
            .assert()
            .success()
            .stdout(predicate::str::contains("no spec files"));
    }

    #[test]
    fn test_process_all_already_processed() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_config(&tmp);

        fs::write(root.join("specs/01-done.spec.md"), "# Done\n").unwrap();
        decree::spec::mark_processed(&root, "01-done.spec.md").unwrap();

        Command::cargo_bin("decree")
            .unwrap()
            .arg("process")
            .current_dir(&root)
            .assert()
            .success()
            .stdout(predicate::str::contains("all specs are already processed"));
    }

    #[test]
    fn test_process_handles_spec() {
        let tmp = TempDir::new().unwrap();
        let root = setup_with_config(&tmp);

        fs::write(
            root.join("specs/01-feature.spec.md"),
            "---\nroutine: develop\n---\n# Feature\nImplement the feature.\n",
        )
        .unwrap();

        Command::cargo_bin("decree")
            .unwrap()
            .arg("process")
            .current_dir(&root)
            .assert()
            .success()
            .stdout(predicate::str::contains("processing spec: 01-feature.spec.md"))
            .stdout(predicate::str::contains("done"));

        // Spec should be marked processed
        let processed = decree::spec::read_processed(&root).unwrap();
        assert!(processed.contains("01-feature.spec.md"));
    }
}

mod diff_apply_tests {
    use super::*;

    /// Helper: create a message directory with a changes.diff file.
    fn create_msg_with_diff(root: &std::path::Path, msg_id: &str, diff: &str) {
        let msg_dir = root.join(format!(".decree/runs/{msg_id}"));
        fs::create_dir_all(&msg_dir).unwrap();
        fs::write(msg_dir.join("changes.diff"), diff).unwrap();
    }

    /// Helper: create a message directory with a changes.diff and message.md.
    fn create_msg_full(
        root: &std::path::Path,
        msg_id: &str,
        diff: &str,
        message_md: &str,
    ) {
        let msg_dir = root.join(format!(".decree/runs/{msg_id}"));
        fs::create_dir_all(&msg_dir).unwrap();
        fs::write(msg_dir.join("changes.diff"), diff).unwrap();
        fs::write(msg_dir.join("message.md"), message_md).unwrap();
    }

    #[test]
    fn test_diff_latest_message() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let diff = "--- /dev/null\n+++ b/hello.txt\n@@ -0,0 +1 @@\n+hello\n";
        create_msg_with_diff(&root, "2026022600000000-0", diff);

        let result =
            decree::diff_apply::read_diff(&root.join(".decree/runs"), "2026022600000000-0")
                .unwrap();
        assert!(result.is_some());
        assert!(result.unwrap().contains("+hello"));
    }

    #[test]
    fn test_diff_no_changes() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let msg_dir = root.join(".decree/runs/2026022600000000-0");
        fs::create_dir_all(&msg_dir).unwrap();
        // No changes.diff

        let result =
            decree::diff_apply::read_diff(&root.join(".decree/runs"), "2026022600000000-0")
                .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_and_apply_new_file() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let diff = "--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+line1\n+line2\n";

        let file_diffs = decree::diff_apply::parse_diff(diff).unwrap();
        assert_eq!(file_diffs.len(), 1);
        assert_eq!(file_diffs[0].kind, decree::diff_apply::FileChangeKind::Add);

        // No conflicts expected (file doesn't exist)
        let conflicts = decree::diff_apply::check_conflicts(&root, &file_diffs);
        assert!(conflicts.is_empty());

        // Apply
        decree::diff_apply::apply_diffs(&root, &file_diffs).unwrap();
        let content = fs::read_to_string(root.join("new.txt")).unwrap();
        assert_eq!(content, "line1\nline2\n");
    }

    #[test]
    fn test_parse_and_apply_modification() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        fs::write(root.join("existing.txt"), "aaa\nbbb\nccc\n").unwrap();

        let diff = "\
--- a/existing.txt
+++ b/existing.txt
@@ -1,3 +1,3 @@
 aaa
-bbb
+BBB
 ccc
";
        let file_diffs = decree::diff_apply::parse_diff(diff).unwrap();
        let conflicts = decree::diff_apply::check_conflicts(&root, &file_diffs);
        assert!(conflicts.is_empty());

        decree::diff_apply::apply_diffs(&root, &file_diffs).unwrap();
        let content = fs::read_to_string(root.join("existing.txt")).unwrap();
        assert_eq!(content, "aaa\nBBB\nccc\n");
    }

    #[test]
    fn test_conflict_detection_preimage_mismatch() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        fs::write(root.join("file.txt"), "xxx\nyyy\nzzz\n").unwrap();

        // Diff expects "aaa" at line 1, but file has "xxx"
        let diff = "\
--- a/file.txt
+++ b/file.txt
@@ -1,3 +1,3 @@
-aaa
+AAA
 yyy
 zzz
";
        let file_diffs = decree::diff_apply::parse_diff(diff).unwrap();
        let conflicts = decree::diff_apply::check_conflicts(&root, &file_diffs);
        assert!(!conflicts.is_empty());
        assert!(conflicts[0].detail.contains("expected"));
    }

    #[test]
    fn test_conflict_new_file_already_exists() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        fs::write(root.join("exists.txt"), "content").unwrap();

        let diff = "--- /dev/null\n+++ b/exists.txt\n@@ -0,0 +1 @@\n+new\n";
        let file_diffs = decree::diff_apply::parse_diff(diff).unwrap();
        let conflicts = decree::diff_apply::check_conflicts(&root, &file_diffs);
        assert!(!conflicts.is_empty());
        assert!(conflicts[0].detail.contains("already exists"));
    }

    #[test]
    fn test_parse_and_apply_deletion() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        fs::write(root.join("delete_me.txt"), "old\n").unwrap();

        let diff = "\
--- a/delete_me.txt
+++ /dev/null
@@ -1 +0,0 @@
-old
";
        let file_diffs = decree::diff_apply::parse_diff(diff).unwrap();
        assert_eq!(
            file_diffs[0].kind,
            decree::diff_apply::FileChangeKind::Delete
        );

        decree::diff_apply::apply_diffs(&root, &file_diffs).unwrap();
        assert!(!root.join("delete_me.txt").exists());
    }

    #[test]
    fn test_resolve_chain_targets() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        create_msg_with_diff(&root, "2026022600000000-0", "+a");
        create_msg_with_diff(&root, "2026022600000000-1", "+b");
        create_msg_with_diff(&root, "2026022600000000-2", "+c");

        let runs_dir = root.join(".decree/runs");
        let targets =
            decree::diff_apply::resolve_targets(&runs_dir, "2026022600000000").unwrap();
        assert_eq!(targets.len(), 3);
        assert_eq!(targets[0], "2026022600000000-0");
        assert_eq!(targets[2], "2026022600000000-2");
    }

    #[test]
    fn test_messages_since() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        create_msg_with_diff(&root, "2026022600000000-0", "+a");
        create_msg_with_diff(&root, "2026022600000000-1", "+b");
        create_msg_with_diff(&root, "2026022600000100-0", "+c");

        let runs_dir = root.join(".decree/runs");
        let result =
            decree::diff_apply::messages_since(&runs_dir, "2026022600000000-1").unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "2026022600000000-1");
        assert_eq!(result[1], "2026022600000100-0");
    }

    #[test]
    fn test_messages_through() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        create_msg_with_diff(&root, "2026022600000000-0", "+a");
        create_msg_with_diff(&root, "2026022600000000-1", "+b");
        create_msg_with_diff(&root, "2026022600000100-0", "+c");

        let runs_dir = root.join(".decree/runs");
        let result =
            decree::diff_apply::messages_through(&runs_dir, "2026022600000000-1").unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "2026022600000000-0");
        assert_eq!(result[1], "2026022600000000-1");
    }

    #[test]
    fn test_list_messages_grouped() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let diff1 = "--- /dev/null\n+++ b/a.txt\n@@ -0,0 +1,3 @@\n+a\n+b\n+c\n";
        let diff2 = "--- a/a.txt\n+++ b/a.txt\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n";

        create_msg_full(
            &root,
            "2026022600000000-0",
            diff1,
            "---\nid: 2026022600000000-0\ninput_file: 01-feature.spec.md\n---\n",
        );
        create_msg_full(
            &root,
            "2026022600000000-1",
            diff2,
            "---\nid: 2026022600000000-1\ntask: fix types\n---\n",
        );

        let runs_dir = root.join(".decree/runs");
        let chains = decree::diff_apply::list_messages(&runs_dir).unwrap();
        assert_eq!(chains.len(), 1);

        let msgs = chains.get("2026022600000000").unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].stats.additions, 3);
        assert_eq!(msgs[0].stats.files, 1);
        assert!(msgs[0].description.contains("01-feature"));
        assert!(msgs[1].description.contains("task: fix types"));
    }

    #[test]
    fn test_diff_stats() {
        let diff = "\
--- /dev/null
+++ b/new.txt
@@ -0,0 +1,5 @@
+a
+b
+c
+d
+e
--- a/old.txt
+++ b/old.txt
@@ -1,3 +1,2 @@
 keep
-remove1
-remove2
+add1
";
        let stats = decree::diff_apply::compute_stats(diff);
        assert_eq!(stats.additions, 6);
        assert_eq!(stats.deletions, 2);
        assert_eq!(stats.files, 2);
    }

    #[test]
    fn test_all_messages() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        create_msg_with_diff(&root, "2026022600000000-0", "+a");
        create_msg_with_diff(&root, "2026022600000100-0", "+b");
        create_msg_with_diff(&root, "2026022600000200-0", "+c");

        let runs_dir = root.join(".decree/runs");
        let all = decree::diff_apply::all_messages(&runs_dir).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0], "2026022600000000-0");
        assert_eq!(all[2], "2026022600000200-0");
    }

    #[test]
    fn test_apply_multiple_hunks() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        fs::write(
            root.join("multi.txt"),
            "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10\n",
        )
        .unwrap();

        let diff = "\
--- a/multi.txt
+++ b/multi.txt
@@ -1,4 +1,4 @@
-line1
+LINE1
 line2
 line3
 line4
@@ -7,4 +7,4 @@
 line7
 line8
-line9
+LINE9
 line10
";
        let file_diffs = decree::diff_apply::parse_diff(diff).unwrap();
        let conflicts = decree::diff_apply::check_conflicts(&root, &file_diffs);
        assert!(conflicts.is_empty());

        decree::diff_apply::apply_diffs(&root, &file_diffs).unwrap();
        let content = fs::read_to_string(root.join("multi.txt")).unwrap();
        assert!(content.contains("LINE1"));
        assert!(content.contains("LINE9"));
        assert!(content.contains("line5"));
    }

    #[test]
    fn test_apply_creates_nested_dirs() {
        let tmp = TempDir::new().unwrap();
        let root = setup_project(&tmp);

        let diff = "--- /dev/null\n+++ b/deep/nested/dir/file.txt\n@@ -0,0 +1 @@\n+content\n";
        let file_diffs = decree::diff_apply::parse_diff(diff).unwrap();
        decree::diff_apply::apply_diffs(&root, &file_diffs).unwrap();

        let content = fs::read_to_string(root.join("deep/nested/dir/file.txt")).unwrap();
        assert_eq!(content, "content\n");
    }
}
