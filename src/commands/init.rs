use crate::cli::AiBackend;
use crate::commands::graph;
use crate::commands::skill::DECREE_SKILL;
use crate::error::DecreeError;
use crate::layout;
use crate::machine::{MACHINES_DIR, ROUTER_MACHINE};
use crate::runtime::resolve::SCRIPTS_DIR;
use std::path::Path;
use std::process::Command;

/// An AI backend: its CLI, and how `develop`'s scripts and its router call it.
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
    /// Where `init` writes the decree skill, relative to the project root; `None` if
    /// the backend reads no skills.
    skill_dir: Option<&'static str>,
}

const OPENCODE: Backend = Backend {
    name: "opencode",
    title: "OpenCode",
    invoke: "opencode run",
    ask: "opencode run \"$prompt\"",
    skill_dir: None,
};

const CLAUDE: Backend = Backend {
    name: "claude",
    title: "Claude",
    invoke: "claude -p",
    ask: "printf '%s' \"$prompt\" | claude -p",
    skill_dir: Some(".claude/skills/decree"),
};

const COPILOT: Backend = Backend {
    name: "copilot",
    title: "Copilot",
    invoke: "copilot -p",
    ask: "copilot -p \"$prompt\"",
    skill_dir: Some(".github/skills/decree"),
};

/// AI backends, in the order `init` looks for them on `PATH`. The routers differ only in the
/// CLI call.
const AI_BACKENDS: [Backend; 3] = [OPENCODE, CLAUDE, COPILOT];

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

/// `develop`: implement a message, gate it, verify it.
const DEVELOP: BuiltinMachine = BuiltinMachine {
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
        ("gate", include_str!("../templates/scripts/develop/gate.sh")),
        ("fix", include_str!("../templates/scripts/develop/fix.sh")),
        (
            "verify",
            include_str!("../templates/scripts/develop/verify.sh"),
        ),
    ],
};

/// `infra`: declare what the environment needs in `lib/infra/requirements`, then apply it.
const INFRA: BuiltinMachine = BuiltinMachine {
    name: "infra",
    yml: include_str!("../templates/machines/infra.yml"),
    scripts: &[
        (
            "declare",
            include_str!("../templates/scripts/infra/declare.sh"),
        ),
        ("apply", include_str!("../templates/scripts/infra/apply.sh")),
    ],
};

/// `qa`: attack the project with chaos tests, and hand each real bug to `develop`.
const QA: BuiltinMachine = BuiltinMachine {
    name: "qa",
    yml: include_str!("../templates/machines/qa.yml"),
    scripts: &[
        ("recon", include_str!("../templates/scripts/qa/recon.sh")),
        ("attack", include_str!("../templates/scripts/qa/attack.sh")),
        ("run", include_str!("../templates/scripts/qa/run.sh")),
        ("triage", include_str!("../templates/scripts/qa/triage.sh")),
        ("report", include_str!("../templates/scripts/qa/report.sh")),
    ],
};

/// The built-in machines besides the router.
const BUILTIN_MACHINES: [&BuiltinMachine; 3] = [&DEVELOP, &INFRA, &QA];

/// The built-in machines' files in `lib/`, besides `ai.sh`: path, template, and whether it is
/// executable. `infra/` is the declared environment, which `infra` changes and `develop`'s
/// `precheck` checks: the declaration, the loop that checks or applies it, and the one kind
/// decree ships. `qa/` is qa's settings and guard, and the invariants and wrenches it reads.
const LIB_FILES: &[(&str, &str, bool)] = &[
    (
        "infra/requirements",
        include_str!("../templates/infra/requirements"),
        false,
    ),
    (
        "infra/run.sh",
        include_str!("../templates/infra/run.sh"),
        true,
    ),
    (
        "infra/kinds/command.sh",
        include_str!("../templates/infra/kinds/command.sh"),
        false,
    ),
    ("qa/qa.sh", include_str!("../templates/qa/qa.sh"), false),
    (
        "qa/invariants.md",
        include_str!("../templates/qa/invariants.md"),
        false,
    ),
    (
        "qa/wrenches.md",
        include_str!("../templates/qa/wrenches.md"),
        false,
    ),
];

/// Shared scripts `init` writes to the flat `scripts/` (docs/reference/scripts.md, Resolution;
/// docs/decisions.md, D15). Every built-in machine starts with `clean_tree` (a root `onentry`:
/// a clean git work tree) and ends with `commit` (`done`'s `onentry`: one commit per run) or
/// `keep` (`failed`'s `onentry`: the run's changes stay in the tree, and `.decree/HOLD` stops
/// the queue until the run is retried).
const SHARED_SCRIPTS: &[(&str, &str)] = &[
    (
        "clean_tree",
        include_str!("../templates/scripts/clean_tree.sh"),
    ),
    ("commit", include_str!("../templates/scripts/commit.sh")),
    ("keep", include_str!("../templates/scripts/keep.sh")),
];

