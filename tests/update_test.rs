//! `decree install --update` (docs/reference/plugins.md, `source`): a plugin published at an
//! https URL is fetched with `DECREE_CURL` (a stub here: no network), checked, shown as a diff
//! and taken only after `y` on a terminal. A terminal is util-linux's `script`, which gives the
//! command a pseudo-terminal fed from the test's input; without it the tests that need one
//! are skipped.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tempfile::TempDir;

mod common;

const URL: &str = "https://example.com/plugins/x.md";

/// The plugin `x`, published at `URL`, whose script echoes `word`.
fn plugin(word: &str) -> String {
    format!(
        "---\nname: x\nsource: {URL}\nmachine:\n  description: Says a word.\n  initial: say\n  states:\n    say:\n      invoke: say\n      transitions:\n        done: done\n    done:\n      final: true\n    failed:\n      final: true\nscripts:\n  say.sh: |\n    #!/usr/bin/env bash\n    echo {word}\n---\n# x\n\nSays a word.\n"
    )
}

/// A stub curl: prints `$STUB_BODY` (a file), or with `STUB_FAIL` set fails as curl does,
/// saying so on stderr. It records its arguments in `$STUB_BODY.args`.
const STUB_CURL: &str = "#!/usr/bin/env bash\n\
printf '%s\\n' \"$@\" > \"${STUB_BODY}.args\"\n\
if [ -n \"${STUB_FAIL:-}\" ]; then echo \"curl: (6) Could not resolve host: example.com\" >&2; exit 6; fi\n\
cat \"$STUB_BODY\"\n";

struct Project {
    tmp: TempDir,
}

impl Project {
    /// A project with `x` installed from `plugin("one")`, and the stub curl.
    fn new() -> Project {
        let tmp = TempDir::new().unwrap();
        let p = Project { tmp };
        fs::create_dir_all(p.root().join(".decree/plugins")).unwrap();
        fs::write(p.file(), plugin("one")).unwrap();
        common::write_script(&p.root().join("curl-stub"), STUB_CURL);
        let out = common::std_decree_at(p.root())
            .arg("install")
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        p
    }

    fn root(&self) -> PathBuf {
        self.tmp.path().to_path_buf()
    }

    fn file(&self) -> PathBuf {
        self.root().join(".decree/plugins/x.md")
    }

    fn body(&self) -> PathBuf {
        self.root().join("body.md")
    }

    /// Make the stub serve `text`.
    fn serve(&self, text: &str) {
        fs::write(self.body(), text).unwrap();
    }

    fn installed(&self) -> String {
        fs::read_to_string(self.root().join(".decree/machines/x/scripts/say.sh")).unwrap()
    }

