use crate::cli::AiBackend;
use crate::commands::{graph, skill};
use crate::config;
use crate::error::DecreeError;
use crate::machine::MACHINES_DIR;
use crate::runtime::SCRIPTS_DIR;
use std::path::Path;
use std::process::Command;

/// An AI backend: its CLI, and how the routine templates and its router call it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Backend {
    /// The command, which also names the router machine (`<name>_router`) and its
    /// script (`ask_<name>`).
    name: &'static str,
    /// The name the router machine's description and script use.
    title: &'static str,
    /// The non-interactive prompt command, prompt last: `{ai_invoke}` in routines.
    invoke: &'static str,
    /// The line of `ask_<name>` that sends `$prompt` and prints the reply.
    ask: &'static str,
}

/// AI backends in 0.4.2's detection order. The routers differ only in the CLI call.
const AI_BACKENDS: &[Backend] = &[
    Backend {
        name: "opencode",
        title: "OpenCode",
        invoke: "opencode run",
        ask: "opencode run \"$prompt\"",
    },
    Backend {
        name: "claude",
        title: "Claude",
        invoke: "claude -p",
        ask: "printf '%s' \"$prompt\" | claude -p",
    },
    Backend {
        name: "copilot",
        title: "Copilot",
        invoke: "copilot -p",
        ask: "copilot -p \"$prompt\"",
    },
];

/// The router machine `init` writes (spec section 7, The default router), and its script.
const ROUTER_YML: &str = include_str!("../templates/router/router.yml");
const ROUTER_ASK_SH: &str = include_str!("../templates/router/ask.sh");

/// Git stash hook routine: git-baseline.sh (beforeEach hook)
const GIT_BASELINE_SH: &str = include_str!("../templates/git-baseline.sh");

/// Git stash hook routine: git-stash-changes.sh (afterEach hook)
const GIT_STASH_CHANGES_SH: &str = include_str!("../templates/git-stash-changes.sh");

// Templates embedded from src/templates/ at compile time.
const DEVELOP_SH: &str = include_str!("../templates/develop.sh");
const RUST_DEVELOP_SH: &str = include_str!("../templates/rust-develop.sh");
const ROUTER_MD: &str = include_str!("../templates/router.md");
const DECREE_GITIGNORE: &str = include_str!("../templates/gitignore");

/// Check if a command exists on PATH.
fn command_exists(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check if we're inside a git repository.
fn is_git_repo() -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// The `AI_BACKENDS` entry for a backend.
fn backend_entry(ai: AiBackend) -> Backend {
    let name = match ai {
        AiBackend::Opencode => "opencode",
        AiBackend::Claude => "claude",
        AiBackend::Copilot => "copilot",
    };
    *AI_BACKENDS
        .iter()
        .find(|b| b.name == name)
        .expect("every AiBackend has an AI_BACKENDS entry")
}

/// The backend `--ai` names; without it, the first in detection order for which
/// `found` holds, else opencode.
fn select_backend(ai: Option<AiBackend>, found: impl Fn(&str) -> bool) -> Backend {
    if let Some(ai) = ai {
        return backend_entry(ai);
    }
    AI_BACKENDS
        .iter()
        .copied()
        .find(|b| found(b.name))
        .unwrap_or_else(|| backend_entry(AiBackend::Opencode))
}

impl Backend {
    /// `<name>_router`: the machine `default_router` names.
    fn router(&self) -> String {
        format!("{}_router", self.name)
    }

    /// `machines/<name>_router.yml`.
    fn router_yml(&self) -> String {
        self.fill(ROUTER_YML)
    }

    /// `scripts/<name>_router/ask_<name>.sh`.
    fn router_ask_sh(&self) -> String {
        self.fill(ROUTER_ASK_SH)
    }

    fn fill(&self, template: &str) -> String {
        template
            .replace("{ai_title}", self.title)
            .replace("{ai_call}", self.ask)
            .replace("{ai_cli}", self.invoke)
            .replace("{ai}", self.name)
    }
}

/// Detect shared routines in `~/.decree/routines/`.
fn detect_shared_routines() -> Vec<String> {
    let shared_dir = config::expand_tilde("~/.decree/routines");
    if !shared_dir.is_dir() {
        return Vec::new();
    }

    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&shared_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|ext| ext == "sh") {
                if let Some(stem) = path.file_stem() {
                    names.push(stem.to_string_lossy().to_string());
                }
            }
        }
    }
    names.sort();
    names
}

