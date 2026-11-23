//! The official plugins (docs/reference/plugins.md, Official plugins): the repository's
//! `plugins/decree_<name>.md`, built into the binary, so adding one needs no network and its
//! version is this decree's. Their names, and only theirs, start with `decree_`. A project takes
//! one with `decree install <name>`; `decree init` seeds `DEFAULTS`. A project never edits one:
//! it extends one under its own name.

use crate::plugin::Plugin;

/// One official plugin: its name and its file, byte for byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Official {
    pub name: &'static str,
    pub text: &'static str,
}

/// Every official plugin, by name.
pub const OFFICIAL: [Official; 18] = [
    Official {
        name: "decree_ai",
        text: include_str!("../plugins/decree_ai.md"),
    },
    Official {
        name: "decree_comfy",
        text: include_str!("../plugins/decree_comfy.md"),
    },
    Official {
        name: "decree_develop",
        text: include_str!("../plugins/decree_develop.md"),
    },
    Official {
        name: "decree_develop_by_size",
        text: include_str!("../plugins/decree_develop_by_size.md"),
    },
    Official {
        name: "decree_develop_node",
        text: include_str!("../plugins/decree_develop_node.md"),
    },
    Official {
        name: "decree_develop_react",
        text: include_str!("../plugins/decree_develop_react.md"),
    },
    Official {
        name: "decree_develop_rust",
        text: include_str!("../plugins/decree_develop_rust.md"),
    },
    Official {
        name: "decree_gliner_router",
        text: include_str!("../plugins/decree_gliner_router.md"),
    },
    Official {
        name: "decree_infra",
        text: include_str!("../plugins/decree_infra.md"),
    },
    Official {
        name: "decree_lease",
        text: include_str!("../plugins/decree_lease.md"),
    },
    Official {
        name: "decree_llm_router",
        text: include_str!("../plugins/decree_llm_router.md"),
    },
    Official {
        name: "decree_newsletter",
        text: include_str!("../plugins/decree_newsletter.md"),
    },
    Official {
        name: "decree_order",
        text: include_str!("../plugins/decree_order.md"),
    },
    Official {
        name: "decree_prune",
        text: include_str!("../plugins/decree_prune.md"),
    },
    Official {
        name: "decree_qa",
        text: include_str!("../plugins/decree_qa.md"),
    },
    Official {
        name: "decree_registry",
        text: include_str!("../plugins/decree_registry.md"),
    },
    Official {
        name: "decree_router",
        text: include_str!("../plugins/decree_router.md"),
    },
    Official {
        name: "decree_skill",
        text: include_str!("../plugins/decree_skill.md"),
    },
];

/// The official plugin `name`.
/// The official plugins `decree init` adds: a project removes one by deleting its file in
/// `.decree/plugins/` (D86).
pub const DEFAULTS: [&str; 4] = ["decree_ai", "decree_order", "decree_prune", "decree_skill"];

