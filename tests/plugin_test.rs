//! Plugins (docs/reference/plugins.md): installed only when asked, export from the installed
//! copy, import, and an install that never refuses over a change but moves what it would
//! lose to `.decree/archive/` first. One test per
//! acceptance criterion of migration A09 and decision D85, each in its own temp project.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[path = "common/schema.rs"]
mod schema;

/// `develop`: runs its own script, which reads a data file beside it, then a flat script,
/// then the `helper` machine.
const DEVELOP: &str = "\
---
name: develop
needs:
- helper
machine:
  description: Implements, then tidies, then asks the helper.
  initial: implement
  states:
    implement:
      invoke: implement
      transitions:
        done: tidy
    tidy:
      invoke: tidy
      transitions:
        done: help
    help:
      invoke:
        machine:
          name: helper
      transitions:
        done: done
    done:
      final: true
    failed:
      final: true
scripts:
  data.txt: |
    from the data file
  implement.sh: |
    #!/usr/bin/env bash
    echo \"implement: $(cat .decree/machines/develop/scripts/data.txt)\"
  tidy: |
    #!/usr/bin/env bash
    echo tidy
---
# develop

Implements, then tidies, then asks the helper.
";

const HELPER: &str = "\
---
name: helper
machine:
  description: Helps.
  initial: help
  states:
    help:
      invoke: help
      transitions:
        done: done
    done:
      final: true
    failed:
      final: true
scripts:
  help: |
    #!/usr/bin/env bash
    echo helped
---
# helper

Helps.
";

/// The line `decree install` prints for an archived plugin file, `path` relative to `.decree/`.
fn archived_line(path: &str) -> String {
    format!(
        "archived what installing `helper` replaced as .decree/{path}; `decree install .decree/{path}` restores it"
    )
}

const MESSAGE: &str = "---\nmachine: develop\n---\nDo the thing.\n";

struct Project {
    _tmp: TempDir,
    root: PathBuf,
}

impl Project {
    fn new() -> Project {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        fs::create_dir_all(root.join(".decree")).unwrap();
        Project { _tmp: tmp, root }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(".decree").join(rel)
    }

