//! install.sh picks the release the README promises: the newest stable from 0.5.0 on, however
//! many betas follow it; while there is none, the newest beta from 0.5.0 on; a version given on
//! the command line wins. A stub `curl` serves the release list and the assets, so no test
//! touches the network.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

const ASSET: &str = "decree-x86_64-linux";

/// Run install.sh with a stub `curl` whose release list has `tags`, newest first as GitHub
/// lists them, each serving a fake binary that prints its own version. Returns the installed
/// `decree --version`, or install.sh's stderr on failure.
fn install(tags: &[&str], args: &[&str]) -> Result<String, String> {
    install_with(tags, args, false)
}

/// `install`, with every `.sha256` wrong when `corrupt`.
fn install_with(tags: &[&str], args: &[&str], corrupt: bool) -> Result<String, String> {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let rel = root.join("rel");
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let list: Vec<String> = tags
        .iter()
        .map(|t| format!("  {{\n    \"tag_name\": \"{t}\",\n    \"name\": \"decree {t}\"\n  }}"))
        .collect();
    fs::write(
        root.join("releases.json"),
        format!("[\n{}\n]\n", list.join(",\n")),
    )
    .unwrap();
    for t in tags {
        let d = rel.join(t);
        fs::create_dir_all(&d).unwrap();
        let exe = d.join(ASSET);
        fs::write(&exe, format!("#!/bin/sh\necho 'decree {}'\n", &t[1..])).unwrap();
        let sum = Command::new("sha256sum")
            .arg(ASSET)
            .current_dir(&d)
            .output()
            .unwrap();
        let sum = if corrupt {
            format!("{}  {ASSET}\n", "0".repeat(64)).into_bytes()
        } else {
            sum.stdout
        };
        fs::write(d.join(format!("{ASSET}.sha256")), sum).unwrap();
    }
    // `curl [-o FILE] URL`: the API URL prints the list; a download URL copies rel/<tag>/<file>.
    let stub = format!(
        r#"#!/bin/sh
out=; prev=; url=
for a; do [ "$prev" = -o ] && out=$a; prev=$a; url=$a; done
case "$url" in
  *api.github.com*) cat '{root}/releases.json'; exit 0 ;;
esac
rest=${{url#*/releases/download/}}
f='{rel}'/$rest
[ -f "$f" ] || exit 22
cp "$f" "$out"
"#,
        root = root.display(),
        rel = rel.display()
    );
    let curl = bin.join("curl");
    fs::write(&curl, stub).unwrap();
    fs::set_permissions(&curl, fs::Permissions::from_mode(0o755)).unwrap();

    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let out = Command::new("sh")
        .arg(&script)
        .args(args)
        .env("PATH", path)
        .env("XDG_BIN_HOME", root.join("out"))
        .output()
        .unwrap();
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    let v = Command::new(root.join("out/decree"))
        .arg("--version")
        .output()
        .unwrap();
    Ok(String::from_utf8_lossy(&v.stdout).trim().to_string())
}

#[test]
fn only_betas_from_0_5_installs_the_newest_beta() {
    let got = install(&["v0.5.0-beta.12", "v0.5.0-beta.9", "v0.2.0"], &[]);
    assert_eq!(got.unwrap(), "decree 0.5.0-beta.12");
}

#[test]
fn a_stable_from_0_5_wins_over_newer_betas() {
    let tags = [
        "v0.6.0-beta.3",
        "v0.6.0-beta.2",
        "v0.5.1",
        "v0.5.0",
        "v0.5.0-beta.12",
        "v0.2.0",
    ];
    assert_eq!(install(&tags, &[]).unwrap(), "decree 0.5.1");
}

#[test]
fn a_newer_stable_replaces_the_old_one() {
    let tags = ["v0.6.0", "v0.6.0-beta.1", "v0.5.10", "v0.5.9"];
    assert_eq!(install(&tags, &[]).unwrap(), "decree 0.6.0");
}

#[test]
fn releases_before_0_5_are_never_picked() {
    let err = install(&["v0.3.0", "v0.2.0"], &[]).unwrap_err();
    assert!(err.contains("no release of 0.5.0 or later found"), "{err}");
}

#[test]
fn a_version_given_wins() {
    let tags = ["v0.5.1", "v0.5.0-beta.12"];
    assert_eq!(
        install(&tags, &["v0.5.0-beta.12"]).unwrap(),
        "decree 0.5.0-beta.12"
    );
}

#[test]
fn a_version_with_no_release_fails() {
    let err = install(&["v0.5.0-beta.12"], &["v0.5.0-beta.99"]).unwrap_err();
    assert!(
        err.contains("v0.5.0-beta.99 has no decree-x86_64-linux"),
        "{err}"
    );
}

#[test]
fn a_bad_checksum_installs_nothing() {
    let err = install_with(&["v0.5.0-beta.12"], &[], true).unwrap_err();
    assert!(
        err.contains("checksum mismatch for v0.5.0-beta.12"),
        "{err}"
    );
}