/// The decree skill the official `skill` plugin installs: each file's path under the skill
/// folder (`SKILL.md`, `reference/<file>.md`) and its text.
pub fn skill_files() -> Vec<(String, String)> {
    get("decree_skill")
        .and_then(|o| o.plugin().ok())
        .map(|p| {
            p.scripts
                .iter()
                .filter_map(|(path, text)| {
                    path.strip_prefix("skill/")
                        .map(|rel| (rel.to_string(), text.clone()))
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn get(name: &str) -> Option<Official> {
    OFFICIAL.iter().copied().find(|o| o.name == name)
}

/// Every official plugin's name, in order.
pub fn names() -> Vec<&'static str> {
    OFFICIAL.iter().map(|o| o.name).collect()
}

impl Official {
    /// The plugin, parsed. Every official plugin passes the plugin schema (a test keeps it so),
    /// so a failure here is a build of decree with a broken plugin file.
    pub fn plugin(&self) -> Result<Plugin, String> {
        Plugin::parse(self.text, Some(self.name)).map_err(|problems| {
            let lines: Vec<String> = problems.into_iter().map(|p| p.message).collect();
            format!(
                "official plugin `{}` is broken in this build of decree: {}",
                self.name,
                lines.join("; ")
            )
        })
    }

    /// Its one-line description: the machine's `description`, its base's for one that
    /// extends another without a machine of its own.
    pub fn description(&self) -> String {
        match self.plugin() {
            Ok(p) if p.machine.is_none() => match p.extends.as_deref().and_then(get) {
                Some(base) => base.description(),
                None => p.description().trim().to_string(),
            },
            Ok(p) => p.description().trim().to_string(),
            Err(_) => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::Path;

    /// `OFFICIAL` lists exactly the repository's `plugins/*.md`, by name.
    #[test]
    fn lists_every_plugin_file() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins");
        let mut files: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        files.sort();
        let listed: Vec<String> = names().iter().map(|n| format!("{n}.md")).collect();
        assert_eq!(files, listed);
    }

    /// Every official plugin passes the plugin schema, has a one-line description and a body
    /// that starts with its name, and is exactly what export writes (so exporting an unedited
    /// copy changes nothing).
    #[test]
    fn every_plugin_parses_and_is_canonical() {
        for o in OFFICIAL {
            let plugin = o.plugin().unwrap();
            assert_eq!(plugin.name, o.name);
            let description = o.description();
            assert!(!description.is_empty(), "{}", o.name);
            assert!(!description.contains('\n'), "{}", o.name);
            assert!(
                plugin.body.starts_with(&format!("# {}\n", o.name)),
                "{}",
                o.name
            );
            assert_eq!(
                String::from_utf8(plugin.to_bytes()).unwrap(),
                o.text,
                "{} is not as export writes it",
                o.name
            );
        }
    }

    /// Every `needs` is an official plugin, and no plugin needs itself.
    #[test]
    fn needs_are_official() {
        for o in OFFICIAL {
            for need in o.plugin().unwrap().needs {
                assert_ne!(need, o.name);
                assert!(get(&need).is_some(), "{} needs {need}", o.name);
            }
        }
    }

    /// A file that several plugins carry (`clean_tree.sh`, `commit.sh`, `keep.sh`, `ai.sh`) is
    /// the same in each. The develop variants carry none: they extend `decree_develop`.
    #[test]
    fn shared_files_are_identical() {
        let mut copies: BTreeMap<&str, Vec<(&str, String)>> = BTreeMap::new();
        for o in OFFICIAL {
            let plugin = o.plugin().unwrap();
            for file in ["clean_tree.sh", "commit.sh", "keep.sh", "ai.sh"] {
                if let Some(text) = plugin.scripts.get(file) {
                    copies.entry(file).or_default().push((o.name, text.clone()));
                }
            }
        }
        for (file, list) in &copies {
            let (first, text) = &list[0];
            for (name, other) in &list[1..] {
                assert_eq!(other, text, "{file}: {name} differs from {first}");
            }
        }
        let carriers = |file: &str| -> Vec<&str> { copies[file].iter().map(|(n, _)| *n).collect() };
        let with_infra_qa = ["decree_develop", "decree_infra", "decree_qa"];
        assert_eq!(carriers("commit.sh"), with_infra_qa);
        assert_eq!(carriers("keep.sh"), with_infra_qa);
        assert_eq!(carriers("clean_tree.sh"), with_infra_qa);
        // One `ai.sh`, the `decree_ai` library's; every plugin that asks AI needs it, and
        // sources it through `DECREE_NEED_DECREE_AI`.
        assert_eq!(carriers("ai.sh"), ["decree_ai"]);
        for o in OFFICIAL.iter().filter(|o| o.name != "decree_ai") {
            let plugin = o.plugin().unwrap();
            let sources = plugin.scripts.values().any(|t| {
                t.contains(
                    ". \".decree/machines/${DECREE_NEED_DECREE_AI:-decree_ai}/scripts/ai.sh\"",
                )
            });
            assert_eq!(
                sources,
                plugin.needs.iter().any(|n| n == "decree_ai"),
                "{}",
                o.name
            );
        }
    }

    /// Every official plugin's name has the prefix, and its `extends` names an official one.
    #[test]
    fn every_name_has_the_prefix() {
        for o in OFFICIAL {
            assert!(
                o.name.starts_with(crate::plugin::OFFICIAL_PREFIX),
                "{}",
                o.name
            );
            let plugin = o.plugin().unwrap();
            if let Some(base) = &plugin.extends {
                assert!(get(base).is_some(), "{} extends {base}", o.name);
            }
            assert!(plugin.source.is_none(), "{} has a source", o.name);
        }
    }

    /// The lines of a script that run: not blank, not a comment.
    fn code_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
        text.lines()
            .enumerate()
            .map(|(i, l)| (i + 1, l.trim()))
            .filter(|(_, l)| !l.is_empty() && !l.starts_with('#'))
    }

    /// Whether a plugin file path is a script decree or another script runs: a `.sh` file, or
    /// one that starts with `#!`.
    fn is_code(path: &str, text: &str) -> bool {
        path.ends_with(".sh") || text.starts_with("#!")
    }

    /// An official plugin with a machine finds its own files under
    /// `.decree/machines/${DECREE_MACHINE}/`, and another plugin's through `DECREE_NEED_*`: no
    /// script names a plugin's folder, nor emits to a plugin by name (docs/reference/plugins.md,
    /// Meeting a need with another plugin). So each works in a plugin that extends it.
    #[test]
    fn scripts_reach_plugins_only_through_the_variables() {
        let names = names();
        for o in OFFICIAL {
            let plugin = o.plugin().unwrap();
            if plugin.machine.is_none() && plugin.extends.is_none() {
                continue;
            }
            for (path, text) in plugin.scripts.iter().filter(|(p, t)| is_code(p, t)) {
                for (n, line) in code_lines(text) {
                    for other in &names {
                        let folder = format!(".decree/machines/{other}/");
                        assert!(
                            !line.contains(&folder),
                            "{}: scripts/{path}: line {n} names {folder}: {line}",
                            o.name
                        );
                        for emit in [format!("--machine {other}"), format!("emit {other}")] {
                            assert!(
                                !line.contains(&emit),
                                "{}: scripts/{path}: line {n} emits to {other} by name: {line}",
                                o.name
                            );
                        }
                    }
                }
            }
        }
    }

    /// Whether a script line runs or sources a path under `state/` (the heuristic `decree check`
    /// warns with).
    fn runs_state(line: &str) -> bool {
        crate::commands::check::runs_state_path(line)
    }

    /// No official plugin seeds code in `state/` (a file with `#!`), or runs or sources a path
    /// under it: `state/` holds data (docs/reference/scripts.md, State).
    #[test]
    fn no_code_in_state() {
        for o in OFFICIAL {
            let plugin = o.plugin().unwrap();
            for (path, text) in &plugin.state {
                assert!(!text.starts_with("#!"), "{}: state/{path} has #!", o.name);
            }
            for (path, text) in plugin.scripts.iter().filter(|(p, t)| is_code(p, t)) {
                for (n, line) in code_lines(text) {
                    assert!(
                        !runs_state(line),
                        "{}: scripts/{path}: line {n} runs a path under state/: {line}",
                        o.name
                    );
                }
            }
        }
    }

    /// The develop variants extend `decree_develop` and hold only their gate, conventions and
    /// requirements, with no machine: no copies of `decree_develop`'s scripts ship.
    #[test]
    fn the_develop_variants_extend_decree_develop() {
        for name in [
            "decree_develop_rust",
            "decree_develop_node",
            "decree_develop_react",
        ] {
            let plugin = get(name).unwrap().plugin().unwrap();
            assert_eq!(plugin.extends.as_deref(), Some("decree_develop"), "{name}");
            assert!(plugin.machine.is_none(), "{name}");
            assert!(plugin.needs.is_empty(), "{name}");
            assert_eq!(
                plugin.scripts.keys().collect::<Vec<_>>(),
                ["conventions.md", "gate.sh", "requirements"],
                "{name}"
            );
        }
    }

    /// No official plugin names the backend `decree init --ai` used to bake into installed
    /// files: no `{ai}` placeholder is left.
    #[test]
    fn no_placeholder_is_left() {
        for o in OFFICIAL {
            for p in ["{ai}", "{ai_title}", "{ai_cli}", "{ai_call}"] {
                assert!(!o.text.contains(p), "{} has {p}", o.name);
            }
        }
    }
}
