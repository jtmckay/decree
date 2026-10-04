use crate::cli::AiBackend;
use crate::commands::graph;
use crate::error::DecreeError;
use crate::layout;
use crate::machine::{MACHINES_DIR, ROUTER_MACHINE};
use crate::runtime::SCRIPTS_DIR;
use std::path::Path;
use std::process::Command;

/// An AI backend: its CLI, and how the routine templates and its router call it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Backend {
    /// The command, which also names the router's script (`ask_<name>`).
    name: &'static str,
    /// The name the router machine's description and script use.
    title: &'static str,
    /// The non-interactive prompt command, prompt last: `{ai_cli}` in the router.
    invoke: &'static str,
    /// The line of `ask_<name>` that sends `$prompt` and prints the reply.
    ask: &'static str,
    /// The bash function `ai <prompt>` that the develop machines' scripts call.
    ai_function: &'static str,
    /// Where `init` writes the decree skill, relative to the project root; `None` if
    /// the backend reads no skills.
    skill_dir: Option<&'static str>,
}

/// AI backends in 0.4.2's detection order. The routers differ only in the CLI call.
const AI_BACKENDS: &[Backend] = &[
    Backend {
        name: "opencode",
        title: "OpenCode",
        invoke: "opencode run",
        ask: "opencode run \"$prompt\"",
        ai_function: AI_PLAIN_SH,
        skill_dir: None,
    },
    Backend {
        name: "claude",
        title: "Claude",
        invoke: "claude -p",
        ask: "printf '%s' \"$prompt\" | claude -p",
        ai_function: AI_CLAUDE_SH,
        skill_dir: Some(".claude/skills/decree"),
    },
    Backend {
        name: "copilot",
        title: "Copilot",
        invoke: "copilot -p",
        ask: "copilot -p \"$prompt\"",
        ai_function: AI_PLAIN_SH,
        skill_dir: Some(".github/skills/decree"),
    },
];

/// The router machine `init` writes (docs/reference/runs.md, The default router), and its script.
const ROUTER_YML: &str = include_str!("../templates/router/router.yml");
const ROUTER_ASK_SH: &str = include_str!("../templates/router/ask.sh");

/// A built-in machine `init` writes, with its scripts (`scripts/<name>/<script>.sh`).
struct BuiltinMachine {
    name: &'static str,
    yml: &'static str,
    /// Script name and template.
    scripts: &'static [(&'static str, &'static str)],
}

const DEVELOP_MACHINES: &[BuiltinMachine] = &[
    BuiltinMachine {
        name: "develop",
        yml: include_str!("../templates/machines/develop.yml"),
        scripts: &[
            (
                "precheck",
                include_str!("../templates/scripts/develop/precheck.sh"),
            ),
            (
                "implement",
                include_str!("../templates/scripts/develop/implement.sh"),
            ),
            (
                "verify",
                include_str!("../templates/scripts/develop/verify.sh"),
            ),
        ],
    },
    BuiltinMachine {
        name: "rust_develop",
        yml: include_str!("../templates/machines/rust_develop.yml"),
        scripts: &[
            (
                "precheck",
                include_str!("../templates/scripts/rust_develop/precheck.sh"),
            ),
            (
                "implement",
                include_str!("../templates/scripts/rust_develop/implement.sh"),
            ),
            (
                "gate",
                include_str!("../templates/scripts/rust_develop/gate.sh"),
            ),
            (
                "qa",
                include_str!("../templates/scripts/rust_develop/qa.sh"),
            ),
        ],
    },
];

/// Shared scripts `init` writes to the flat `scripts/` (docs/reference/scripts.md, Resolution):
/// 0.4.2's git-stash hooks as per-visit scripts (docs/decisions.md, D15). `git_baseline` is a root
/// `onentry` that records `HEAD` once; `snapshot` is a working state's `onentry` that
/// stashes a checkpoint on each visit. No built-in machine uses them; add them where wanted.
const SHARED_SCRIPTS: &[(&str, &str)] = &[
    (
        "git_baseline",
        include_str!("../templates/scripts/git_baseline.sh"),
    ),
    ("snapshot", include_str!("../templates/scripts/snapshot.sh")),
];

