//! The official plugins (docs/reference/plugins.md, Official plugins): the repository's
//! `plugins/<name>.md`, built into the binary, so adding one needs no network and its version
//! is this decree's. A project takes one with `decree plugin add <name>`; `decree init` adds none.

use crate::plugin::Plugin;

/// One official plugin: its name and its file, byte for byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Official {
    pub name: &'static str,
    pub text: &'static str,
}

/// Every official plugin, by name.
pub const OFFICIAL: [Official; 17] = [
    Official {
        name: "ai",
        text: include_str!("../plugins/ai.md"),
    },
    Official {
        name: "comfy",
        text: include_str!("../plugins/comfy.md"),
    },
    Official {
        name: "develop",
        text: include_str!("../plugins/develop.md"),
    },
    Official {
        name: "develop_by_size",
        text: include_str!("../plugins/develop_by_size.md"),
    },
    Official {
        name: "develop_node",
        text: include_str!("../plugins/develop_node.md"),
    },
    Official {
        name: "develop_react",
        text: include_str!("../plugins/develop_react.md"),
    },
    Official {
        name: "develop_rust",
        text: include_str!("../plugins/develop_rust.md"),
    },
    Official {
        name: "gliner_router",
        text: include_str!("../plugins/gliner_router.md"),
    },
    Official {
        name: "infra",
        text: include_str!("../plugins/infra.md"),
    },
    Official {
        name: "lease",
        text: include_str!("../plugins/lease.md"),
    },
    Official {
        name: "llm_router",
        text: include_str!("../plugins/llm_router.md"),
    },
    Official {
        name: "newsletter",
        text: include_str!("../plugins/newsletter.md"),
    },
    Official {
        name: "order",
        text: include_str!("../plugins/order.md"),
    },
    Official {
        name: "plugin_install",
        text: include_str!("../plugins/plugin_install.md"),
    },
    Official {
        name: "prune",
        text: include_str!("../plugins/prune.md"),
    },
    Official {
        name: "qa",
        text: include_str!("../plugins/qa.md"),
    },
    Official {
        name: "router",
        text: include_str!("../plugins/router.md"),
    },
];

/// The official plugin `name`.
/// The official plugins `decree init` adds: a project removes one by deleting its file in
/// `.decree/plugins/` (D86).
pub const DEFAULTS: [&str; 2] = ["order", "prune"];

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

    /// Its one-line description: the machine's `description`.
    pub fn description(&self) -> String {
        self.plugin()
            .map(|p| p.description().trim().to_string())
            .unwrap_or_default()
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
    /// the same in each.
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
        let develops = ["develop", "develop_node", "develop_react", "develop_rust"];
        let with_infra_qa = [&develops[..], &["infra", "qa"]].concat();
        assert_eq!(carriers("commit.sh"), with_infra_qa);
        assert_eq!(carriers("keep.sh"), with_infra_qa);
        assert_eq!(carriers("clean_tree.sh"), with_infra_qa);
        // One `ai.sh`, the `ai` library's; every plugin that asks AI needs it.
        assert_eq!(carriers("ai.sh"), ["ai"]);
        for o in OFFICIAL.iter().filter(|o| o.name != "ai") {
            let plugin = o.plugin().unwrap();
            let sources = plugin
                .scripts
                .values()
                .any(|t| t.contains(". .decree/machines/ai/scripts/ai.sh"));
            assert_eq!(
                sources,
                plugin.needs.iter().any(|n| n == "ai"),
                "{}",
                o.name
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
