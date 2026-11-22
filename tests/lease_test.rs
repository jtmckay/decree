//! The official `lease` plugin (plugins/lease.md, migration A12): one holder per resource,
//! batched by mode, with linger and max_wait. Each test is a fresh project with `lease`
//! added, the resource `gpu` at `max_lease 3s`, `linger 1s` and `max_wait 4s`, a stub swap
//! script that records each call in `swaps.log` (and fails for mode `z`, and sleeps for mode
//! `slow`), and `decree daemon --interval 1s` running. A lease is held exactly while its
//! run's state is `held`; every wait polls, and checks that no two leases are ever `held`.

use assert_cmd::Command;
use serde_json::Value;
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const RULES: &str = "# test rules\ngpu 3s 1s 4s\n";

const STUB: &str = "#!/usr/bin/env bash\n\
printf '\"%s\" -> %s\\n' \"$1\" \"$2\" >> swaps.log\n\
[ \"$2\" != slow ] || sleep 60\n\
[ \"$2\" != z ]\n";

struct Project {
    tmp: TempDir,
    leases: RefCell<Vec<String>>,
    daemon: RefCell<Option<Child>>,
}

/// decree's own `DECREE_*` variables, when the tests run inside a decree run, would point
/// the binary at that project; drop them all.
fn clean(cmd: &mut std::process::Command) {
    for (k, _) in std::env::vars() {
        if k.starts_with("DECREE") {
            cmd.env_remove(k);
        }
    }
    cmd.env("NO_COLOR", "1");
}

impl Project {
    /// `decree init`, `decree install lease`, the test rules and the stub swap script.
    fn new() -> Project {
        Project::with_rules(RULES)
    }