    fn put(&self, rel: &str, text: &str) {
        let path = self.path(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
    }

    fn put_exec(&self, rel: &str, text: &str) {
        self.put(rel, text);
        fs::set_permissions(self.path(rel), fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn get(&self, rel: &str) -> String {
        fs::read_to_string(self.path(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
    }

    fn exists(&self, rel: &str) -> bool {
        self.path(rel).exists()
    }

    /// The plugin file archived at `path`, relative to `.decree/`, as JSON.
    fn archived(&self, path: &str) -> serde_json::Value {
        assert!(path.ends_with(".md"), "{path}");
        schema::frontmatter_to_json(&self.get(path)).unwrap()
    }

    /// What is archived for plugin `name`, relative to `.decree/`, oldest first.
    fn archives(&self, name: &str) -> Vec<String> {
        let mut out: Vec<String> = fs::read_dir(self.path(&format!("archive/{name}")))
            .map(|entries| {
                entries
                    .map(|e| {
                        format!(
                            "archive/{name}/{}",
                            e.unwrap().file_name().to_string_lossy()
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort();
        out
    }

    /// (exit code, stdout, stderr).
    fn decree(&self, args: &[&str], stdin: &str) -> (i32, String, String) {
        let out = common::decree_at(&self.root)
            .env("NO_COLOR", "1")
            .args(args)
            .write_stdin(stdin)
            .output()
            .unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    }

    /// Every file under `.decree/<dir>`, relative to `.decree/`, with its bytes.
    fn tree(&self, dir: &str) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let base = self.path("");
        let mut stack = vec![self.path(dir)];
        while let Some(d) = stack.pop() {
            let Ok(entries) = fs::read_dir(&d) else {
                continue;
            };
            for e in entries {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    let rel = p
                        .strip_prefix(&base)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned();
                    out.push((rel, fs::read(&p).unwrap()));
                }
            }
        }
        out.sort();
        out
    }

    /// The logs of the one run, by name, with their text.
    fn run_logs(&self) -> Vec<(String, String)> {
        let runs: Vec<PathBuf> = fs::read_dir(self.path("runs"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        let mut out = Vec::new();
        for run in runs {
            for e in fs::read_dir(&run).unwrap() {
                let p = e.unwrap().path();
                let name = p.file_name().unwrap().to_string_lossy().into_owned();
                if name.ends_with(".log") {
                    out.push((name, fs::read_to_string(&p).unwrap()));
                }
            }
        }
        out.sort();
        out
    }
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// The machine of a plugin file with `name` first, as hand-written YAML.
fn machine_yaml(plugin: &str, name: &str) -> String {
    let yaml = plugin.split("---\n").nth(1).unwrap();
    let value: serde_norway::Value = serde_norway::from_str(yaml).unwrap();
    let mut machine = serde_norway::Mapping::new();
    machine.insert("name".into(), name.into());
    for (k, v) in value["machine"].as_mapping().unwrap() {
        machine.insert(k.clone(), v.clone());
    }
    serde_norway::to_string(&machine).unwrap()
}

/// `process` installs nothing; once `decree install` has, a project of only plugins
/// and a message runs as the same files written by hand do.
#[test]
fn installed_plugins_run_as_if_written_by_hand() {
    let p = Project::new();
    p.put("plugins/develop.md", DEVELOP);
    p.put("plugins/helper.md", HELPER);
    let (code, _, err) = p.decree(&["daemon", "--once"], "");
    assert_eq!(code, 0, "{err}");
    assert!(
        err.contains("plugins/develop.md: not installed, or changed since it was installed: run `decree install`"),
        "{err}"
    );
    assert!(!p.exists("machines/develop/machine.yml") && !p.exists("scripts/develop"));
    let (code, out, err) = p.decree(&["install"], "");
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out,
        "installed .decree/plugins/develop.md\ninstalled .decree/plugins/helper.md\n"
    );
    p.put("inbox/01-a.md", MESSAGE);
    let (code, out, err) = p.decree(&["daemon", "--once"], "");
    assert_eq!(code, 0, "{out}{err}");
    assert!(p.exists("machines/develop/machine.yml") && p.exists("machines/helper/machine.yml"));
    assert_eq!(
        p.get("machines/develop/scripts/data.txt"),
        "from the data file\n"
    );
    assert_eq!(
        mode(&p.path("machines/develop/scripts/implement.sh")),
        0o755
    );
    assert_eq!(mode(&p.path("machines/develop/scripts/data.txt")), 0o644);
    assert!(out.contains("implement: from the data file"), "{out}");
    let with_plugins = p.run_logs();
    assert!(
        with_plugins.iter().any(|(_, log)| log.contains("helped")),
        "{with_plugins:?}"
    );

    let hand = Project::new();
    hand.put(
        "machines/develop/machine.yml",
        &machine_yaml(DEVELOP, "develop"),
    );
    hand.put(
        "machines/helper/machine.yml",
        &machine_yaml(HELPER, "helper"),
    );
    hand.put("machines/develop/scripts/data.txt", "from the data file\n");
    hand.put_exec(
        "machines/develop/scripts/implement.sh",
        "#!/usr/bin/env bash\necho \"implement: $(cat .decree/machines/develop/scripts/data.txt)\"\n",
    );
    hand.put_exec(
        "machines/develop/scripts/tidy",
        "#!/usr/bin/env bash\necho tidy\n",
    );
    hand.put_exec(
        "machines/helper/scripts/help",
        "#!/usr/bin/env bash\necho helped\n",
    );
    hand.put("inbox/01-a.md", MESSAGE);
    let (code, out, err) = hand.decree(&["daemon", "--once"], "");
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(with_plugins, hand.run_logs());
}

/// Export of a hand-written machine holds its folder's scripts and a data file; once the
/// installed files are deleted, `decree install` installs them again with the same
/// bytes, and the run resolves the same scripts.
#[test]
fn export_takes_the_folder_and_installs_it_again() {
    let p = Project::new();
    p.put(
        "machines/hello/machine.yml",
        "\
# Written by hand.
name: hello
description: Greets, then tidies.
initial: greet
states:
  greet:
    invoke: greet
    transitions: { done: tidy }
  tidy:
    invoke: tidy
    transitions: { done: done }
  done: { final: true }
  failed: { final: true }
",
    );
    p.put_exec(
        "machines/hello/scripts/greet.sh",
        "#!/usr/bin/env bash\necho \"hi $(cat .decree/machines/hello/scripts/name.txt)\"\n",
    );
    p.put("machines/hello/scripts/name.txt", "world\n");
    p.put_exec(
        "machines/hello/scripts/tidy",
        "#!/usr/bin/env bash\necho tidy\n",
    );

    let (code, out, err) = p.decree(&["export", "hello"], "");
    assert_eq!(code, 0, "{err}");
    assert!(out.starts_with("wrote .decree/plugins/hello.md\n"), "{out}");
    let plugin = p.get("plugins/hello.md");
    for needle in [
        "greet.sh: |",
        "name.txt: |",
        "tidy: |",
        "echo tidy",
        "# hello\n\nGreets, then tidies.\n",
    ] {
        assert!(plugin.contains(needle), "{needle}: {plugin}");
    }
    let scripts = [
        (
            "machines/hello/scripts/greet.sh",
            p.get("machines/hello/scripts/greet.sh"),
        ),
        (
            "machines/hello/scripts/name.txt",
            p.get("machines/hello/scripts/name.txt"),
        ),
        (
            "machines/hello/scripts/tidy",
            p.get("machines/hello/scripts/tidy"),
        ),
    ];

    fs::remove_file(p.path("machines/hello/machine.yml")).unwrap();
    fs::remove_dir_all(p.path("machines/hello/scripts")).unwrap();
    let (code, out, err) = p.decree(&["install"], "");
    assert_eq!(
        (code, out.as_str()),
        (0, "installed .decree/plugins/hello.md\n"),
        "{err}"
    );
    for (rel, text) in &scripts {
        assert_eq!(&p.get(rel), text, "{rel}");
    }
    assert_eq!(mode(&p.path("machines/hello/scripts/tidy")), 0o755);

    p.put("inbox/01-a.md", "---\nmachine: hello\n---\nhi\n");
    let (code, out, err) = p.decree(&["daemon", "--once"], "");
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("hi world") && out.contains("tidy"), "{out}");
    // The run resolved the script in the machine's folder.
    let events = fs::read_dir(p.path("runs"))
        .unwrap()
        .map(|e| fs::read_to_string(e.unwrap().path().join("events.jsonl")).unwrap())
        .collect::<String>();
    assert!(events.contains("machines/hello/scripts/tidy"), "{events}");
}

/// Exporting an installed plugin twice gives the file it was installed from, byte for byte.
#[test]
fn export_of_an_installed_plugin_is_byte_identical() {
    let p = Project::new();
    p.put("plugins/develop.md", DEVELOP);
    p.put("plugins/helper.md", HELPER);
    let (code, _, err) = p.decree(&["install"], "");
    assert_eq!(code, 0, "{err}");
    for _ in 0..2 {
        let (code, _, err) = p.decree(&["export", "develop"], "");
        assert_eq!(code, 0, "{err}");
        assert_eq!(p.get("plugins/develop.md"), DEVELOP);
    }
    let (code, _, _) = p.decree(&["export", "helper"], "");
    assert_eq!(code, 0);
    assert_eq!(p.get("plugins/helper.md"), HELPER);
}

/// An edit to an installed script runs, and stays, while the plugin is unchanged.
#[test]
fn an_edited_script_runs_and_is_not_overwritten() {
    let p = Project::new();
    p.put("plugins/helper.md", HELPER);
    assert_eq!(p.decree(&["install"], "").0, 0);
    let edited = "#!/usr/bin/env bash\necho edited in place\n";
    p.put_exec("machines/helper/scripts/help", edited);
    p.put("inbox/01-a.md", "---\nmachine: helper\n---\nhi\n");
    let (code, out, err) = p.decree(&["daemon", "--once"], "");
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("edited in place"), "{out}");
    assert_eq!(p.get("machines/helper/scripts/help"), edited);
}

/// Set up the conflict: an installed copy edited in place, and a changed plugin file.
fn conflict() -> (Project, String) {
    let p = Project::new();
    p.put("plugins/helper.md", HELPER);
    assert_eq!(p.decree(&["install"], "").0, 0);
    let edited = "#!/usr/bin/env bash\necho edited in place\n";
    p.put_exec("machines/helper/scripts/help", edited);
    p.put(
        "plugins/helper.md",
        &HELPER.replace("echo helped", "echo helped twice"),
    );
    (p, edited.to_string())
}

/// A plugin changed over an edit in place: `check` and `daemon` warn and run the edit;
/// `decree install` archives the edit and installs the plugin, exiting 0.
#[test]
fn a_changed_plugin_over_an_edit_archives_the_edit_and_installs() {
    let (p, edited) = conflict();
    let (code, out, err) = p.decree(&["check"], "");
    assert_eq!((code, out.as_str()), (0, ""), "{err}");
    assert!(
        err.contains("plugins/helper.md: its installed copy (machines/helper/machine.yml, or a file in machines/helper/scripts/ or machines/helper/cron/) was edited in place"),
        "{err}"
    );
    assert!(
        err.contains("`decree export helper` keeps the edit"),
        "{err}"
    );
    assert_eq!(err.matches("plugins/helper.md").count(), 1, "{err}");
    assert_eq!(p.get("machines/helper/scripts/help"), edited);

    // `daemon` keeps running the edited copy and says so once.
    p.put("inbox/01-a.md", "---\nmachine: helper\n---\nhi\n");
    p.put("inbox/a.md", "---\nmachine: helper\n---\nhi\n");
    let (code, out, err) = p.decree(&["daemon", "--once"], "");
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("edited in place"), "{out}");
    assert_eq!(err.matches("was edited in place").count(), 1, "{err}");
    assert_eq!(p.get("machines/helper/scripts/help"), edited);

    // `decree install` with no name moves the edit to the archive and installs the plugin.
    let (code, out, err) = p.decree(&["install"], "");
    assert_eq!(code, 0, "{err}");
    let [dir] = p.archives("helper").try_into().unwrap();
    assert_eq!(
        out,
        format!(
            "{}\ninstalled .decree/plugins/helper.md\n",
            archived_line(&dir)
        )
    );
    assert_eq!(p.archived(&dir)["scripts"]["help"], edited);
    assert_eq!(p.archived(&dir)["machine"]["description"], "Helps.");
    assert_eq!(
        p.archives("helper"),
        std::slice::from_ref(&dir),
        "no `.replaced.md`"
    );
    assert_eq!(p.get("archive/.gitignore").lines().last(), Some("*"));
    assert_eq!(
        p.get("machines/helper/scripts/help"),
        "#!/usr/bin/env bash\necho helped twice\n"
    );
    assert!(p.exists("schema/v1/machine.schema.json"));
    let (code, out, err) = p.decree(&["check"], "");
    assert_eq!((code, out.as_str()), (0, ""), "{err}");
    assert!(!err.contains("edited in place"), "{err}");

    // Installed as its file says, it installs again and archives nothing.
    let (code, out, _) = p.decree(&["install"], "");
    assert_eq!(
        (code, out.as_str()),
        (0, "installed .decree/plugins/helper.md\n")
    );
    assert_eq!(p.archives("helper").len(), 1);
}

/// An edit with the plugin unchanged is archived too: an install always installs.
#[test]
fn an_edit_under_an_unchanged_plugin_is_archived() {
    let p = Project::new();
    p.put("plugins/helper.md", HELPER);
    assert_eq!(p.decree(&["install"], "").0, 0);
    p.put_exec(
        "machines/helper/scripts/help",
        "#!/usr/bin/env bash\necho mine\n",
    );
    let (code, _, err) = p.decree(&["install"], "");
    assert_eq!(code, 0, "{err}");
    let [dir] = p.archives("helper").try_into().unwrap();
    assert_eq!(
        p.archived(&dir)["scripts"]["help"],
        "#!/usr/bin/env bash\necho mine\n"
    );
    assert_eq!(
        p.get("machines/helper/scripts/help"),
        "#!/usr/bin/env bash\necho helped\n"
    );
}

#[test]
fn plugin_install_takes_the_plugin_and_archives_added_files() {
    let (p, edited) = conflict();
    p.put("machines/helper/scripts/added.txt", "added in place\n");
    let (code, out, err) = p.decree(&["install", "helper"], "");
    assert_eq!(code, 0, "{err}");
    let [dir] = p.archives("helper").try_into().unwrap();
    assert_eq!(out, format!("{}\ninstalled helper\n", archived_line(&dir)));
    assert_eq!(p.archived(&dir)["scripts"]["help"], edited);
    assert_eq!(p.archived(&dir)["scripts"]["added.txt"], "added in place\n");
    assert_eq!(
        p.get("machines/helper/scripts/help"),
        "#!/usr/bin/env bash\necho helped twice\n"
    );
    assert!(!p.exists("machines/helper/scripts/added.txt"));
    let (code, out, err) = p.decree(&["check"], "");
    assert_eq!((code, out.as_str()), (0, ""), "{err}");
}

/// A machine written by hand where a plugin installs is archived, not a refusal.
#[test]
fn installing_over_a_hand_written_machine_archives_it() {
    let p = Project::new();
    let by_hand = "name: helper\ndescription: By hand.\ninitial: work\nstates:\n  work:\n    invoke: work\n    transitions: { done: done }\n  done: { final: true }\n  failed: { final: true }\n";
    p.put("machines/helper/machine.yml", by_hand);
    let (code, out, err) = p.decree(&["install", "-"], HELPER);
    assert_eq!(code, 0, "{err}");
    let [dir] = p.archives("helper").try_into().unwrap();
    assert!(out.contains(&archived_line(&dir)), "{out}");
    assert_eq!(p.archived(&dir)["machine"]["description"], "By hand.");
    assert!(p.get("machines/helper/machine.yml").contains("Helps."));
}

/// The archive is the copy exported as one plugin file, its cron files with it, and
/// `decree install <that file>` restores it, archiving what it replaces in turn.
#[test]
fn an_archived_copy_is_a_plugin_file_that_install_restores() {
    let p = Project::new();
    p.put("plugins/helper.md", HELPER);
    assert_eq!(p.decree(&["install"], "").0, 0);
    let cron = "---\ncron: 0 9 * * *\nmachine: helper\n---\nEach morning.\n";
    p.put("machines/helper/cron/morning.md", cron);
    p.put_exec(
        "machines/helper/scripts/help",
        "#!/usr/bin/env bash\necho mine\n",
    );
    assert_eq!(p.decree(&["install"], "").0, 0);
    assert!(!p.exists("machines/helper/cron/morning.md"));
    let [dir] = p.archives("helper").try_into().unwrap();
    let plugin = p.archived(&dir);
    assert_eq!(plugin["name"], "helper");
    assert_eq!(plugin["cron"]["morning.md"], cron);
    assert_eq!(
        plugin["scripts"]["help"],
        "#!/usr/bin/env bash\necho mine\n"
    );
    assert!(
        p.get(&dir).ends_with("# helper\n\nHelps.\n"),
        "the body is kept"
    );

    let file = p.path(&dir);
    let (code, out, err) = p.decree(&["install", file.to_str().unwrap()], "");
    assert_eq!(code, 0, "{err}");
    assert_eq!(p.get("machines/helper/cron/morning.md"), cron);
    assert_eq!(
        p.get("machines/helper/scripts/help"),
        "#!/usr/bin/env bash\necho mine\n"
    );
    assert_eq!(p.get("plugins/helper.md"), p.get(&dir));
    // The plugin file it replaced is archived in turn.
    let archives = p.archives("helper");
    assert_eq!(archives.len(), 2, "{archives:?}");
    assert!(out.contains(&archived_line(&archives[1])), "{out}");
    assert_eq!(p.get(&archives[1]), HELPER);
}

/// A copy that cannot be a plugin file, its machine.yml not YAML, is archived as it was,
/// as a folder.
#[test]
fn a_copy_that_cannot_be_a_plugin_is_archived_as_a_folder() {
    let p = Project::new();
    p.put("plugins/helper.md", HELPER);
    assert_eq!(p.decree(&["install"], "").0, 0);
    p.put("machines/helper/machine.yml", "states: [unclosed\n");
    let (code, out, err) = p.decree(&["install"], "");
    assert_eq!(code, 0, "{err}");
    let [dir] = p.archives("helper").try_into().unwrap();
    assert!(!dir.ends_with(".md"), "{dir}");
    assert!(
        out.contains(&format!("to .decree/{dir}/ (it cannot be a plugin file")),
        "{out}"
    );
    assert_eq!(p.get(&format!("{dir}/machine.yml")), "states: [unclosed\n");
    assert_eq!(
        p.get(&format!("{dir}/scripts/help")),
        "#!/usr/bin/env bash\necho helped\n"
    );
    assert!(p.get("machines/helper/machine.yml").contains("Helps."));
}

#[test]
fn a_deleted_plugin_removes_its_copy_and_keeps_hand_written_machines() {
    let p = Project::new();
    p.put("plugins/helper.md", HELPER);
    p.put(
        "machines/extra/machine.yml",
        "name: extra\ndescription: By hand.\ninitial: work\nstates:\n  work:\n    invoke: work\n    transitions: { done: done }\n  done: { final: true }\n  failed: { final: true }\n",
    );
    p.put_exec("scripts/work", "#!/usr/bin/env bash\n");
    assert_eq!(p.decree(&["install"], "").0, 0);
    assert!(p.exists("machines/helper/machine.yml"));
    fs::remove_file(p.path("plugins/helper.md")).unwrap();
    // `check` warns and removes nothing; `decree install` removes it.
    let (code, _, err) = p.decree(&["check"], "");
    assert_eq!(code, 0, "{err}");
    assert!(
        err.contains("plugins/helper.md: deleted, but its installed copy is still there"),
        "{err}"
    );
    assert!(p.exists("machines/helper/machine.yml"));
    let (code, out, err) = p.decree(&["install"], "");
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("removed the installed copy of .decree/plugins/helper.md"),
        "{out}"
    );
    assert!(!p.exists("machines/helper/machine.yml"));
    assert!(!p.exists("scripts/helper"));
    assert!(p.exists("machines/extra/machine.yml") && p.exists("scripts/work"));
}

/// `decree install -` copies the plugin on stdin into `plugins/` and installs it
/// (`curl -fsSL <url> | decree install -`); with no argument `decree install` never reads
/// stdin, even when something is piped (a loop's input stays the loop's), and installs every
/// plugin as its file says; `--official` needs a name.
#[test]
fn install_reads_stdin_only_with_a_dash() {
    let p = Project::new();
    let (code, out, err) = p.decree(&["install"], HELPER);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "no plugins in .decree/plugins/: nothing to install\n");
    assert!(!p.exists("plugins/helper.md"));
    let (code, out, err) = p.decree(&["install", "-"], HELPER);
    assert_eq!(code, 0, "{err}");
    assert!(
        out.ends_with("installed .decree/plugins/helper.md\n"),
        "{out}"
    );
    assert_eq!(p.get("plugins/helper.md"), HELPER);
    assert!(p.exists("machines/helper/machine.yml"));

    let (code, out, err) = p.decree(&["install", "-"], DEVELOP);
    assert_eq!(code, 0, "{err}");
    assert!(
        out.ends_with("installed .decree/plugins/develop.md\n"),
        "{out}"
    );

    let (code, out, err) = p.decree(&["install"], "");
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out,
        "installed .decree/plugins/develop.md\ninstalled .decree/plugins/helper.md\n"
    );
    let (code, _, err) = p.decree(&["install", "--official"], "");
    assert_eq!(code, 1);
    assert!(
        err.contains("--official takes this decree's copy of the plugins named"),
        "{err}"
    );
}

#[test]
fn import_refuses_missing_needs_and_writes_nothing() {
    let p = Project::new();
    let before = p.tree("");
    let (code, out, err) = p.decree(&["install", "-"], DEVELOP);
    assert_eq!(code, 1);
    assert_eq!(out, "");
    assert!(err.contains("`helper`"), "{err}");
    assert_eq!(p.tree(""), before);
}

/// A plugin whose name `plugins/` has is installed over it, the old file archived;
/// `--official` is for names only.
#[test]
fn import_replaces_a_taken_name_archiving_the_old_file() {
    let p = Project::new();
    p.put("plugins/helper.md", HELPER);
    assert_eq!(p.decree(&["install"], "").0, 0);
    let changed = HELPER.replace("echo helped", "echo helped twice");

    let before = p.tree("");
    let (code, _, err) = p.decree(&["install", "-", "--official"], &changed);
    assert_eq!(code, 1);
    assert!(
        err.contains("--official takes this decree's copy of the plugins named"),
        "{err}"
    );
    assert_eq!(p.tree(""), before);

    let (code, out, err) = p.decree(&["install", "-"], &changed);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("plugin `helper`: Helps."), "{out}");
    assert!(
        out.contains(".decree/machines/helper/scripts/help (2 lines)"),
        "{out}"
    );
    assert!(out.contains("needs: none"), "{out}");
    assert!(
        out.contains("changed .decree/machines/helper/scripts/help"),
        "{out}"
    );
    let [dir] = p.archives("helper").try_into().unwrap();
    assert!(out.contains(&archived_line(&dir)), "{out}");
    assert_eq!(
        p.get(&dir),
        HELPER,
        "the copy was unedited: the old file alone"
    );
    assert_eq!(p.get("plugins/helper.md"), changed);
    assert_eq!(
        p.get("machines/helper/scripts/help"),
        "#!/usr/bin/env bash\necho helped twice\n"
    );

    // The same file again replaces nothing worth keeping.
    assert_eq!(p.decree(&["install", "-"], &changed).0, 0);
    assert_eq!(p.archives("helper").len(), 1);
}

/// `--official` takes decree's built-in copy, so a project's own plugin has none to take:
/// it exits 1, writing nothing.
#[test]
fn official_refuses_a_plugin_decree_does_not_ship() {
    let p = Project::new();
    p.put("plugins/helper.md", HELPER);
    let before = p.tree("");
    let (code, out, err) = p.decree(&["install", "--official", "helper"], "");
    assert_eq!((code, out.as_str()), (1, ""));
    assert!(
        err.contains("--official: `helper` is not an official plugin"),
        "{err}"
    );
    assert_eq!(p.tree(""), before);
}

#[test]
fn import_from_a_file_prints_what_it_runs_and_installs_it() {
    let p = Project::new();
    p.put("plugins/helper.md", HELPER);
    let file = p.root.join("develop-download.md");
    fs::write(&file, DEVELOP).unwrap();
    let (code, out, err) = p.decree(&["install", file.to_str().unwrap()], "");
    assert_eq!(code, 0, "{err}");
    assert!(
        out.starts_with("plugin `develop`: Implements, then tidies, then asks the helper.\n"),
        "{out}"
    );
    assert!(out.contains("needs: helper"), "{out}");
    assert!(
        out.ends_with("installed .decree/plugins/develop.md\n"),
        "{out}"
    );
    assert_eq!(p.get("plugins/develop.md"), DEVELOP);
    assert!(p.exists("machines/develop/scripts/implement.sh"));
}

#[test]
fn a_script_path_that_leaves_the_folder_fails_check() {
    let p = Project::new();
    p.put(
        "plugins/bad.md",
        "---\nname: bad\nmachine: { description: d }\nscripts:\n  ../x.sh: hi\n---\n",
    );
    let (code, out, _) = p.decree(&["check"], "");
    assert_eq!(code, 1);
    assert!(out.contains("`../x.sh`") && out.contains("(P1)"), "{out}");
    assert!(!p.exists("scripts/x.sh") && !p.exists("machines/bad/machine.yml"));
    // import refuses it the same way.
    let (code, _, err) = p.decree(&["install", "-"], &p.get("plugins/bad.md"));
    assert_eq!(code, 1);
    assert!(err.contains("`../x.sh`"), "{err}");
}

#[test]
fn a_hand_written_machine_beside_plugins_is_a_warning() {
    let p = Project::new();
    p.put("plugins/helper.md", HELPER);
    p.put(
        "machines/extra/machine.yml",
        "name: extra\ndescription: By hand.\ninitial: work\nstates:\n  work:\n    invoke: work\n    transitions: { done: done }\n  done: { final: true }\n  failed: { final: true }\n",
    );
    p.put_exec("scripts/work", "#!/usr/bin/env bash\n");
    let (code, out, err) = p.decree(&["check"], "");
    assert_eq!((code, out.as_str()), (0, ""), "{err}");
    assert!(err.contains("machines/extra/machine.yml: no plugin installed this machine; run `decree export extra`"), "{err}");
    assert!(!err.contains("machines/helper/machine.yml"), "{err}");
}

#[test]
fn json_output_follows_the_cli_schema() {
    let validator = schema::validator(schema::CLI_PLUGIN_SCHEMA);
    let p = Project::new();
    p.put("plugins/helper.md", HELPER);
    let mut docs = Vec::new();
    for args in [
        vec!["install", "--format", "json"],
        vec!["export", "helper", "--format", "json"],
    ] {
        let (code, out, err) = p.decree(&args, "");
        assert_eq!(code, 0, "{err}");
        docs.push(out);
    }
    let changed = HELPER.replace("echo helped", "echo helped twice");
    let (code, out, err) = p.decree(&["install", "-", "--format", "json"], &changed);
    assert_eq!(code, 0, "{err}");
    docs.push(out);
    for doc in &docs {
        let json: serde_json::Value = serde_json::from_str(doc).unwrap();
        let errors = schema::errors(&validator, &json);
        assert!(errors.is_empty(), "{doc}: {errors:?}");
    }
    let import: serde_json::Value = serde_json::from_str(&docs[2]).unwrap();
    assert_eq!(import["replaced"], true);
    assert_eq!(
        import["changes"],
        serde_json::json!([{ "path": ".decree/machines/helper/scripts/help", "change": "changed" }])
    );
}

/// Every plugin file a test writes validates against `plugin.schema.json`.
#[test]
fn the_test_plugins_validate() {
    let validator = schema::plugin_validator();
    for text in [DEVELOP, HELPER, KIT] {
        let json = schema::frontmatter_to_json(text).unwrap();
        assert!(schema::errors(&validator, &json).is_empty(), "{text}");
    }
}

/// A plugin with no machine, a cron file, a state seed, an order seed and an `x-` key.
const KIT: &str = "\
---
name: kit
needs:
- helper
scripts:
  lib.sh: |
    kit() { echo kit; }
cron:
  daily.md: |
    ---
    cron: 0 9 * * *
    machine: helper
    ---
    Daily.
state:
  seeds.txt: |
    a
order:
  A01-kit.md: |
    ---
    machine: helper
    ---
    Set up.
x-exist-services:
- web
---
# kit
";

/// D85, D86: `check` installs nothing and says so; `decree install` installs the cron
/// file into `machines/kit/cron/`, which the daemon's scan and `check` read, and writes the
/// state and order seeds once; removing the plugin keeps its state and order.
#[test]
fn a_plugin_carries_cron_state_and_order_and_installs_only_when_asked() {
    let p = Project::new();
    p.put("plugins/helper.md", HELPER);
    p.put("plugins/kit.md", KIT);
    let (code, _, err) = p.decree(&["check"], "");
    assert_eq!(code, 0, "{err}");
    assert!(
        err.contains("plugins/kit.md: not installed, or changed since it was installed: run `decree install`"),
        "{err}"
    );
    assert!(!p.exists("machines"));

    let (code, out, err) = p.decree(&["install"], "");
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out,
        "installed .decree/plugins/helper.md\ninstalled .decree/plugins/kit.md\n"
    );
    assert!(!p.exists("machines/kit/machine.yml"));
    assert!(p
        .get("machines/kit/cron/daily.md")
        .contains("cron: 0 9 * * *"));
    assert!(p.get("machines/kit/order/A01-kit.md").contains("Set up."));
    assert_eq!(p.get("machines/kit/state/seeds.txt"), "a\n");
    assert_eq!(p.decree(&["graph"], "").0, 0);
    let (code, out, err) = p.decree(&["check"], "");
    assert_eq!((code, out.as_str()), (0, ""), "{err}");
    let (code, out, err) = p.decree(&["status", "--cron", "--format", "json"], "");
    assert_eq!(code, 0, "{err}");
    let doc: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doc["cron"][0]["file"], "kit/daily.md", "{doc}");

    // A cron file a plugin installs is checked like any other (M3).
    p.put(
        "machines/kit/cron/daily.md",
        "---\ncron: 0 9 * * *\nmachine: nobody\n---\n",
    );
    let (code, out, _) = p.decree(&["check"], "");
    assert_eq!(code, 1);
    assert!(out.contains("machines/kit/cron/daily.md"), "{out}");
    p.put(
        "machines/kit/cron/daily.md",
        "---\ncron: 0 9 * * *\nmachine: helper\n---\nDaily.\n",
    );

    // Export keeps the `x-` key and the state and order seeds.
    let (code, _, err) = p.decree(&["export", "kit"], "");
    assert_eq!(code, 0, "{err}");
    assert_eq!(p.get("plugins/kit.md"), KIT);

    p.put("machines/kit/state/seeds.txt", "mine\n");
    fs::remove_file(p.path("plugins/kit.md")).unwrap();
    let (code, out, err) = p.decree(&["install"], "");
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("removed the installed copy of .decree/plugins/kit.md"),
        "{out}"
    );
    assert!(!p.exists("machines/kit/cron") && !p.exists("machines/kit/scripts"));
    assert!(p.exists("machines/kit/order/A01-kit.md"));
    assert_eq!(p.get("machines/kit/state/seeds.txt"), "mine\n");
}

/// The official `plugin_install`: its run installs what `process` only reports.
#[test]
fn the_plugin_install_plugin_installs_the_plugins() {
    let p = Project::new();
    let (code, _, err) = p.decree(&["install", "plugin_install"], "");
    assert_eq!(code, 0, "{err}");
    assert!(p
        .get("machines/plugin_install/cron/boot.md")
        .contains("'@reboot'"));
    p.put("plugins/helper.md", HELPER);
    p.put(
        "inbox/a.md",
        "---\nmachine: plugin_install\n---\nInstall.\n",
    );
    let bin = Path::new(env!("CARGO_BIN_EXE_decree")).parent().unwrap();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = common::decree_at(&p.root)
        .env("NO_COLOR", "1")
        .env("PATH", path)
        .args(["daemon", "--once"])
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    assert!(text.contains("plugins/helper.md: not installed"), "{text}");
    assert!(p.exists("machines/helper/machine.yml"), "{text}");
}