/// Replace AI placeholders in a routine template.
fn replace_ai_placeholders(template: &str, backend: Backend) -> String {
    template
        .replace("{ai_name}", backend.name)
        .replace("{ai_invoke}", backend.invoke)
}

/// Create a default permissions file for the selected AI backend.
fn create_permissions_file(ai_name: &str) -> Result<(), DecreeError> {
    match ai_name {
        "claude" => {
            std::fs::create_dir_all(".claude")?;
            let settings_path = ".claude/settings.json";
            if Path::new(settings_path).exists() {
                println!(
                    "Note: .claude/settings.json already exists — add \"Write\" and \"Edit\" to the allow list manually."
                );
            } else {
                std::fs::write(
                    settings_path,
                    "{\n  \"permissions\": {\n    \"allow\": [\n      \"Write\",\n      \"Edit\"\n    ]\n  }\n}\n",
                )?;
                println!("Created .claude/settings.json with Write and Edit permissions.");
            }
        }
        "opencode" => {
            let settings_path = "opencode.json";
            if Path::new(settings_path).exists() {
                println!(
                    "Note: opencode.json already exists — add Write and Edit permissions manually."
                );
            } else {
                std::fs::write(
                    settings_path,
                    "{\n  \"$schema\": \"https://opencode.ai/config.json\",\n  \"autoshare\": false\n}\n",
                )?;
                println!("Created opencode.json (configure permissions via opencode's settings).");
            }
        }
        _ => {
            println!(
                "Note: Automatic permission setup is not supported for {ai_name}. Configure Write and Edit permissions in your AI tool's project settings."
            );
        }
    }
    Ok(())
}

/// Generate config.yml content, with the selected backend's router as `default_router`.
fn generate_config(
    backend: Backend,
    git_hooks: bool,
    routine_names: &[&str],
    shared_routine_names: &[String],
) -> String {
    let mut config = String::new();

    config.push_str(&format!(
        "default_router: {} # router machine for choose: model invokes that name none\n",
        backend.router()
    ));
    config.push_str("max_attempts: 3\n");
    config.push_str("max_depth: 10\n");
    config.push_str("max_log_size: 2097152 # Per-log size cap in bytes (2MB), 0 to disable\n");
    config.push_str("default_routine: develop\n");
    config
        .push_str("routine_source: \"~/.decree/routines\" # optional, shared routines directory\n");
    config.push('\n');

    config.push_str("hooks:\n");
    config.push_str("  beforeAll: \"\"\n");
    config.push_str("  afterAll: \"\"\n");

    if git_hooks {
        config.push_str("  beforeEach: \"git-baseline\"\n");
        config.push_str("  afterEach: \"git-stash-changes\"\n");
    } else {
        config.push_str("  beforeEach: \"\"\n");
        config.push_str("  afterEach: \"\"\n");
    }

    config.push_str("  # --- Git stash workflow (uncomment to enable) ---\n");
    config.push_str("  # beforeEach: \"git-baseline\"\n");
    config.push_str("  # afterEach: \"git-stash-changes\"\n");

    // Routine registry
    if !routine_names.is_empty() {
        config.push('\n');
        config.push_str("routines:\n");
        let mut sorted: Vec<&str> = routine_names.to_vec();
        sorted.sort();
        for name in sorted {
            config.push_str(&format!("  {name}:\n    enabled: true\n"));
        }
    }

    // Shared routine registry
    if !shared_routine_names.is_empty() {
        config.push('\n');
        config.push_str("shared_routines:\n");
        let mut sorted = shared_routine_names.to_vec();
        sorted.sort();
        for name in sorted {
            config.push_str(&format!("  {name}:\n    enabled: false\n"));
        }
    }

    config
}