    fn archives(&self) -> Vec<String> {
        fs::read_dir(self.root().join(".decree/archive/x"))
            .map(|d| {
                d.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `decree install --update <args>` with stdin not a terminal: (code, stdout, stderr).
    fn update(&self, args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
        let mut cmd = common::std_decree_at(self.root());
        cmd.args(["install", "--update"])
            .args(args)
            .env("DECREE_CURL", self.root().join("curl-stub"))
            .env("STUB_BODY", self.body())
            .env("NO_COLOR", "1")
            .stdin(Stdio::null());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// `decree install --update x` on a terminal, answering `answer`: (code, everything the
    /// terminal showed).
    fn update_on_terminal(&self, answer: &str) -> (i32, String) {
        let decree = env!("CARGO_BIN_EXE_decree");
        let mut child = Command::new("script")
            .args(["-qec", &format!("{decree} install --update x"), "/dev/null"])
            .current_dir(self.root())
            .env("DECREE_CURL", self.root().join("curl-stub"))
            .env("STUB_BODY", self.body())
            .env("NO_COLOR", "1")
            .env_remove("ORDER_MAX")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(answer.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
        )
    }
}

/// Whether util-linux's `script` can give a command a terminal here.
fn has_script() -> bool {
    Command::new("script")
        .args(["-qec", "test -t 0", "/dev/null"])
        .stdin(Stdio::piped())
        .output()
        .is_ok_and(|o| o.status.success())
}

/// AC (A14): with a stub that serves a changed copy, `--update x` on a terminal answered `n`
/// prints the diff and writes nothing; answered `y` it writes and installs the copy and
/// archives the old file; then, with stdin not a terminal and a further change, it prints the
/// diff and the `not updated` line and writes nothing.
#[test]
fn an_update_is_taken_only_after_y_on_a_terminal() {
    if !has_script() {
        eprintln!("no util-linux `script`: skipping");
        return;
    }
    let p = Project::new();
    p.serve(&plugin("two"));
    let (code, shown) = p.update_on_terminal("n\n");
    assert_eq!(code, 0, "{shown}");
    assert!(
        shown.contains("--- .decree/plugins/x.md\n+++ https://example.com/plugins/x.md\n"),
        "{shown}"
    );
    assert!(
        shown.contains("-     echo one\n+     echo two\n"),
        "{shown}"
    );
    assert!(
        shown.contains("Take this update to .decree/plugins/x.md? [y/N]"),
        "{shown}"
    );
    assert!(
        shown.contains("not updated: .decree/plugins/x.md is unchanged"),
        "{shown}"
    );
    assert_eq!(fs::read_to_string(p.file()).unwrap(), plugin("one"));
    assert!(p.installed().contains("echo one"));
    assert!(p.archives().is_empty());

    let (code, shown) = p.update_on_terminal("y\n");
    assert_eq!(code, 0, "{shown}");
    assert!(
        shown.contains("updated .decree/plugins/x.md; installed x"),
        "{shown}"
    );
    assert_eq!(fs::read_to_string(p.file()).unwrap(), plugin("two"));
    assert!(p.installed().contains("echo two"));
    let archives = p.archives();
    assert_eq!(archives.len(), 1, "{archives:?}");
    assert_eq!(
        fs::read_to_string(p.root().join(".decree/archive/x").join(&archives[0])).unwrap(),
        plugin("one")
    );
    // The fetch is HTTPS only, redirects too, with a timeout.
    let args = fs::read_to_string(format!("{}.args", p.body().display())).unwrap();
    for arg in [
        "--proto\n=https\n",
        "--proto-redir\n=https\n",
        "--max-time\n",
    ] {
        assert!(args.contains(arg), "{args}");
    }

    p.serve(&plugin("three"));
    let (code, out, _) = p.update(&["x"], &[]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("-     echo two\n+     echo three\n"), "{out}");
    assert!(
        out.contains("not updated: run decree install --update x in a terminal to review it"),
        "{out}"
    );
    assert_eq!(fs::read_to_string(p.file()).unwrap(), plugin("two"));

    // The same file: up to date.
    p.serve(&plugin("two"));
    let (code, out, _) = p.update(&[], &[]);
    assert_eq!(
        (code, out.as_str()),
        (0, ".decree/plugins/x.md: up to date\n")
    );
}

/// AC (A14): a copy with another `name` or `source`, one that fails the schema, and a fetch
/// that fails are refused or reported, exit 1, and nothing is written; a named plugin with no
/// `source` exits 1, naming it.
#[test]
fn a_wrong_copy_or_a_failed_fetch_writes_nothing() {
    let p = Project::new();
    let before = fs::read_to_string(p.file()).unwrap();
    let cases: [(String, &str); 3] = [
        (
            plugin("two").replace("name: x", "name: y"),
            "its `name` is `y`, not `x`",
        ),
        (
            plugin("two").replace(URL, "https://example.com/other.md"),
            "its `source` is `https://example.com/other.md`, not `https://example.com/plugins/x.md`",
        ),
        (
            plugin("two").replace("machine:", "when: x\nmachine:"),
            "it fails the plugin schema: line 4: unknown key `when`",
        ),
    ];
    for (text, why) in cases {
        p.serve(&text);
        let (code, out, err) = p.update(&["x"], &[]);
        assert_eq!(code, 1, "{out}{err}");
        assert!(
            err.contains("refused the copy at https://example.com/plugins/x.md"),
            "{err}"
        );
        assert!(err.contains(why), "{err}");
        assert!(err.contains("nothing was written"), "{err}");
        assert_eq!(fs::read_to_string(p.file()).unwrap(), before);
    }
    let (code, _, err) = p.update(&["x"], &[("STUB_FAIL", "1")]);
    assert_eq!(code, 1);
    assert!(
        err.contains("fetching https://example.com/plugins/x.md failed: curl: (6) Could not resolve host: example.com"),
        "{err}"
    );
    assert_eq!(fs::read_to_string(p.file()).unwrap(), before);

    fs::write(
        p.root().join(".decree/plugins/plain.md"),
        "---\nname: plain\n---\n# plain\n",
    )
    .unwrap();
    let (code, _, err) = p.update(&["plain"], &[]);
    assert_eq!(code, 1);
    assert!(
        err.contains(".decree/plugins/plain.md has no `source`: nothing updates it"),
        "{err}"
    );
}

/// `source` takes an https URL only, and an official plugin has none.
#[test]
fn source_is_https_only() {
    let p = Project::new();
    let check = |text: &str| -> (i32, String) {
        fs::write(p.file(), text).unwrap();
        let out = common::std_decree_at(p.root())
            .arg("check")
            .env("NO_COLOR", "1")
            .output()
            .unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };
    let (code, out) = check(&plugin("one").replace("https://", "http://"));
    assert_eq!(code, 1);
    assert!(out.contains("`source` is `http://example.com/plugins/x.md`: it must be the https:// URL the plugin is published at (P1)"), "{out}");
    let (code, out) = check(&plugin("one"));
    assert_eq!(code, 0, "{out}");
}
