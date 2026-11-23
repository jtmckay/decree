use crate::cli::AiBackend;
use crate::commands::graph;
use crate::commands::plugin;
use crate::error::DecreeError;
use crate::layout;
use crate::official::skill_files;
use crate::plugin::PLUGINS_DIR;
use std::path::Path;
use std::process::Command;

/// An AI backend: its CLI, and where its skills go. The official plugins pick their backend
/// when they run (`ai.sh`), so `init` bakes it into no file but `.env.example`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Backend {
    /// The command, and `AI_CLI`'s value.
    name: &'static str,
    /// Where `init` writes the decree skill, relative to the project root; `None` if
    /// the backend reads no skills.
    skill_dir: Option<&'static str>,
}

const OPENCODE: Backend = Backend {
    name: "opencode",
    skill_dir: None,
};

const CLAUDE: Backend = Backend {
    name: "claude",
    skill_dir: Some(".claude/skills/decree"),
};

const COPILOT: Backend = Backend {
    name: "copilot",
    skill_dir: Some(".github/skills/decree"),
};

/// AI backends, in the order `init` looks for them on `PATH` to pick the skill.
const AI_BACKENDS: [Backend; 3] = [OPENCODE, CLAUDE, COPILOT];

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

/// The `AI_BACKENDS` entry for a backend.
fn backend_entry(ai: AiBackend) -> Backend {
    match ai {
        AiBackend::Opencode => OPENCODE,
        AiBackend::Claude => CLAUDE,
        AiBackend::Copilot => COPILOT,
    }
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
        .unwrap_or(OPENCODE)
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
                    "Note: opencode.json already exists — add \"permission\": {{ \"external_directory\": \"deny\" }} manually, so a write outside the project does not end the session."
                );
            } else {
                // `deny`, not OpenCode's default `ask`: `opencode run` cannot answer, so `ask`
                // rejects a write outside the project and ends the session with no reply;
                // `deny` rejects it and lets the agent go on (ai.sh, ai_opencode).
                std::fs::write(
                    settings_path,
                    "{\n  \"$schema\": \"https://opencode.ai/config.json\",\n  \"share\": \"manual\",\n  \"permission\": {\n    \"external_directory\": \"deny\"\n  }\n}\n",
                )?;
                println!("Created opencode.json: writes outside the project are denied, not asked about.");
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

/// Run `decree init`. Never prompts; refuses to touch an existing `.decree/`. Adds and installs
/// the default official plugins (`official::DEFAULTS`), then prints the others and the line
/// that adds one.
pub fn run(ai: Option<AiBackend>, permissions: bool) -> Result<(), DecreeError> {
    let decree_dir = Path::new(layout::DECREE_DIR);
    if decree_dir.exists() {
        return Err(DecreeError::AlreadyInitialized);
    }

    let backend = select_backend(ai, command_exists);
    let ai_name = backend.name;
    if ai.is_none() && !command_exists(ai_name) {
        println!("No AI backend detected (opencode, claude, copilot); defaulting to opencode.");
        println!("Visit https://opencode.ai/ to install opencode, or pass --ai.");
    }
    println!("AI backend: {ai_name}");

    if permissions {
        create_permissions_file(ai_name)?;
    }

    write_layout(decree_dir, ai.map(|_| backend))?;
    write_skill(Path::new("."), backend)?;
    add_defaults(decree_dir)?;
    // Installing writes the schemas, as `decree install` does.
    crate::commands::schema::write(Path::new("."))?;
    // Draw the machines, so each graph is current from the start.
    graph::write(Path::new("."))?;

    println!("Decree initialized successfully.");
    println!();
    plugin::print_available(decree_dir)?;
    println!("Delete a file in .decree/plugins/ to drop that plugin, then run `decree install`. Official plugins (decree_*) are never edited: a plugin of your own that extends one holds your changes. Add an official plugin with `decree install <name>`, such as `decree install decree_develop`.");
    Ok(())
}

/// Write each default official plugin into `plugins/` and install it.
fn add_defaults(decree_dir: &Path) -> Result<(), DecreeError> {
    for name in crate::official::DEFAULTS {
        let o = crate::official::get(name).ok_or_else(|| {
            DecreeError::Other(format!("official plugin `{name}` is not built in"))
        })?;
        let p = o.plugin().map_err(DecreeError::Other)?;
        std::fs::write(decree_dir.join(crate::plugin::plugin_file(name)), o.text)?;
        crate::plugin::install(decree_dir, &p, o.text.as_bytes())?;
        println!(
            "Added the official plugin `{name}` (delete .decree/plugins/{name}.md to drop it)."
        );
    }
    Ok(())
}

/// Write the `.decree/` layout (docs/reference/README.md) under `decree_dir`: `.gitignore`,
/// `.env.example`, the empty queues and the empty `plugins/`. No plugin is
/// installed, so `machines/` is not written. `decree graph` writes each machine's `graph.md`
/// and `system.md`; `schema/` is written as the defaults are installed.
fn write_layout(decree_dir: &Path, ai: Option<Backend>) -> Result<(), DecreeError> {
    for dir in [layout::INBOX_DIR, layout::RUNS_DIR, PLUGINS_DIR] {
        std::fs::create_dir_all(decree_dir.join(dir))?;
    }
    std::fs::write(decree_dir.join(layout::GITIGNORE_FILE), DECREE_GITIGNORE)?;
    std::fs::write(decree_dir.join(layout::ENV_EXAMPLE_FILE), env_example(ai))?;
    Ok(())
}

/// `.env.example`: `AI_CLI` set to the backend `--ai` named, or, without `--ai`, commented out,
/// so the official plugins pick the first AI CLI on `PATH`.
fn env_example(ai: Option<Backend>) -> String {
    let line = match ai {
        Some(b) => format!("AI_CLI={}\n", b.name),
        None => "# AI_CLI=claude\n".to_string(),
    };
    format!(
        "# Copy to .decree/.env (git ignores it) and set what your machines read.\n\
         # The AI CLI the official plugins ask: claude, opencode or copilot. Unset, they use\n\
         # the first of claude, opencode and copilot found on PATH; a run's `ai_cli` data wins.\n\
         {line}"
    )
}

/// Write the decree skill under `root` for the backend. Never overwrites an existing file.
fn write_skill(root: &Path, backend: Backend) -> Result<(), DecreeError> {
    let Some(skill_dir) = backend.skill_dir else {
        return Ok(());
    };
    let skill_dir = root.join(skill_dir);
    let files = skill_files();
    let mut kept = 0;
    for (name, content) in &files {
        let path = skill_dir.join(name);
        if path.exists() {
            kept += 1;
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)?;
    }
    println!(
        "Decree skill: {} ({} written, {kept} existing kept)",
        skill_dir.display(),
        files.len() - kept
    );
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
    fn test_select_backend_flag_wins_over_detection() {
        assert_eq!(select_backend(Some(AiBackend::Claude), |_| true), claude());
        assert_eq!(
            select_backend(Some(AiBackend::Copilot), |_| false).name,
            "copilot"
        );
    }

    #[test]
    fn test_select_backend_detects_in_path_order() {
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

    /// Cron file `file` of plugin `plugin` in `examples/project/.decree/plugins/`.
    fn example_plugin_cron(plugin: &str, file: &str) -> String {
        let text = example(&format!("plugins/{plugin}.md"));
        let found = crate::plugin::Plugin::parse(&text, Some(plugin)).unwrap();
        found.cron[file].clone()
    }

    /// A file under `examples/project/.decree/`.
    fn example(path: &str) -> String {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(root.join("examples/project/.decree").join(path)).unwrap()
    }

    /// A file under `tests/fixtures/feature/.decree/`: the files of the `feature` machine
    /// and the git scripts, which `examples/project` does not have.
    fn feature_fixture(path: &str) -> String {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(root.join("tests/fixtures/feature/.decree").join(path)).unwrap()
    }

    /// Every file under `dir`, as paths relative to it with `/` separators, sorted.
    fn files_under(dir: &Path) -> Vec<String> {
        fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(base, &path, out);
                } else {
                    let rel = path.strip_prefix(base).unwrap();
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, dir, &mut out);
        out.sort();
        out
    }

    fn skill_names() -> Vec<String> {
        let mut names: Vec<String> = skill_files().into_iter().map(|(n, _)| n).collect();
        names.sort();
        names
    }

    /// The `skill` plugin ships `SKILL.md` and the four reference files.
    #[test]
    fn test_decree_skill_lists_every_file() {
        assert_eq!(
            skill_names(),
            [
                "SKILL.md",
                "reference/machines.md",
                "reference/messages.md",
                "reference/runs.md",
                "reference/scripts.md"
            ]
        );
    }

    /// The skill names no `outbox`, `hooks` or `router.md`: decree has none of them.
    #[test]
    fn test_decree_skill_names_no_removed_concept() {
        for (name, content) in skill_files() {
            let lower = content.to_lowercase();
            for term in ["outbox", "hooks", "router.md"] {
                assert!(!lower.contains(term), "{name} mentions {term}");
            }
        }
    }

    /// SKILL.md points to every reference file, and the skill covers the 0.5 building
    /// blocks and commands.
    #[test]
    fn test_decree_skill_covers_messages_machines_scripts_and_commands() {
        let files = skill_files();
        let skill = &files[0].1;
        assert!(skill.starts_with("---\nname: decree\ndescription: "));
        for (name, _) in &files[1..] {
            assert!(
                skill.contains(&format!("`{name}`")),
                "SKILL.md omits {name}"
            );
        }
        for needle in [
            "message",
            "machine",
            "script",
            "decree check",
            "decree graph",
            "decree emit",
        ] {
            assert!(skill.contains(needle), "SKILL.md omits {needle}");
        }
    }

    /// The skill's examples are real files: `examples/project`'s `hello` plugin and cron file, and the
    /// `feature` fixture's verify script.
    #[test]
    fn test_decree_skill_examples_are_the_example_files() {
        let text: String = skill_files().into_iter().map(|(_, c)| c).collect();
        // `hello` is a plugin in `examples/project`; the skill shows its machine as written by
        // hand, as docs/reference/machines.md does (tests/docs_test.rs holds that equal to it).
        let plugin = example("plugins/hello.md");
        let installed = crate::plugin::Plugin::parse(&plugin, Some("hello")).unwrap();
        let shown = text
            .split("```yaml\n")
            .skip(1)
            .filter_map(|b| b.split_once("```").map(|(yaml, _)| yaml))
            .find(|yaml| yaml.lines().find(|l| !l.starts_with('#')) == Some("name: hello"))
            .expect("the skill shows `hello`");
        let shown: serde_norway::Value = serde_norway::from_str(shown).unwrap();
        assert_eq!(shown, installed.machine_value().unwrap(), "hello");
        let verify = feature_fixture("machines/feature/scripts/verify.sh");
        let body = verify
            .lines()
            .filter(|l| !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains(body.trim()), "verify.sh");
        assert!(
            text.contains(&example_plugin_cron("develop", "nightly-audit.md")),
            "nightly-audit.md"
        );
    }

    /// This repository's installed copies are the `decree_skill` plugin's, byte for byte.
    #[test]
    fn test_repository_skill_copies_are_the_templates() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        // The repository's skill folders link to its installed `decree_skill` plugin, which git
        // ignores: the plugin file it installs from is the copy to compare. Once installed
        // (`decree install`), the folders themselves are compared too.
        let found = crate::plugin::read_one(&root.join(".decree"), "decree_skill").unwrap();
        let plugin = found.parsed.unwrap();
        for (name, content) in skill_files() {
            assert_eq!(
                plugin.scripts.get(&format!("skill/{name}")),
                Some(&content),
                ".decree/plugins/decree_skill.md: skill/{name}"
            );
        }
        for b in AI_BACKENDS {
            let Some(skill_dir) = b.skill_dir else {
                continue;
            };
            let dir = root.join(skill_dir);
            assert_eq!(
                std::fs::read_link(&dir).unwrap(),
                Path::new("../../.decree/machines/decree_skill/scripts/skill"),
                "{skill_dir}"
            );
            if !dir.exists() {
                continue; // a fresh clone: not installed yet
            }
            assert_eq!(files_under(&dir), skill_names(), "{skill_dir}");
            for (name, content) in skill_files() {
                let installed = std::fs::read_to_string(dir.join(&name)).unwrap();
                assert_eq!(installed, content, "{skill_dir}/{name}");
            }
        }
    }
}