/// Write `machines/<ai>_router.yml` and its executable script
/// `scripts/<ai>_router/ask_<ai>.sh` under `decree_dir`.
fn write_router(decree_dir: &Path, backend: Backend) -> Result<(), DecreeError> {
    let router = backend.router();
    let machines = decree_dir.join(MACHINES_DIR);
    std::fs::create_dir_all(&machines)?;
    std::fs::write(machines.join(format!("{router}.yml")), backend.router_yml())?;
    let scripts = decree_dir.join(SCRIPTS_DIR).join(&router);
    std::fs::create_dir_all(&scripts)?;
    let script = scripts.join(format!("ask_{}.sh", backend.name));
    std::fs::write(&script, backend.router_ask_sh())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Run `decree init`. Never prompts; refuses to touch an existing `.decree/`.
pub fn run(ai: Option<AiBackend>, permissions: bool) -> Result<(), DecreeError> {
    let decree_dir = Path::new(config::DECREE_DIR);
    if decree_dir.exists() {
        return Err(DecreeError::AlreadyInitialized);
    }

    // 1. Pick the AI backend
    let backend = select_backend(ai, command_exists);
    let ai_name = backend.name;
    if ai.is_none() && !command_exists(ai_name) {
        println!("No AI backend detected (opencode, claude, copilot); defaulting to opencode.");
        println!("Visit https://opencode.ai/ to install opencode, or pass --ai.");
    }
    println!("AI backend: {ai_name}");

    // 2. Default permissions for the selected AI backend
    if permissions {
        create_permissions_file(ai_name)?;
    }

    // 3. Detect git (hook scripts are written but not enabled by default)
    let has_git = command_exists("git") && is_git_repo();
    let git_hooks = false;

    // 4. Create directory structure
    let dirs = [
        config::DECREE_DIR,
        &format!("{}/{}", config::DECREE_DIR, config::ROUTINES_DIR),
        &format!("{}/{}", config::DECREE_DIR, config::CRON_DIR),
        &format!("{}/{}", config::DECREE_DIR, config::INBOX_DIR),
        &format!(
            "{}/{}/{}",
            config::DECREE_DIR,
            config::INBOX_DIR,
            config::DEAD_DIR
        ),
        &format!("{}/{}", config::DECREE_DIR, config::OUTBOX_DIR),
        &format!(
            "{}/{}/{}",
            config::DECREE_DIR,
            config::OUTBOX_DIR,
            config::DEAD_DIR
        ),
        &format!("{}/{}", config::DECREE_DIR, config::RUNS_DIR),
        &format!("{}/{}", config::DECREE_DIR, config::MIGRATIONS_DIR),
    ];
    for dir in &dirs {
        std::fs::create_dir_all(dir)?;
    }

    // 5. Write config.yml
    let mut routine_names: Vec<&str> = vec!["develop", "rust-develop"];
    if has_git {
        routine_names.push("git-baseline");
        routine_names.push("git-stash-changes");
    }

    // Check for shared routines at ~/.decree/routines/
    let shared_routine_names = detect_shared_routines();

    let config_content = generate_config(backend, git_hooks, &routine_names, &shared_routine_names);
    std::fs::write(
        format!("{}/{}", config::DECREE_DIR, config::CONFIG_FILE),
        &config_content,
    )?;

    // 5. Write .gitignore
    std::fs::write(
        format!("{}/{}", config::DECREE_DIR, config::GITIGNORE_FILE),
        DECREE_GITIGNORE,
    )?;

    // 6. Write router.md
    std::fs::write(
        format!("{}/{}", config::DECREE_DIR, config::ROUTER_FILE),
        ROUTER_MD,
    )?;

    // 7. Write routine templates (replace {ai_name}/{ai_invoke} with detected backend)
    let routines_base = format!("{}/{}", config::DECREE_DIR, config::ROUTINES_DIR);
    std::fs::write(
        format!("{routines_base}/develop.sh"),
        replace_ai_placeholders(DEVELOP_SH, backend),
    )?;
    std::fs::write(
        format!("{routines_base}/rust-develop.sh"),
        replace_ai_placeholders(RUST_DEVELOP_SH, backend),
    )?;

    // 9. Write git hook routines when inside a git repo (not enabled by default)
    if has_git {
        std::fs::write(format!("{routines_base}/git-baseline.sh"), GIT_BASELINE_SH)?;
        std::fs::write(
            format!("{routines_base}/git-stash-changes.sh"),
            GIT_STASH_CHANGES_SH,
        )?;
    }

    // 10. Write empty processed.md tracker
    std::fs::write(
        format!("{}/{}", config::DECREE_DIR, config::PROCESSED_FILE),
        "",
    )?;

    // 11. Write the router machine and its script (section 7, The default router)
    write_router(decree_dir, backend)?;

    // Make routine scripts executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let routines_path = Path::new(&routines_base);
        if let Ok(entries) = std::fs::read_dir(routines_path) {
            for entry in entries.flatten() {
                if entry.path().extension().is_some_and(|ext| ext == "sh") {
                    let mut perms = std::fs::metadata(entry.path())?.permissions();
                    perms.set_mode(0o755);
                    std::fs::set_permissions(entry.path(), perms)?;
                }
            }
        }
    }

    // Install the decree skill for the selected AI at project scope.
    skill::install_for_init(ai_name)?;

    // Draw the machines, so `.decree/graph/` is current from the start.
    graph::write(Path::new("."))?;

    println!("Decree initialized successfully.");
    if has_git {
        println!(
            "Tip: Git lifecycle hooks are available (git-baseline, git-stash-changes).\n     \
             To enable them, set beforeEach/afterEach in .decree/config.yml."
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude() -> Backend {
        backend_entry(AiBackend::Claude)
    }

    fn opencode() -> Backend {
        backend_entry(AiBackend::Opencode)
    }

    #[test]
    fn test_generate_config_without_git_hooks() {
        let config = generate_config(claude(), false, &["develop", "rust-develop"], &[]);
        assert!(config.starts_with("default_router: claude_router "));
        assert!(!config.contains("commands:"));
        assert!(!config.contains("ai_router"));
        assert!(config.contains("max_attempts: 3"));
        assert!(config.contains("beforeEach: \"\""));
        assert!(config.contains("# beforeEach: \"git-baseline\""));
        assert!(config.contains("routine_source: \"~/.decree/routines\""));
        assert!(config.contains("routines:\n"));
        assert!(config.contains("  develop:\n    enabled: true"));
        assert!(config.contains("  rust-develop:\n    enabled: true"));
    }

    #[test]
    fn test_generate_config_with_git_hooks() {
        let config = generate_config(
            opencode(),
            true,
            &[
                "develop",
                "rust-develop",
                "git-baseline",
                "git-stash-changes",
            ],
            &[],
        );
        assert!(config.starts_with("default_router: opencode_router "));
        assert!(config.contains("beforeEach: \"git-baseline\""));
        assert!(config.contains("afterEach: \"git-stash-changes\""));
        // Should still contain commented versions
        assert!(config.contains("# beforeEach: \"git-baseline\""));
        // Routines section should include hook routines
        assert!(config.contains("  git-baseline:\n    enabled: true"));
        assert!(config.contains("  git-stash-changes:\n    enabled: true"));
    }

    #[test]
    fn test_generate_config_with_shared_routines() {
        let config = generate_config(
            claude(),
            false,
            &["develop"],
            &["deploy".to_string(), "notify".to_string()],
        );
        assert!(config.contains("shared_routines:\n"));
        assert!(config.contains("  deploy:\n    enabled: false"));
        assert!(config.contains("  notify:\n    enabled: false"));
    }

    /// The generated config.yml loads as the typed config, with and without hooks.
    #[test]
    fn test_generate_config_parses_as_app_config() {
        let configs = [
            generate_config(claude(), false, &["develop", "rust-develop"], &[]),
            generate_config(
                opencode(),
                true,
                &[
                    "develop",
                    "rust-develop",
                    "git-baseline",
                    "git-stash-changes",
                ],
                &["deploy".to_string()],
            ),
        ];
        for config in configs {
            serde_norway::from_str::<config::AppConfig>(&config).unwrap();
        }
    }

    /// The config `init` writes loads from disk as the typed config with the defaults.
    #[test]
    fn test_generate_config_loads_as_app_config() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join(config::CONFIG_FILE);
        let content = generate_config(opencode(), false, &["develop", "rust-develop"], &[]);
        std::fs::write(&path, content).unwrap();

        let config = config::AppConfig::load(&path).unwrap();
        assert_eq!(config.max_attempts, 3);
        assert_eq!(config.max_depth, 10);
        assert_eq!(config.max_log_size, 2_097_152);
        assert_eq!(config.default_routine, "develop");
        assert_eq!(config.default_router.as_deref(), Some("opencode_router"));
    }

    #[test]
    fn test_select_backend_flag_wins_over_detection() {
        assert_eq!(select_backend(Some(AiBackend::Claude), |_| true), claude());
        assert_eq!(
            select_backend(Some(AiBackend::Copilot), |_| false).invoke,
            "copilot -p"
        );
    }

    #[test]
    fn test_select_backend_detects_in_0_4_2_order() {
        assert_eq!(select_backend(None, |_| true).name, "opencode");
        assert_eq!(select_backend(None, |c| c != "opencode").name, "claude");
        assert_eq!(select_backend(None, |c| c == "copilot").name, "copilot");
    }

    #[test]
    fn test_select_backend_defaults_to_opencode() {
        assert_eq!(select_backend(None, |_| false), opencode());
    }

    #[test]
    fn test_command_exists_true() {
        // `ls` should exist on any system
        assert!(command_exists("ls"));
    }

    #[test]
    fn test_command_exists_false() {
        assert!(!command_exists("definitely_not_a_real_command_xyz"));
    }

    #[test]
    fn test_develop_template_has_precheck() {
        assert!(DEVELOP_SH.contains("DECREE_PRE_CHECK"));
        assert!(DEVELOP_SH.contains("{ai_name}"));
        assert!(DEVELOP_SH.contains("{ai_invoke}"));
    }

    #[test]
    fn test_rust_develop_template_has_precheck() {
        assert!(RUST_DEVELOP_SH.contains("DECREE_PRE_CHECK"));
        assert!(RUST_DEVELOP_SH.contains("{ai_name}"));
        assert!(RUST_DEVELOP_SH.contains("{ai_invoke}"));
        assert!(RUST_DEVELOP_SH.contains("cargo"));
    }

    #[test]
    fn test_develop_template_has_description_header() {
        // First non-shebang comment line is the title
        let lines: Vec<&str> = DEVELOP_SH.lines().collect();
        assert_eq!(lines[1], "# Develop");
        assert_eq!(lines[2], "#");
        // Description follows
        assert!(lines[3].starts_with("# "));
    }

    #[test]
    fn test_rust_develop_template_has_description_header() {
        let lines: Vec<&str> = RUST_DEVELOP_SH.lines().collect();
        assert_eq!(lines[1], "# Rust Develop");
        assert_eq!(lines[2], "#");
        assert!(lines[3].starts_with("# "));
    }

    #[test]
    fn test_develop_template_references_message_dir() {
        assert!(DEVELOP_SH.contains("${message_dir}"));
    }

    #[test]
    fn test_rust_develop_template_references_message_dir() {
        assert!(RUST_DEVELOP_SH.contains("${message_dir}"));
    }

    #[test]
    fn test_precheck_prints_to_stderr() {
        // Both routines should print errors to stderr (>&2)
        assert!(DEVELOP_SH.contains(">&2"));
        assert!(RUST_DEVELOP_SH.contains(">&2"));
    }

    #[test]
    fn test_router_has_placeholders() {
        assert!(ROUTER_MD.contains("{routines}"));
        assert!(ROUTER_MD.contains("{message}"));
    }

    #[test]
    fn test_gitignore_content() {
        assert!(DECREE_GITIGNORE.contains("inbox/"));
        assert!(DECREE_GITIGNORE.contains("outbox/"));
        assert!(DECREE_GITIGNORE.contains("runs/"));
    }

    #[test]
    fn test_ai_placeholder_replacement() {
        let replaced = replace_ai_placeholders(DEVELOP_SH, claude());
        // {ai_invoke} is replaced with the command; the prompt is built into a
        // variable that is echoed (for visibility) then passed to the AI.
        assert!(replaced.contains("claude -p ${resume_flag} \"${implement_prompt}\""));
        assert!(replaced.contains("implement_prompt=\"Read"));
        assert!(replaced.contains("command -v claude"));
        assert!(!replaced.contains("{ai_name}"));
        assert!(!replaced.contains("{ai_invoke}"));
    }

    fn mock(path: &str) -> String {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(root.join("mock/.decree").join(path)).unwrap()
    }

    /// `claude_router` and `ask_claude` are the mock's, byte for byte (section 7, The
    /// default router).
    #[test]
    fn test_claude_router_is_the_mocks() {
        assert_eq!(claude().router_yml(), mock("machines/claude_router.yml"));
        assert_eq!(
            claude().router_ask_sh(),
            mock("scripts/claude_router/ask_claude.sh")
        );
    }

    /// The copilot and opencode routers are claude's with their own names and CLI call.
    #[test]
    fn test_routers_differ_only_in_names_and_the_cli_call() {
        let claude = claude();
        for ai in [AiBackend::Copilot, AiBackend::Opencode] {
            let b = backend_entry(ai);
            let swap = |text: String| {
                text.replace(claude.ask, b.ask)
                    .replace(claude.invoke, b.invoke)
                    .replace(claude.title, b.title)
                    .replace(claude.name, b.name)
            };
            assert_eq!(swap(claude.router_yml()), b.router_yml(), "{}", b.name);
            assert_eq!(
                swap(claude.router_ask_sh()),
                b.router_ask_sh(),
                "{}",
                b.name
            );
            assert!(b
                .router_yml()
                .contains(&format!("name: {}_router\n", b.name)));
            assert!(b
                .router_yml()
                .contains(&format!("invoke: ask_{}\n", b.name)));
            assert!(b.router_ask_sh().contains(&format!("reply=$({})\n", b.ask)));
        }
    }

    #[test]
    fn test_write_router_writes_the_machine_and_an_executable_script() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        write_router(dir.path(), backend_entry(AiBackend::Copilot)).unwrap();
        let yml = std::fs::read_to_string(dir.path().join("machines/copilot_router.yml")).unwrap();
        assert!(yml.contains("name: copilot_router\n"));
        let script = dir.path().join("scripts/copilot_router/ask_copilot.sh");
        let mode = std::fs::metadata(&script).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    #[test]
    fn test_git_baseline_has_precheck() {
        assert!(GIT_BASELINE_SH.contains("DECREE_PRE_CHECK"));
        assert!(GIT_BASELINE_SH.contains("git rev-parse --is-inside-work-tree"));
    }

    #[test]
    fn test_git_baseline_has_description_header() {
        let lines: Vec<&str> = GIT_BASELINE_SH.lines().collect();
        assert_eq!(lines[1], "# Git Baseline");
        assert_eq!(lines[2], "#");
        assert!(lines[3].starts_with("# "));
    }

    #[test]
    fn test_git_baseline_uses_env_vars() {
        assert!(GIT_BASELINE_SH.contains("DECREE_ATTEMPT"));
        assert!(GIT_BASELINE_SH.contains("DECREE_MAX_ATTEMPTS"));
    }

    #[test]
    fn test_git_baseline_named_stashes() {
        assert!(GIT_BASELINE_SH.contains("decree-baseline: ${message_id}"));
        assert!(GIT_BASELINE_SH.contains("decree-failed: ${message_id}"));
    }

    #[test]
    fn test_git_baseline_has_parameters() {
        assert!(GIT_BASELINE_SH.contains("message_file="));
        assert!(GIT_BASELINE_SH.contains("message_id="));
        assert!(GIT_BASELINE_SH.contains("message_dir="));
        assert!(GIT_BASELINE_SH.contains("chain="));
        assert!(GIT_BASELINE_SH.contains("seq="));
    }

    #[test]
    fn test_git_baseline_no_destructive_commands() {
        assert!(!GIT_BASELINE_SH.contains("git reset"));
        assert!(!GIT_BASELINE_SH.contains("git clean"));
        assert!(!GIT_BASELINE_SH.contains("git checkout ."));
    }

    #[test]
    fn test_git_stash_changes_has_precheck() {
        assert!(GIT_STASH_CHANGES_SH.contains("DECREE_PRE_CHECK"));
    }

    #[test]
    fn test_git_stash_changes_has_description_header() {
        let lines: Vec<&str> = GIT_STASH_CHANGES_SH.lines().collect();
        assert_eq!(lines[1], "# Git Stash Changes");
        assert_eq!(lines[2], "#");
        assert!(lines[3].starts_with("# "));
    }

    #[test]
    fn test_git_stash_changes_uses_env_vars() {
        assert!(GIT_STASH_CHANGES_SH.contains("DECREE_ATTEMPT"));
        assert!(GIT_STASH_CHANGES_SH.contains("DECREE_MAX_ATTEMPTS"));
        assert!(GIT_STASH_CHANGES_SH.contains("DECREE_ROUTINE_EXIT_CODE"));
    }

    #[test]
    fn test_git_stash_changes_named_stashes() {
        assert!(GIT_STASH_CHANGES_SH.contains("decree: ${message_id} attempt ${ATTEMPT}"));
        assert!(GIT_STASH_CHANGES_SH.contains("decree-exhausted: ${message_id}"));
    }

    #[test]
    fn test_git_stash_changes_has_parameters() {
        assert!(GIT_STASH_CHANGES_SH.contains("message_file="));
        assert!(GIT_STASH_CHANGES_SH.contains("message_id="));
        assert!(GIT_STASH_CHANGES_SH.contains("message_dir="));
        assert!(GIT_STASH_CHANGES_SH.contains("chain="));
        assert!(GIT_STASH_CHANGES_SH.contains("seq="));
    }

    #[test]
    fn test_git_stash_changes_no_destructive_commands() {
        assert!(!GIT_STASH_CHANGES_SH.contains("git reset"));
        assert!(!GIT_STASH_CHANGES_SH.contains("git clean"));
        assert!(!GIT_STASH_CHANGES_SH.contains("git checkout ."));
    }

    #[test]
    fn test_git_stash_changes_restores_baseline_on_exhaustion() {
        // Should restore baseline when exit code != 0 and attempt == max_attempts
        assert!(GIT_STASH_CHANGES_SH.contains("EXIT_CODE\" -ne 0"));
        assert!(GIT_STASH_CHANGES_SH.contains("ATTEMPT\" -eq \"$MAX_ATTEMPTS\""));
        assert!(GIT_STASH_CHANGES_SH.contains("decree-baseline: ${message_id}"));
    }

    #[test]
    fn test_git_baseline_restores_on_final_retry() {
        // Final attempt should stash failed changes and restore baseline
        assert!(GIT_BASELINE_SH.contains("ATTEMPT\" -eq \"$MAX_ATTEMPTS\""));
        assert!(GIT_BASELINE_SH.contains("git stash apply"));
    }
}