    /// As `new`, with these rules for `gpu`.
    fn with_rules(rules: &str) -> Project {
        let p = Project {
            tmp: TempDir::new().unwrap(),
            leases: RefCell::new(Vec::new()),
            daemon: RefCell::new(None),
        };
        p.decree(&["init"]).assert().success();
        p.decree(&["install", "lease"]).assert().success();
        fs::write(p.lease_dir().join("resources"), rules).unwrap();
        let stub = p.lease_dir().join("swap/gpu.sh");
        fs::write(&stub, STUB).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    fn root(&self) -> PathBuf {
        self.tmp.path().to_path_buf()
    }

    fn lease_dir(&self) -> PathBuf {
        self.root().join(".decree/machines/lease/scripts")
    }

    fn decree(&self, args: &[&str]) -> Command {
        let mut std_cmd = std::process::Command::new(env!("CARGO_BIN_EXE_decree"));
        clean(&mut std_cmd);
        let mut cmd = Command::from_std(std_cmd);
        cmd.current_dir(self.root()).args(args);
        cmd
    }

    fn start(&self) {
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_decree"));
        clean(&mut cmd);
        let child = cmd
            .current_dir(self.root())
            .args(["daemon", "--interval", "1s"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        *self.daemon.borrow_mut() = Some(child);
    }

    /// SIGTERM the daemon and wait for it to exit.
    fn stop(&self) {
        let Some(mut child) = self.daemon.borrow_mut().take() else {
            return;
        };
        // SAFETY: sends SIGTERM to the daemon this test spawned.
        unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
        let deadline = Instant::now() + Duration::from_secs(20);
        while child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "daemon did not exit");
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Asks for a lease; returns its id.
    fn ask(&self, params: &[&str]) -> String {
        let mut args = vec!["emit", "--machine", "lease"];
        for p in params {
            args.push("--param");
            args.push(p);
        }
        let out = self
            .decree(&args)
            .write_stdin("A lease.\n")
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let id = String::from_utf8(out).unwrap().trim().to_string();
        assert!(!id.is_empty());
        self.leases.borrow_mut().push(id.clone());
        id
    }

    fn ask_mode(&self, mode: &str) -> String {
        self.ask(&["resource=gpu", &format!("mode={mode}")])
    }

    fn run_json(&self, id: &str) -> Value {
        let out = self
            .decree(&["status", id, "--format", "json"])
            .output()
            .unwrap();
        serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
    }

    /// (status, state) of a run; ("", "") while it is still in the inbox.
    fn state(&self, id: &str) -> (String, String) {
        let v = self.run_json(id);
        (
            v["status"].as_str().unwrap_or("").to_string(),
            v["state"].as_str().unwrap_or("").to_string(),
        )
    }

    fn held(&self, id: &str) -> bool {
        self.state(id).1 == "held"
    }

    /// No two leases are ever held at once.
    fn check_one_holder(&self) {
        let ids = self.leases.borrow().clone();
        let held: Vec<&String> = ids.iter().filter(|id| self.held(id)).collect();
        assert!(held.len() <= 1, "two holders at once: {held:?}");
    }

    /// Polls until `done` (checking one holder at a time throughout); panics after 20s.
    fn wait_for(&self, what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            self.check_one_holder();
            if done() {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting: {what}");
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn wait_state(&self, id: &str, state: &str) {
        self.wait_for(&format!("{id} in {state}"), || self.state(id).1 == state);
    }

    /// Waits for the run's `run_finished` event, which follows its final state's onentry
    /// (`leave`); status says `finished` as soon as the final state is entered.
    fn wait_finished(&self, id: &str, state: &str) {
        self.wait_for(&format!("{id} finished in {state}"), || {
            self.run_json(id)["events"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|e| e["type"] == "run_finished" && e["state"] == state)
        });
    }

    /// Watches for `d`, checking that `id` is never held meanwhile.
    fn never_held_for(&self, id: &str, d: Duration) {
        let end = Instant::now() + d;
        while Instant::now() < end {
            self.check_one_holder();
            assert!(!self.held(id), "{id} was held");
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// `decree reply <id> <event>`. A lease looks every second, and for that instant it is
    /// not waiting, so `decree reply` refuses; send again, as a client would.
    fn send(&self, id: &str, event: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let out = self.decree(&["reply", id, event]).output().unwrap();
            if out.status.success() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "decree reply {id} {event}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// `decree reply <id> release` if the run still waits; nothing if it has finished.
    fn release_if_live(&self, id: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.state(id).0 != "finished" {
            if self
                .decree(&["reply", id, "release"])
                .output()
                .unwrap()
                .status
                .success()
            {
                return;
            }
            assert!(Instant::now() < deadline, "decree reply {id} release");
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn swaps(&self) -> Vec<String> {
        fs::read_to_string(self.root().join("swaps.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// Every script log of the run, in order.
    fn log(&self, id: &str) -> String {
        let dir = self.root().join(".decree/runs").join(id);
        let mut logs: Vec<PathBuf> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "log"))
            .collect();
        logs.sort();
        logs.iter()
            .map(|p| fs::read_to_string(p).unwrap())
            .collect()
    }

    /// Still in line: waiting, or in `turn` for its look each second, and never held.
    fn in_line(&self, id: &str) -> bool {
        let (status, state) = self.state(id);
        status != "finished" && (state == "waiting" || state == "turn") && !self.ever_in(id, "held")
    }

    /// Whether the run was ever in `state`.
    fn ever_in(&self, id: &str, state: &str) -> bool {
        self.run_json(id)["events"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|e| e["to"] == state)
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        if let Some(mut child) = self.daemon.borrow_mut().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn path_has(path: &Path, text: &str) -> bool {
    fs::read_to_string(path).unwrap_or_default().contains(text)
}

/// AC: no lease on gpu; a lease for x is held, the stub records `"" -> x` once, and release
/// ends the run in `done`.
#[test]
fn a_free_resource_is_swapped_and_held_then_released() {
    let p = Project::new();
    p.start();
    let a = p.ask(&["resource=gpu", "mode=x", "holder=box-1"]);
    p.wait_state(&a, "held");
    assert_eq!(p.swaps(), vec!["\"\" -> x"]);
    p.send(&a, "release");
    p.wait_finished(&a, "done");
    assert_eq!(p.swaps(), vec!["\"\" -> x"]);
    let log = p.log(&a);
    for line in [
        "lease: grant ",
        "lease: swap ",
        "lease: held ",
        "lease: release ",
    ] {
        assert!(log.contains(line), "{line} missing in {log}");
    }
    assert!(
        log.contains(&format!("resource=gpu mode=x lease={a}")),
        "{log}"
    );
    assert!(log.contains("holder=\"box-1\""), "{log}");
    p.stop();
}

/// AC: A holds gpu in x; B for x is not held until A is released, then held with no swap.
#[test]
fn a_second_lease_in_the_same_mode_waits_then_goes_without_a_swap() {
    let p = Project::new();
    p.start();
    let a = p.ask_mode("x");
    p.wait_state(&a, "held");
    let b = p.ask_mode("x");
    p.wait_state(&b, "waiting");
    p.never_held_for(&b, Duration::from_millis(800));
    p.send(&a, "release");
    p.wait_state(&b, "held");
    assert_eq!(p.state(&a), ("finished".into(), "done".into()));
    assert_eq!(p.swaps(), vec!["\"\" -> x"]);
    p.stop();
}

/// AC: A holds x, B asks y, C asks x. A released: C next, no swap; C released: B after the
/// linger, with exactly one `x -> y`.
#[test]
fn the_current_mode_goes_first_then_the_other_after_the_linger() {
    let p = Project::new();
    p.start();
    let a = p.ask_mode("x");
    p.wait_state(&a, "held");
    let b = p.ask_mode("y");
    let c = p.ask_mode("x");
    p.wait_state(&b, "waiting");
    p.wait_state(&c, "waiting");
    p.send(&a, "release");
    p.wait_state(&c, "held");
    assert!(!p.held(&b));
    assert_eq!(p.swaps(), vec!["\"\" -> x"]);
    let released = Instant::now();
    p.send(&c, "release");
    p.wait_state(&b, "held");
    assert!(
        released.elapsed() >= Duration::from_millis(1000),
        "B was held {:?} after C's release, before the 1s linger",
        released.elapsed()
    );
    assert_eq!(p.swaps(), vec!["\"\" -> x", "\"x\" -> y"]);
    p.stop();
}

/// AC: A holds x and nothing waits; A released and D for x asked within the linger: D is
/// held with no swap.
#[test]
fn a_lease_in_the_lingering_mode_goes_without_a_swap() {
    let p = Project::new();
    p.start();
    let a = p.ask_mode("x");
    p.wait_state(&a, "held");
    p.send(&a, "release");
    let d = p.ask_mode("x");
    p.wait_state(&d, "held");
    assert_eq!(p.state(&a), ("finished".into(), "done".into()));
    assert_eq!(p.swaps(), vec!["\"\" -> x"]);
    p.stop();
}

/// AC: A holds x and B waits for y; A released with no x lease asked: B is held once the
/// linger has passed, after one `x -> y`.
#[test]
fn another_mode_goes_once_the_linger_has_passed() {
    let p = Project::new();
    p.start();
    let a = p.ask_mode("x");
    p.wait_state(&a, "held");
    let b = p.ask_mode("y");
    p.wait_state(&b, "waiting");
    let released = Instant::now();
    p.send(&a, "release");
    p.wait_state(&b, "held");
    assert!(
        released.elapsed() >= Duration::from_millis(1000),
        "B held {:?} after the release",
        released.elapsed()
    );
    assert_eq!(p.swaps(), vec!["\"\" -> x", "\"x\" -> y"]);
    let log = p.log(&b);
    assert!(
        log.contains(&format!(
            "lease: swap resource=gpu mode=y lease={b} from=x to=y"
        )),
        "{log}"
    );
    p.stop();
}

/// AC: gpu was swapped to x and leases for x keep arriving; B asks for y after gpu has been
/// in x longer than max_wait (4s): the next grant goes to B though leases for x wait.
#[test]
fn a_mode_past_its_slice_gives_way_to_another_mode() {
    let p = Project::new();
    p.start();
    let a = p.ask_mode("x");
    p.wait_state(&a, "held");
    let swapped = Instant::now();
    // A expires at 3s; C1, in the current mode and alone in line, goes next, until 6s or later.
    let c1 = p.ask_mode("x");
    p.wait_state(&c1, "held");
    let c2 = p.ask_mode("x");
    thread::sleep(Duration::from_millis(4300).saturating_sub(swapped.elapsed()));
    let b = p.ask_mode("y");
    p.wait_state(&b, "waiting");
    p.wait_state(&c2, "waiting");
    // Released, or expired meanwhile: either way the resource is next free with C2 and B in line.
    p.release_if_live(&c1);
    p.wait_state(&b, "held");
    assert!(p.in_line(&c2), "{:?}", p.state(&c2));
    assert_eq!(p.swaps(), vec!["\"\" -> x", "\"x\" -> y"]);
    p.send(&b, "release");
    p.wait_state(&c2, "held");
    p.stop();
}

/// AC: gpu was just swapped to x, leases for x keep arriving and B asks for y at once:
/// leases for x keep being granted inside the slice; once max_wait has passed since the
/// swap, the next grant goes to B.
#[test]
fn a_mode_just_swapped_in_serves_its_own_leases_until_its_slice_is_used() {
    let p = Project::new();
    p.start();
    let a = p.ask_mode("x");
    p.wait_state(&a, "held");
    let swapped = Instant::now();
    let b = p.ask_mode("y");
    let c1 = p.ask_mode("x");
    let c2 = p.ask_mode("x");
    for id in [&b, &c1, &c2] {
        p.wait_state(id, "waiting");
    }
    p.send(&a, "release");
    p.wait_state(&c1, "held");
    assert!(!p.held(&b));
    p.send(&c1, "release");
    p.wait_state(&c2, "held");
    assert!(
        swapped.elapsed() < Duration::from_secs(4),
        "C2 was held {:?} after the swap, past the slice",
        swapped.elapsed()
    );
    assert!(!p.held(&b));
    assert_eq!(p.swaps(), vec!["\"\" -> x"]);
    let c3 = p.ask_mode("x");
    p.wait_state(&c3, "waiting");
    thread::sleep(Duration::from_millis(4300).saturating_sub(swapped.elapsed()));
    p.send(&c2, "release");
    p.wait_state(&b, "held");
    assert!(p.in_line(&c3), "{:?}", p.state(&c3));
    assert_eq!(p.swaps(), vec!["\"\" -> x", "\"x\" -> y"]);
    p.stop();
}

/// AC: gpu has been in x longer than max_wait and only leases for x wait: when the holder is
/// released, the next lease for x is held, with no swap.
#[test]
fn a_mode_past_its_slice_goes_on_while_no_other_mode_waits() {
    let p = Project::new();
    p.start();
    let a = p.ask_mode("x");
    p.wait_state(&a, "held");
    let swapped = Instant::now();
    let c1 = p.ask_mode("x");
    p.wait_state(&c1, "held");
    let c2 = p.ask_mode("x");
    p.wait_state(&c2, "waiting");
    thread::sleep(Duration::from_millis(4300).saturating_sub(swapped.elapsed()));
    p.send(&c1, "release");
    p.wait_state(&c2, "held");
    assert_eq!(p.swaps(), vec!["\"\" -> x"]);
    p.stop();
}

/// Once its slice is used, a mode does not linger: with a 5s linger (and a 10s max_lease, so
/// A holds past the 4s slice), B (for y) is held soon after A is released, not 5s later.
#[test]
fn a_mode_past_its_slice_does_not_linger() {
    let p = Project::with_rules("gpu 10s 5s 4s\n");
    p.start();
    let a = p.ask_mode("x");
    p.wait_state(&a, "held");
    let swapped = Instant::now();
    thread::sleep(Duration::from_millis(4300).saturating_sub(swapped.elapsed()));
    let b = p.ask_mode("y");
    p.wait_state(&b, "waiting");
    let released = Instant::now();
    p.send(&a, "release");
    p.wait_state(&b, "held");
    assert!(
        released.elapsed() < Duration::from_secs(4),
        "B held {:?} after the release: it lingered",
        released.elapsed()
    );
    assert_eq!(p.swaps(), vec!["\"\" -> x", "\"x\" -> y"]);
    p.stop();
}

/// AC: a held lease nobody releases ends at max_lease, and the next waiting lease is held.
#[test]
fn a_lease_nobody_releases_ends_at_its_expiry() {
    let p = Project::new();
    p.start();
    let a = p.ask_mode("x");
    p.wait_state(&a, "held");
    let held_at = Instant::now();
    let b = p.ask_mode("x");
    p.wait_finished(&a, "done");
    let held_for = held_at.elapsed();
    assert!(held_for >= Duration::from_millis(2500), "{held_for:?}");
    p.wait_state(&b, "held");
    assert!(p
        .log(&a)
        .contains(&format!("lease: expire resource=gpu mode=x lease={a}")));
    p.stop();
}

/// AC: duration 10s is capped at max_lease 3s, and the log names the cap.
#[test]
fn a_duration_past_max_lease_is_capped() {
    let p = Project::new();
    p.start();
    let a = p.ask(&["resource=gpu", "mode=x", "duration=10s"]);
    p.wait_state(&a, "held");
    let held_at = Instant::now();
    p.wait_finished(&a, "done");
    assert!(
        held_at.elapsed() < Duration::from_secs(6),
        "{:?}",
        held_at.elapsed()
    );
    let log = p.log(&a);
    assert!(
        log.contains(&format!(
            "lease: cap resource=gpu mode=x lease={a} duration=10s max_lease=3s"
        )),
        "{log}"
    );
    p.stop();
}

/// AC: a renewal while another lease waits is refused in the log, and the lease ends at its
/// original expiry.
#[test]
fn a_renewal_is_refused_while_another_lease_waits() {
    let p = Project::new();
    p.start();
    let a = p.ask_mode("x");
    p.wait_state(&a, "held");
    let held_at = Instant::now();
    let b = p.ask_mode("x");
    p.wait_state(&b, "waiting");
    p.send(&a, "renew");
    p.wait_for("the refusal", || {
        p.log(&a).contains("lease: renew refused ")
    });
    p.wait_finished(&a, "done");
    assert!(
        held_at.elapsed() < Duration::from_millis(5000),
        "{:?}",
        held_at.elapsed()
    );
    let log = p.log(&a);
    assert!(
        log.contains(&format!(
            "lease: renew refused resource=gpu mode=x lease={a}"
        )),
        "{log}"
    );
    assert!(log.contains("lease: expire "), "{log}");
    p.wait_state(&b, "held");
    p.stop();
}

/// AC: a renewal with nothing waiting keeps the lease past its original expiry.
#[test]
fn a_renewal_with_nothing_waiting_extends_the_lease() {
    let p = Project::new();
    p.start();
    let a = p.ask_mode("x");
    p.wait_state(&a, "held");
    let held_at = Instant::now();
    thread::sleep(Duration::from_millis(1500));
    p.send(&a, "renew");
    p.wait_for("the renewal", || {
        p.log(&a).contains("lease: renew resource=")
    });
    // The original expiry was 3s after the hold began.
    let past = Duration::from_millis(3800).saturating_sub(held_at.elapsed());
    thread::sleep(past);
    assert!(p.held(&a), "{:?}", p.state(&a));
    p.send(&a, "release");
    p.wait_finished(&a, "done");
    p.stop();
}

/// AC: a lease still waiting in line, sent release, ends in done and is never held.
#[test]
fn a_waiting_lease_can_be_released() {
    let p = Project::new();
    p.start();
    let a = p.ask_mode("x");
    p.wait_state(&a, "held");
    let b = p.ask_mode("y");
    p.wait_state(&b, "waiting");
    p.send(&b, "release");
    p.wait_finished(&b, "done");
    assert!(!p.ever_in(&b, "held"));
    assert!(!p.ever_in(&b, "swap"));
    assert!(p.log(&b).contains("never held"), "{}", p.log(&b));
    p.send(&a, "release");
    p.wait_finished(&a, "done");
    assert_eq!(p.swaps(), vec!["\"\" -> x"]);
    p.stop();
}

/// AC: the holder's run is interrupted; once its expiry passes, the next waiting lease is
/// held, and no two leases are ever held at once.
#[test]
fn an_interrupted_holder_holds_nothing_past_its_expiry() {
    let p = Project::new();
    p.start();
    // The swap for `slow` takes a minute: the lease holds gpu while it swaps.
    let a = p.ask_mode("slow");
    p.wait_state(&a, "swap");
    let b = p.ask_mode("x");
    p.stop();
    assert_eq!(p.state(&a), ("interrupted".into(), "swap".into()));
    p.start();
    p.wait_state(&b, "held");
    assert_eq!(p.state(&a).0, "interrupted");
    let log = p.log(&b);
    assert!(
        log.contains(&format!("lease: expire resource=gpu mode=slow lease={a}")),
        "{log}"
    );
    // The interrupted swap left the mode unknown.
    assert_eq!(p.swaps(), vec!["\"\" -> slow", "\"\" -> x"]);
    p.send(&b, "release");
    p.wait_finished(&b, "done");
    p.stop();
}

/// AC: a swap that fails for z fails that lease, and the next lease swaps from unknown.
#[test]
fn a_failed_swap_fails_the_lease_and_leaves_the_mode_unknown() {
    let p = Project::new();
    p.start();
    let a = p.ask_mode("x");
    p.wait_state(&a, "held");
    let z = p.ask_mode("z");
    p.send(&a, "release");
    p.wait_finished(&z, "failed");
    assert!(p.log(&z).contains("lease: swap failed "), "{}", p.log(&z));
    let y = p.ask_mode("y");
    p.wait_state(&y, "held");
    assert_eq!(p.swaps(), vec!["\"\" -> x", "\"x\" -> z", "\"\" -> y"]);
    p.send(&y, "release");
    p.wait_finished(&y, "done");
    p.stop();
}

/// AC: a lease for tpu, not in the rules, fails at once naming tpu and listing gpu; a
/// lease with no mode fails naming the mode.
#[test]
fn a_bad_request_fails_at_once() {
    let p = Project::new();
    let t = p.ask(&["resource=tpu", "mode=x"]);
    let m = p.ask(&["resource=gpu"]);
    let n = p.ask(&["mode=x"]);
    let _ = p.decree(&["daemon", "--once"]).output().unwrap();
    for id in [&t, &m, &n] {
        assert_eq!(p.state(id), ("finished".into(), "failed".into()), "{id}");
    }
    let log = p.log(&t);
    assert!(log.contains("unknown resource `tpu`"), "{log}");
    assert!(log.contains("The resources are: gpu"), "{log}");
    assert!(
        p.log(&m).contains("no mode for resource `gpu`"),
        "{}",
        p.log(&m)
    );
    assert!(p.log(&n).contains("no resource"), "{}", p.log(&n));
    assert!(p.log(&n).contains("gpu"), "{}", p.log(&n));
    assert!(p.swaps().is_empty());
}

/// AC: a fresh init lists lease with its description; after adding it, check passes.
#[test]
fn lease_is_listed_and_adds_cleanly() {
    let tmp = TempDir::new().unwrap();
    let run = |args: &[&str]| {
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_decree"));
        clean(&mut cmd);
        let mut cmd = Command::from_std(cmd);
        cmd.current_dir(tmp.path())
            .args(args)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone()
    };
    run(&["init"]);
    let list = String::from_utf8(run(&["list"])).unwrap();
    let line = list
        .lines()
        .find(|l| l.trim_start().starts_with("lease "))
        .unwrap_or_else(|| panic!("{list}"));
    assert!(line.contains("One holder per resource"), "{line}");
    run(&["install", "lease"]);
    run(&["check"]);
    let req = tmp
        .path()
        .join(".decree/machines/lease/scripts/requirements");
    for tool in ["flock", "jq"] {
        assert!(path_has(&req, &format!("command   {tool}")), "{tool}");
    }
    assert!(path_has(
        &tmp.path().join(".decree/machines/lease/scripts/resources"),
        "\ngpu 15m 5s 30m\n"
    ));
}