/// `ai <prompt>` for Claude: waits out its usage limit and resumes the session.
const AI_CLAUDE_SH: &str = include_str!("../templates/ai/claude.sh");
/// `ai <prompt>` for the other backends: one call.
const AI_PLAIN_SH: &str = include_str!("../templates/ai/plain.sh");

const DECREE_GITIGNORE: &str = include_str!("../templates/gitignore");

/// The decree skill `init` writes: path under the skill directory, and content.
const DECREE_SKILL: &[(&str, &str)] = &[
    (
        "SKILL.md",
        include_str!("../templates/skills/decree/SKILL.md"),
    ),
    (
        "reference/machines.md",
        include_str!("../templates/skills/decree/reference/machines.md"),
    ),
    (
        "reference/messages.md",
        include_str!("../templates/skills/decree/reference/messages.md"),
    ),
    (
        "reference/runs.md",
        include_str!("../templates/skills/decree/reference/runs.md"),
    ),
    (
        "reference/scripts.md",
        include_str!("../templates/skills/decree/reference/scripts.md"),
    ),
];

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
    /// `machines/router.yml`.
    fn router_yml(&self) -> String {
        self.fill(ROUTER_YML)
    }

    /// `scripts/router/ask_<name>.sh`.
    fn router_ask_sh(&self) -> String {
        self.fill(ROUTER_ASK_SH)
    }

    fn fill(&self, template: &str) -> String {
        template
            .replace("{ai_function}", self.ai_function.trim_end())
            .replace("{ai_title}", self.title)
            .replace("{ai_call}", self.ask)
            .replace("{ai_cli}", self.invoke)
            .replace("{ai}", self.name)
    }
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