/// `lib/ai.sh`: `ai <prompt>`, and a function for every backend, `{ai}` the one `init`
/// chose.
const AI_SH: &str = include_str!("../templates/ai/ai.sh");

/// The AI helper under `lib/`, which `develop`'s scripts source.
const LIB_AI_FILE: &str = "ai.sh";

const DECREE_GITIGNORE: &str = include_str!("../templates/gitignore");

/// `lib/README.md`: what goes in `lib/`.
const LIB_README: &str = "Code that scripts source (`. \"$DECREE_LIB/<file>\"`), config and data. \
                          decree never runs anything in it.\n";

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

/// Where decree writes the skill for `ai`, or, without it, for the backend `init` would
/// pick; `None` if that backend reads no skills. Also the backend's name, for messages.
pub(crate) fn skill_dir(ai: Option<AiBackend>) -> (&'static str, Option<&'static str>) {
    let backend = select_backend(ai, command_exists);
    (backend.name, backend.skill_dir)
}

/// Every directory decree writes the skill into, in detection order.
pub(crate) fn skill_dirs() -> impl Iterator<Item = &'static str> {
    AI_BACKENDS.iter().filter_map(|b| b.skill_dir)
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

    /// `lib/ai.sh`, with this backend as the default.
    fn lib_ai_sh(&self) -> String {
        self.fill(AI_SH)
    }

    fn fill(&self, template: &str) -> String {
        template
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
                    "Note: opencode.json already exists — add \"permission\": {{ \"external_directory\": \"deny\" }} manually, so a write outside the project does not end the session."
                );
            } else {
                // `deny`, not OpenCode's default `ask`: `opencode run` cannot answer, so `ask`
                // rejects a write outside the project and ends the session with no reply;
                // `deny` rejects it and lets the agent go on (lib/ai.sh, ai_opencode).
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

/// Write an executable script.
fn write_script(path: &Path, content: &str) -> Result<(), DecreeError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, content)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

/// Write a built-in machine and its executable scripts (`scripts/<machine>/<name>.sh`)
/// under `decree_dir`, for `backend`.
fn write_machine(
    decree_dir: &Path,
    machine: &BuiltinMachine,
    backend: Backend,
) -> Result<(), DecreeError> {
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
    Ok(())
}