/// Write an executable script.
fn write_script(path: &Path, content: &str) -> Result<(), DecreeError> {
    std::fs::write(path, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Write the `develop` and `rust_develop` machines and their executable scripts
/// (`scripts/<machine>/<name>.sh`) under `decree_dir`, for `backend`.
fn write_develop_machines(decree_dir: &Path, backend: Backend) -> Result<(), DecreeError> {
    for machine in DEVELOP_MACHINES {
        std::fs::write(
            decree_dir
                .join(MACHINES_DIR)
                .join(format!("{}.yml", machine.name)),
            backend.fill(machine.yml),
        )?;
        let dir = decree_dir.join(SCRIPTS_DIR).join(machine.name);
        std::fs::create_dir_all(&dir)?;
        for (name, script) in machine.scripts {
            write_script(&dir.join(format!("{name}.sh")), &backend.fill(script))?;
        }
    }
    Ok(())
}

/// Write the `SHARED_SCRIPTS` as executable `scripts/<name>.sh` under `decree_dir`.
fn write_shared_scripts(decree_dir: &Path) -> Result<(), DecreeError> {
    let dir = decree_dir.join(SCRIPTS_DIR);
    std::fs::create_dir_all(&dir)?;
    for (name, script) in SHARED_SCRIPTS {
        write_script(&dir.join(format!("{name}.sh")), script)?;
    }
    Ok(())
}

/// Write `machines/router.yml` and its executable script `scripts/router/ask_<ai>.sh`
/// under `decree_dir` (docs/reference/runs.md, The default router).
fn write_router(decree_dir: &Path, backend: Backend) -> Result<(), DecreeError> {
    let machines = decree_dir.join(MACHINES_DIR);
    std::fs::create_dir_all(&machines)?;
    std::fs::write(
        machines.join(format!("{ROUTER_MACHINE}.yml")),
        backend.router_yml(),
    )?;
    let scripts = decree_dir.join(SCRIPTS_DIR).join(ROUTER_MACHINE);
    std::fs::create_dir_all(&scripts)?;
    write_script(
        &scripts.join(format!("ask_{}.sh", backend.name)),
        &backend.router_ask_sh(),
    )
}

/// Run `decree init`. Never prompts; refuses to touch an existing `.decree/`.
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

    write_layout(decree_dir, backend)?;
    write_skill(Path::new("."), backend)?;
    // Draw the machines, so `.decree/graph/` is current from the start.
    graph::write(Path::new("."))?;

    println!("Decree initialized successfully.");
    Ok(())
}

/// Write the `.decree/` layout (docs/reference/README.md) under `decree_dir`: `.gitignore`,
/// `processed.md`, the empty queues, the router machine with its script, the
/// `develop` and `rust_develop` machines with theirs, and the shared scripts.
/// `graph/` is written by `decree graph`.
fn write_layout(decree_dir: &Path, backend: Backend) -> Result<(), DecreeError> {
    for dir in [
        layout::MIGRATIONS_DIR,
        layout::INBOX_DIR,
        layout::RUNS_DIR,
        layout::CRON_DIR,
        MACHINES_DIR,
        SCRIPTS_DIR,
    ] {
        std::fs::create_dir_all(decree_dir.join(dir))?;
    }
    std::fs::write(decree_dir.join(layout::GITIGNORE_FILE), DECREE_GITIGNORE)?;
    std::fs::write(decree_dir.join(layout::PROCESSED_FILE), "")?;
    write_router(decree_dir, backend)?;
    write_develop_machines(decree_dir, backend)?;
    write_shared_scripts(decree_dir)
}

/// Write the decree skill under `root` for the backend. Never overwrites an existing file.
fn write_skill(root: &Path, backend: Backend) -> Result<(), DecreeError> {
    let Some(skill_dir) = backend.skill_dir else {
        return Ok(());
    };
    let skill_dir = root.join(skill_dir);
    let mut kept = 0;
    for (name, content) in DECREE_SKILL {
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
        DECREE_SKILL.len() - kept
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

    /// `write_layout` creates the docs/reference/README.md layout entries except `graph/`, which `decree graph` writes.
    #[test]
    fn test_write_layout_writes_the_section_3_entries() {
        let dir = tempfile::TempDir::new().unwrap();
        write_layout(dir.path(), claude()).unwrap();
        let mut names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                ".gitignore",
                "cron",
                "inbox",
                "machines",
                "migrations",
                "processed.md",
                "runs",
                "scripts"
            ]
        );
        assert!(dir.path().join("machines/router.yml").is_file());
        assert!(dir.path().join("scripts/router/ask_claude.sh").is_file());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("processed.md")).unwrap(),
            ""
        );
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
    fn test_gitignore_content() {
        assert_eq!(DECREE_GITIGNORE, "inbox/\nruns/\n");
    }

    fn mock(path: &str) -> String {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(root.join("mock/.decree").join(path)).unwrap()
    }

    /// Claude's `router` and `ask_claude` are the mock's, byte for byte (docs/reference/runs.md, The
    /// default router).
    #[test]
    fn test_router_for_claude_is_the_mocks() {
        assert_eq!(claude().router_yml(), mock("machines/router.yml"));
        assert_eq!(
            claude().router_ask_sh(),
            mock("scripts/router/ask_claude.sh")
        );
    }

    /// The copilot and opencode routers are claude's with their own script and CLI call.
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
            assert!(b.router_yml().contains("name: router\n"));
            assert!(b
                .router_yml()
                .contains(&format!("invoke: ask_{}\n", b.name)));
            assert!(b.router_ask_sh().contains(&format!("reply=$({})\n", b.ask)));
        }
    }

    /// `develop` and `rust_develop` are written with every script executable and every
    /// placeholder filled; only Claude's scripts wait out a usage limit.
    #[test]
    fn test_write_develop_machines_writes_machines_and_executable_scripts() {
        use std::os::unix::fs::PermissionsExt;
        for ai in [AiBackend::Claude, AiBackend::Opencode, AiBackend::Copilot] {
            let b = backend_entry(ai);
            let dir = tempfile::TempDir::new().unwrap();
            std::fs::create_dir_all(dir.path().join(MACHINES_DIR)).unwrap();
            write_develop_machines(dir.path(), b).unwrap();
            for BuiltinMachine {
                name: machine,
                scripts,
                ..
            } in DEVELOP_MACHINES
            {
                let yml =
                    std::fs::read_to_string(dir.path().join(format!("machines/{machine}.yml")))
                        .unwrap();
                assert!(yml.contains(&format!("name: {machine}\n")));
                assert!(yml.contains(b.title), "{machine}");
                for (name, _) in *scripts {
                    let path = dir.path().join(format!("scripts/{machine}/{name}.sh"));
                    let text = std::fs::read_to_string(&path).unwrap();
                    assert!(!text.contains("{ai"), "{}: {text}", path.display());
                    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
                    assert_eq!(mode & 0o777, 0o755, "{}", path.display());
                    let calls_ai = text.contains("\nai \"${prompt}\"\n");
                    assert_eq!(text.contains("\nai() {\n"), calls_ai, "{}", path.display());
                    assert_eq!(
                        text.contains("Usage limit reached"),
                        calls_ai && b.name == "claude",
                        "{}",
                        path.display()
                    );
                }
            }
        }
    }

    /// `git_baseline` and `snapshot` are the mock's, byte for byte, and executable.
    #[test]
    fn test_write_shared_scripts_writes_the_mocks_git_scripts() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        write_shared_scripts(dir.path()).unwrap();
        let mut names: Vec<String> = std::fs::read_dir(dir.path().join(SCRIPTS_DIR))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["git_baseline.sh", "snapshot.sh"]);
        for name in names {
            let path = dir.path().join(SCRIPTS_DIR).join(&name);
            let text = std::fs::read_to_string(&path).unwrap();
            assert_eq!(text, mock(&format!("scripts/{name}")), "{name}");
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755, "{name}");
        }
    }

    #[test]
    fn test_write_router_writes_the_machine_and_an_executable_script() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        write_router(dir.path(), backend_entry(AiBackend::Copilot)).unwrap();
        let yml = std::fs::read_to_string(dir.path().join("machines/router.yml")).unwrap();
        assert!(yml.contains("name: router\n"));
        assert!(yml.contains("invoke: ask_copilot\n"));
        let script = dir.path().join("scripts/router/ask_copilot.sh");
        let mode = std::fs::metadata(&script).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
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
        let mut names: Vec<String> = DECREE_SKILL.iter().map(|(n, _)| n.to_string()).collect();
        names.sort();
        names
    }

    /// `DECREE_SKILL` lists exactly the files in `src/templates/skills/decree/`.
    #[test]
    fn test_decree_skill_lists_every_template_file() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let dir = root.join("src/templates/skills/decree");
        assert_eq!(files_under(&dir), skill_names());
    }

    /// The skill mentions no 0.4 concept.
    #[test]
    fn test_decree_skill_mentions_no_0_4_concept() {
        for (name, content) in DECREE_SKILL {
            let lower = content.to_lowercase();
            for term in ["routine", "outbox", "hooks", "router.md"] {
                assert!(!lower.contains(term), "{name} mentions {term}");
            }
        }
    }

    /// SKILL.md points to every reference file, and the skill covers the 0.5 building
    /// blocks and commands.
    #[test]
    fn test_decree_skill_covers_messages_machines_scripts_and_commands() {
        let skill = DECREE_SKILL[0].1;
        assert!(skill.starts_with("---\nname: decree\ndescription: "));
        for (name, _) in &DECREE_SKILL[1..] {
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

    /// The skill's examples are the mock's: `hello` in SKILL.md, `feature`'s verify
    /// script, and the cron file.
    #[test]
    fn test_decree_skill_examples_are_the_mocks() {
        let text: String = DECREE_SKILL.iter().map(|(_, c)| *c).collect();
        let hello = mock("machines/hello.yml");
        assert!(text.contains(&hello), "hello.yml");
        let verify = mock("scripts/feature/verify.sh");
        let body = verify
            .lines()
            .filter(|l| !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains(body.trim()), "verify.sh");
        assert!(
            text.contains(&mock("cron/nightly-audit.md")),
            "nightly-audit.md"
        );
    }

    /// This repository's installed copies are the templates, byte for byte.
    #[test]
    fn test_repository_skill_copies_are_the_templates() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        for b in AI_BACKENDS {
            let Some(skill_dir) = b.skill_dir else {
                continue;
            };
            let dir = root.join(skill_dir);
            assert_eq!(files_under(&dir), skill_names(), "{skill_dir}");
            for (name, content) in DECREE_SKILL {
                let installed = std::fs::read_to_string(dir.join(name)).unwrap();
                assert_eq!(&installed, content, "{skill_dir}/{name}");
            }
        }
    }
}