/// Write `LIB_FILES` under `decree_dir`'s `lib/`, for `backend`.
fn write_lib_files(decree_dir: &Path, backend: Backend) -> Result<(), DecreeError> {
    let dir = decree_dir.join(layout::LIB_DIR);
    for (name, template, executable) in LIB_FILES {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if *executable {
            write_script(&path, &backend.fill(template))?;
        } else {
            std::fs::write(&path, backend.fill(template))?;
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
/// `processed.md`, the empty queues, `lib/` with its `README.md`, `ai.sh`, `infra/` and `qa/`, the
/// router machine with its script, the built-in machines with their scripts, and the shared
/// scripts.
/// `graph/` is written by `decree graph`; `schema/` only by `decree schema`, on request.
fn write_layout(decree_dir: &Path, backend: Backend) -> Result<(), DecreeError> {
    for dir in [
        layout::MIGRATIONS_DIR,
        layout::INBOX_DIR,
        layout::RUNS_DIR,
        layout::CRON_DIR,
        MACHINES_DIR,
        SCRIPTS_DIR,
        layout::LIB_DIR,
    ] {
        std::fs::create_dir_all(decree_dir.join(dir))?;
    }
    std::fs::write(decree_dir.join(layout::GITIGNORE_FILE), DECREE_GITIGNORE)?;
    std::fs::write(
        decree_dir.join(layout::LIB_DIR).join("README.md"),
        LIB_README,
    )?;
    std::fs::write(
        decree_dir.join(layout::LIB_DIR).join(LIB_AI_FILE),
        backend.lib_ai_sh(),
    )?;
    std::fs::write(decree_dir.join(layout::PROCESSED_FILE), "")?;
    write_lib_files(decree_dir, backend)?;
    write_router(decree_dir, backend)?;
    for machine in BUILTIN_MACHINES {
        write_machine(decree_dir, machine, backend)?;
    }
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

    #[test]
    fn test_select_backend_flag_wins_over_detection() {
        assert_eq!(select_backend(Some(AiBackend::Claude), |_| true), claude());
        assert_eq!(
            select_backend(Some(AiBackend::Copilot), |_| false).invoke,
            "copilot -p"
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
            assert!(b.router_yml().contains(&format!(
                "script: {{ name: ask_{}, attempts: 2 }}\n",
                b.name
            )));
            assert!(b.router_ask_sh().contains(&format!("reply=$({})\n", b.ask)));
        }
    }

    /// Each built-in machine is written with every script executable and every placeholder
    /// filled; the scripts that ask the AI source `lib/ai.sh` and define no `ai` function.
    #[test]
    fn test_write_machine_writes_the_machine_and_executable_scripts() {
        use std::os::unix::fs::PermissionsExt;
        for ai in [AiBackend::Claude, AiBackend::Opencode, AiBackend::Copilot] {
            let b = backend_entry(ai);
            for machine in BUILTIN_MACHINES {
                let dir = tempfile::TempDir::new().unwrap();
                std::fs::create_dir_all(dir.path().join(MACHINES_DIR)).unwrap();
                write_machine(dir.path(), machine, b).unwrap();
                let yml = std::fs::read_to_string(
                    dir.path().join(format!("machines/{}.yml", machine.name)),
                )
                .unwrap();
                assert!(yml.contains(&format!("name: {}\n", machine.name)));
                assert!(yml.contains(b.title), "{}", b.name);
                assert!(!yml.contains("{ai"), "{yml}");
                for (name, _) in machine.scripts {
                    let path = dir
                        .path()
                        .join(format!("scripts/{}/{name}.sh", machine.name));
                    let text = std::fs::read_to_string(&path).unwrap();
                    assert!(!text.contains("{ai"), "{}: {text}", path.display());
                    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
                    assert_eq!(mode & 0o777, 0o755, "{}", path.display());
                    assert!(!text.contains("ai() {"), "{}", path.display());
                    let calls_ai = text.contains("\nai \"${prompt}\"");
                    assert_eq!(
                        text.contains(". \"${DECREE_LIB}/ai.sh\"\n"),
                        calls_ai,
                        "{}",
                        path.display()
                    );
                    assert_eq!(
                        calls_ai,
                        [
                            "implement",
                            "fix",
                            "verify",
                            "declare",
                            "recon",
                            "attack",
                            "triage"
                        ]
                        .contains(name),
                        "{}",
                        path.display()
                    );
                }
            }
        }
    }

    /// `lib/infra/` declares the backend's CLI, and only `infra/run.sh` is executable.
    #[test]
    fn test_write_lib_files_declares_the_ai_cli() {
        use std::os::unix::fs::PermissionsExt;
        for b in AI_BACKENDS {
            let dir = tempfile::TempDir::new().unwrap();
            write_lib_files(dir.path(), b).unwrap();
            let lib = dir.path().join("lib");
            let requirements = std::fs::read_to_string(lib.join("infra/requirements")).unwrap();
            assert!(
                requirements.ends_with(&format!("\ncommand   {}\ncommand   git\n", b.name)),
                "{requirements}"
            );
            for (name, _, executable) in LIB_FILES {
                let mode = std::fs::metadata(lib.join(name))
                    .unwrap()
                    .permissions()
                    .mode();
                assert_eq!(mode & 0o111 != 0, *executable, "{name}");
            }
        }
    }

    /// `lib/ai.sh` holds `ai` and a function for every backend, defaulting to the one `init`
    /// chose; only Claude's waits out a usage limit, and only OpenCode's nudges.
    #[test]
    fn test_lib_ai_sh_defines_ai_for_each_backend() {
        for b in AI_BACKENDS {
            let text = b.lib_ai_sh();
            assert!(!text.contains("{ai"), "{}: {text}", b.name);
            assert!(
                text.contains(&format!("\nAI_DEFAULT={}\n", b.name)),
                "{}",
                b.name
            );
            for f in ["ai", "ai_claude", "ai_opencode", "ai_copilot", "ai_log"] {
                assert!(text.contains(&format!("\n{f}() {{\n")), "{}: {f}", b.name);
            }
        }
    }

    /// The shared scripts are the three git scripts, executable.
    #[test]
    fn test_write_shared_scripts_writes_the_git_scripts() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        write_shared_scripts(dir.path()).unwrap();
        let mut names: Vec<String> = std::fs::read_dir(dir.path().join(SCRIPTS_DIR))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["clean_tree.sh", "commit.sh", "keep.sh"]);
        for name in names {
            let path = dir.path().join(SCRIPTS_DIR).join(&name);
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755, "{name}");
        }
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

    /// The skill names no `outbox`, `hooks` or `router.md`: decree has none of them.
    #[test]
    fn test_decree_skill_names_no_removed_concept() {
        for (name, content) in DECREE_SKILL {
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

    /// The skill's examples are real files: `examples/project`'s `hello` and cron file, and the
    /// `feature` fixture's verify script.
    #[test]
    fn test_decree_skill_examples_are_the_example_files() {
        let text: String = DECREE_SKILL.iter().map(|(_, c)| *c).collect();
        let hello = example("machines/hello.yml");
        assert!(text.contains(&hello), "hello.yml");
        let verify = feature_fixture("scripts/feature/verify.sh");
        let body = verify
            .lines()
            .filter(|l| !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains(body.trim()), "verify.sh");
        assert!(
            text.contains(&example("cron/nightly-audit.md")),
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
