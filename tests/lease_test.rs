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
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
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
    /// How many leases may hold gpu at once: its rules' `holders`.
    holders: std::cell::Cell<usize>,
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
            holders: std::cell::Cell::new(1),
        };
        p.decree(&["init"]).assert().success();
        p.decree(&["install", "lease"]).assert().success();
        fs::write(p.lease_dir().join("resources.git.txt"), rules).unwrap();
        let stub = p.lease_dir().join("swap/gpu.git.sh");
        fs::create_dir_all(stub.parent().unwrap()).unwrap();
        fs::write(&stub, STUB).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    fn root(&self) -> PathBuf {
        self.tmp.path().to_path_buf()
    }

    fn lease_dir(&self) -> PathBuf {
        self.root().join(".decree/machines/lease/state")
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

    /// No more leases are ever held at once than gpu's `holders` (one, but where a test
    /// says more), and those held are all in one mode.
    fn check_one_holder(&self) {
        let ids = self.leases.borrow().clone();
        let held: Vec<&String> = ids.iter().filter(|id| self.held(id)).collect();
        assert!(
            held.len() <= self.holders.get(),
            "more holders than allowed: {held:?}"
        );
        let mut modes: Vec<String> = held
            .iter()
            .filter_map(|id| self.info(id)["mode"].as_str().map(str::to_string))
            .collect();
        modes.dedup();
        assert!(modes.len() <= 1, "holders in two modes: {held:?} {modes:?}");
    }

    /// The lease's status, `state/status/leases/<id>.json`, or Null before it joins the line.
    fn info(&self, id: &str) -> Value {
        self.status(&format!("leases/{id}.json"))
    }

    /// A file of the lease machine's `state/status/`, parsed, or Null.
    fn status(&self, path: &str) -> Value {
        fs::read_to_string(self.lease_dir().join("status").join(path))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(Value::Null)
    }

    /// How many times the run entered `held`: once per hold and once per renewal, never
    /// to look again while it holds.
    fn entered_held(&self, id: &str) -> usize {
        let events = self
            .root()
            .join(".decree/runs")
            .join(id)
            .join("events.jsonl");
        fs::read_to_string(events)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|e| e["type"] == "transition" && e["to"] == "held")
            .count()
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

    /// `decree reply <id> <event>`. A woken lease is not waiting while its turn looks again,
    /// and `decree reply` refuses then, or while a wake is queued; send again, as a client
    /// would.
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
    // A long max_lease, so no lease expires on its own here: each goes when released.
    let p = Project::with_rules("gpu 20s 1s 4s\n");
    p.start();
    let a = p.ask_mode("x");
    p.wait_state(&a, "held");
    let swapped = Instant::now();
    // C1, in the current mode and alone in line, goes next once A is released.
    let c1 = p.ask_mode("x");
    p.wait_state(&c1, "waiting");
    p.send(&a, "release");
    p.wait_state(&c1, "held");
    let c2 = p.ask_mode("x");
    p.wait_state(&c2, "waiting");
    // Past x's 4s slice, B asks for y.
    thread::sleep(Duration::from_millis(4300).saturating_sub(swapped.elapsed()));
    let b = p.ask_mode("y");
    p.wait_state(&b, "waiting");
    // C1 released: the slice is used and y waits, so B goes before C2.
    p.send(&c1, "release");
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
    // A slice of 8s, and leases that outlast it: a loaded machine takes about a second for
    // each grant and release below, which a 4s slice would not leave room for.
    let p = Project::with_rules("gpu 10s 1s 8s\n");
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
        swapped.elapsed() < Duration::from_secs(9),
        "C2 was held {:?} after the swap, past the slice",
        swapped.elapsed()
    );
    assert!(!p.held(&b));
    assert_eq!(p.swaps(), vec!["\"\" -> x"]);
    let c3 = p.ask_mode("x");
    p.wait_state(&c3, "waiting");
    thread::sleep(Duration::from_millis(8300).saturating_sub(swapped.elapsed()));
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
        released.elapsed() < Duration::from_secs(6),
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
    // It waited once, until its expiry, without looking in between.
    assert_eq!(p.entered_held(&a), 1);
    assert_eq!(p.info(&a)["state"], "expired", "{}", p.info(&a));
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
        held_at.elapsed() < Duration::from_secs(8),
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
    let expires = p.info(&a)["expires"].clone();
    p.send(&a, "renew");
    p.wait_for("the refusal", || {
        p.log(&a).contains("lease: renew refused ")
    });
    // A refused renewal leaves the expiry, and so the deadline.
    assert_eq!(p.info(&a)["expires"], expires);
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
    let expires = p.info(&a)["expires"].as_str().unwrap().to_string();
    p.send(&a, "renew");
    p.wait_for("the renewal", || {
        p.log(&a).contains("lease: renew resource=")
    });
    // The status is rewritten as the renewal's lock is let go, just after the log line.
    p.wait_for("the renewed status", || {
        p.info(&a)["expires"] != expires.as_str()
    });
    // The new expiry is later, and the held wait's deadline with it.
    let renewed = p.info(&a)["expires"].as_str().unwrap().to_string();
    assert!(renewed > expires, "{expires} -> {renewed}");
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
    p.wait_for("the slow swap to start", || !p.swaps().is_empty());
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
/// lease with no mode fails naming the mode, one with no resource names the resources,
/// and with gpu's modes listed, a lease for another mode fails naming them.
#[test]
fn a_bad_request_fails_at_once() {
    let p = Project::with_rules("gpu 3s 1s 4s x,y\n");
    let t = p.ask(&["resource=tpu", "mode=x"]);
    let m = p.ask(&["resource=gpu"]);
    let w = p.ask(&["resource=gpu", "mode=w"]);
    let n = p.ask(&["mode=x"]);
    let _ = p.decree(&["daemon", "--once"]).output().unwrap();
    for id in [&t, &m, &w, &n] {
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
    assert!(
        p.log(&w)
            .contains("resource `gpu` has no mode `w`: its modes are x,y"),
        "{}",
        p.log(&w)
    );
    assert!(
        p.log(&n)
            .contains("no resource: ask with the params resource and mode. The resources are: gpu"),
        "{}",
        p.log(&n)
    );
    assert!(p.swaps().is_empty());
}

/// AC: a held lease's status in the state folder says which run it is, what it holds, in
/// which mode, and until when; the `held` wait's deadline is that `expires`; after a release
/// it says `released`, when it ended, and keeps the rest.
#[test]
fn a_held_lease_writes_what_it_holds() {
    let p = Project::new();
    p.start();
    let a = p.ask(&["resource=gpu", "mode=y", "holder=job"]);
    p.wait_state(&a, "held");
    let info = p.info(&a);
    assert_eq!(info["run"], a.as_str(), "{info}");
    assert_eq!(info["resource"], "gpu", "{info}");
    assert_eq!(info["mode"], "y", "{info}");
    assert_eq!(info["holder"], "job", "{info}");
    assert_eq!(info["state"], "held", "{info}");
    assert!(info["asked"].as_str().unwrap().ends_with('Z'), "{info}");
    assert!(info.get("position").is_none(), "{info}");
    let expires = info["expires"].as_str().unwrap();
    assert!(expires.ends_with('Z'), "{info}");
    // The state is mirrored before the wait starts, so look until the wait is there.
    let events = p.root().join(".decree/runs").join(&a).join("events.jsonl");
    let held_wait = || {
        fs::read_to_string(&events)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .rfind(|e| e["type"] == "waiting")
            .filter(|e| e["state"] == "held")
    };
    p.wait_for("the held wait", || held_wait().is_some());
    assert_eq!(held_wait().unwrap()["timeout_at"], expires);
    p.send(&a, "release");
    p.wait_finished(&a, "done");
    let after = p.info(&a);
    assert_eq!(after["state"], "released", "{after}");
    assert_eq!(after["expires"], expires, "{after}");
    assert!(after["ended"].as_str().unwrap().ends_with('Z'), "{after}");
    p.stop();
}

/// AC: the resource's status names its mode, its holders with their expiry, and its line in
/// order with each place; each waiting lease's status has its `position`; a lease released
/// while it waits is `cancelled`; once nobody holds the resource it has `free_since`.
#[test]
fn the_status_folder_shows_each_resource_and_each_place_in_line() {
    let p = Project::with_rules("gpu 20s 1s 30m\n");
    p.start();
    let a = p.ask_mode("x");
    p.wait_state(&a, "held");
    let b = p.ask_mode("y");
    p.wait_state(&b, "waiting");
    let c = p.ask_mode("y");
    p.wait_state(&c, "waiting");
    let res = p.status("resources/gpu.json");
    assert_eq!(res["resource"], "gpu", "{res}");
    assert_eq!(res["mode"], "x", "{res}");
    assert_eq!(res["holders"][0]["lease"], a.as_str(), "{res}");
    assert_eq!(res["holders"][0]["state"], "held", "{res}");
    assert_eq!(res["holders"][0]["expires"], p.info(&a)["expires"], "{res}");
    assert_eq!(res["queue"][0]["lease"], b.as_str(), "{res}");
    assert_eq!(res["queue"][1]["lease"], c.as_str(), "{res}");
    assert_eq!(res["queue"][1]["position"], 2, "{res}");
    assert!(res["free_since"].is_null(), "{res}");
    assert_eq!(p.info(&b)["state"], "waiting");
    assert_eq!(p.info(&b)["position"], 1);
    assert_eq!(p.info(&c)["position"], 2);
    p.send(&b, "release");
    p.wait_finished(&b, "done");
    assert_eq!(p.info(&b)["state"], "cancelled", "{}", p.info(&b));
    p.wait_for("c first in line", || p.info(&c)["position"] == 1);
    p.send(&a, "release");
    p.wait_state(&c, "held");
    p.send(&c, "release");
    p.wait_finished(&c, "done");
    let res = p.status("resources/gpu.json");
    assert!(res["holders"].as_array().unwrap().is_empty(), "{res}");
    assert!(res["free_since"].as_str().unwrap().ends_with('Z'), "{res}");
    p.stop();
}

/// AC: a waiting lease does not look every second: while the holder holds, its run turns
/// only when woken or at its deadline, and the release wakes it at once.
#[test]
fn a_waiting_lease_sleeps_until_the_line_changes() {
    let p = Project::with_rules("gpu 20s 1s 30m\n");
    p.start();
    let a = p.ask_mode("x");
    p.wait_state(&a, "held");
    let b = p.ask_mode("x");
    p.wait_state(&b, "waiting");
    let turns = |id: &str| {
        p.run_json(id)["events"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| e["to"] == "turn")
            .count()
    };
    let before = turns(&b);
    thread::sleep(Duration::from_secs(4));
    assert_eq!(turns(&b), before, "it looked again while nothing changed");
    let released = Instant::now();
    p.send(&a, "release");
    p.wait_state(&b, "held");
    assert!(
        released.elapsed() < Duration::from_secs(5),
        "{:?}",
        released.elapsed()
    );
    // Its deadline was a minute away at most, and at least a second.
    let deadline = fs::read_to_string(
        p.root()
            .join(".decree/runs")
            .join(&b)
            .join("lease.deadline"),
    )
    .unwrap();
    assert!(deadline.trim_end().ends_with('Z'), "{deadline}");
    p.send(&b, "release");
    p.wait_finished(&b, "done");
    p.stop();
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
    assert!(line.contains("One mode per resource at a time"), "{line}");
    run(&["install", "lease"]);
    run(&["check"]);
    let req = tmp
        .path()
        .join(".decree/machines/lease/scripts/requirements");
    for tool in ["flock", "jq"] {
        assert!(path_has(&req, &format!("command   {tool}")), "{tool}");
    }
    // The seeded rules name no resource, and no file of the plugin names one.
    let state = tmp.path().join(".decree/machines/lease/state");
    assert!(path_has(
        &state.join("resources.git.txt"),
        "# box1  15m  5s  30m"
    ));
    assert!(!state.join("swap").exists());
    let plugin = fs::read_to_string(tmp.path().join(".decree/plugins/lease.md")).unwrap();
    assert!(!plugin.to_lowercase().contains("gpu"), "lease names a GPU");
}

/// AC: with `holders` 2, two x leases hold gpu together after one swap; a third x waits for
/// room and then joins without a swap; a y lease waits until every x lease is done, then
/// swaps once.
#[test]
fn holders_share_the_resource_in_one_mode() {
    let p = Project::with_rules("gpu 20s 1s 30s * 2\n");
    p.holders.set(2);
    p.start();
    let a = p.ask_mode("x");
    p.wait_state(&a, "held");
    let b = p.ask_mode("x");
    p.wait_state(&b, "held");
    assert!(p.held(&a), "a stopped holding when b joined");
    assert_eq!(p.swaps(), vec!["\"\" -> x"]);
    assert!(p.log(&a).contains("holders=1/2"), "{}", p.log(&a));
    assert!(p.log(&b).contains("holders=2/2"), "{}", p.log(&b));
    let c = p.ask_mode("x");
    let d = p.ask_mode("y");
    p.wait_for("c and d in line", || p.in_line(&c) && p.in_line(&d));
    p.never_held_for(&c, Duration::from_secs(2));
    p.send(&a, "release");
    p.wait_state(&c, "held");
    assert!(!p.held(&d));
    assert_eq!(p.swaps(), vec!["\"\" -> x"]);
    p.send(&b, "release");
    p.never_held_for(&d, Duration::from_secs(1));
    p.send(&c, "release");
    p.wait_state(&d, "held");
    assert_eq!(p.swaps(), vec!["\"\" -> x", "\"x\" -> y"]);
    p.send(&d, "release");
    p.wait_finished(&d, "done");
    p.stop();
}

/// AC: with room for more holders, once x has used its slice and a y lease waits, a new x
/// lease does not join: the x holders drain, and y goes next.
#[test]
fn holders_stop_joining_once_another_mode_waits_past_the_slice() {
    let p = Project::with_rules("gpu 20s 1s 2s * 3\n");
    p.holders.set(3);
    p.start();
    let a = p.ask_mode("x");
    p.wait_state(&a, "held");
    thread::sleep(Duration::from_millis(2500));
    let d = p.ask_mode("y");
    p.wait_for("d in line", || p.in_line(&d));
    let e = p.ask_mode("x");
    p.wait_for("e in line", || p.in_line(&e));
    p.never_held_for(&e, Duration::from_secs(2));
    p.send(&a, "release");
    p.wait_state(&d, "held");
    assert!(!p.held(&e));
    p.send(&d, "release");
    p.wait_state(&e, "held");
    p.send(&e, "release");
    p.wait_finished(&e, "done");
    p.stop();
}

/// AC: a `holders` that is not a whole number from 1 fails the lease, naming it.
#[test]
fn a_bad_holders_fails_naming_it() {
    let p = Project::with_rules("gpu 3s 1s 4s * 0\n");
    let a = p.ask_mode("x");
    let _ = p.decree(&["daemon", "--once"]).output().unwrap();
    assert_eq!(p.state(&a), ("finished".into(), "failed".into()));
    assert!(
        p.log(&a)
            .contains("holders `0` for `gpu` is not a whole number from 1"),
        "{}",
        p.log(&a)
    );
}

/// Each request an `http_recorder` got: its first line and its body.
type Requests = Arc<Mutex<Vec<(String, String)>>>;

/// An address that records each request's first line and body and answers `status`, for
/// the test's life.
fn http_recorder(status: u16) -> (String, Requests) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            // Headers, then as many body bytes as Content-Length says.
            let (head, body) = loop {
                let Ok(n) = s.read(&mut chunk) else {
                    break (String::new(), String::new());
                };
                if n == 0 {
                    break (String::new(), String::new());
                }
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf).to_string();
                let Some(end) = text.find("\r\n\r\n") else {
                    continue;
                };
                let length = text[..end]
                    .lines()
                    .find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                if buf.len() >= end + 4 + length {
                    break (
                        text[..end].to_string(),
                        text[end + 4..end + 4 + length].to_string(),
                    );
                }
            };
            let first = head.lines().next().unwrap_or_default().to_string();
            record.lock().unwrap().push((first, body));
            let _ = s.write_all(
                format!("HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            );
        }
    });
    (
        format!("http://127.0.0.1:{port}/leases/ready?token=t0ken"),
        seen,
    )
}

/// AC: a lease asked for with a callback POSTs its status there once, when the hold
/// starts, then is `held`; a renewal does not call again; the log names the host only.
#[test]
fn a_callback_is_told_once_when_the_hold_starts() {
    let (url, seen) = http_recorder(204);
    let p = Project::new();
    p.start();
    let a = p.ask(&["resource=gpu", "mode=x", &format!("callback={url}")]);
    p.wait_state(&a, "held");
    let calls = seen.lock().unwrap().clone();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0].0, "POST /leases/ready?token=t0ken HTTP/1.1");
    let body: Value = serde_json::from_str(&calls[0].1).unwrap();
    assert_eq!(body["run"], a.as_str(), "{body}");
    assert_eq!(body["resource"], "gpu");
    assert_eq!(body["mode"], "x");
    // The asker is told before the lease reads `held`: the body says `granted`.
    assert_eq!(body["state"], "granted");
    assert_eq!(body["expires"], p.info(&a)["expires"]);
    assert_eq!(p.info(&a)["state"], "held");
    p.send(&a, "renew");
    p.wait_for("the renewal", || {
        p.log(&a).contains("lease: renew resource=")
    });
    p.send(&a, "release");
    p.wait_finished(&a, "done");
    assert_eq!(seen.lock().unwrap().len(), 1);
    let log = p.log(&a);
    assert!(log.contains("lease: callback resource=gpu mode=x"), "{log}");
    assert!(!log.contains("t0ken"), "{log}");
    p.stop();
}

/// AC: a callback nobody answers with a 2xx fails the lease and gives the resource back, so
/// the next lease is held; a callback that is not an http address fails at once.
#[test]
fn a_callback_nobody_takes_gives_the_resource_back() {
    let (url, seen) = http_recorder(500);
    let p = Project::new();
    p.start();
    let a = p.ask(&["resource=gpu", "mode=x", &format!("callback={url}")]);
    // Leases run at once: asked together, either may be granted first.
    p.wait_for("a granted", || p.ever_in(&a, "swap"));
    let b = p.ask_mode("x");
    p.wait_finished(&a, "failed");
    assert_eq!(seen.lock().unwrap().len(), 3, "three tries");
    assert!(!p.ever_in(&a, "held"));
    assert_eq!(p.info(&a)["state"], "failed", "{}", p.info(&a));
    let log = p.log(&a);
    assert!(log.contains("lease: callback failed resource=gpu"), "{log}");
    assert!(!log.contains("t0ken"), "{log}");
    p.wait_state(&b, "held");
    let c = p.ask(&["resource=gpu", "mode=x", "callback=ftp://example"]);
    p.wait_finished(&c, "failed");
    assert!(
        p.log(&c)
            .contains("callback `ftp://example` is not an http:// or https:// address"),
        "{}",
        p.log(&c)
    );
    p.stop();
}

/// AC: a change to the line wakes only the lease next to go: as a line of four drains, each
/// waiting lease turns twice, once on joining and once when its turn comes, wherever it
/// stood.
#[test]
fn a_change_wakes_only_the_lease_next_to_go() {
    let p = Project::with_rules("gpu 20s 1s 30m\n");
    p.start();
    let a = p.ask_mode("x");
    p.wait_state(&a, "held");
    let line: Vec<String> = (0..4).map(|_| p.ask_mode("x")).collect();
    for id in &line {
        p.wait_state(id, "waiting");
    }
    p.send(&a, "release");
    for id in &line {
        p.wait_state(id, "held");
        p.send(id, "release");
        p.wait_finished(id, "done");
    }
    for id in &line {
        let turns = p.run_json(id)["events"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| e["to"] == "turn")
            .count();
        assert_eq!(turns, 2, "{id} turned {turns} times");
    }
    p.stop();
}

/// AC: a holder whose run is over is dropped without making the mode unknown unless it was
/// swapping: with room for two in mode `a`, a dead `granted` holder beside a live one does not
/// send the next `a` lease through a swap while the live one uses the resource.
#[test]
fn a_dead_granted_holder_leaves_the_mode_set() {
    let p = Project::with_rules("gpu 30s 0s 30m * 2\n");
    p.holders.set(2);
    p.start();
    let a = p.ask_mode("a");
    p.wait_state(&a, "held");
    // A lease that was granted in the mode already set, whose run is gone.
    let holder = p.lease_dir().join("leases/gpu/holder");
    let mut text = fs::read_to_string(&holder).unwrap();
    text.push_str("20200101T000000.000000Z-dead00 a 1000 granted\n");
    fs::write(&holder, text).unwrap();
    let c = p.ask_mode("a");
    p.wait_state(&c, "held");
    assert_eq!(p.swaps(), vec!["\"\" -> a"], "a second swap while `a` held");
    p.send(&a, "release");
    p.send(&c, "release");
    p.wait_finished(&c, "done");
    p.stop();
}

/// AC: a wake never ends as a rejected reply: a lease woken just as its own turn grants it,
/// whose swap then fails, leaves no failed `invalid_message` run behind.
#[test]
fn a_wake_for_a_lease_that_ends_is_taken_back() {
    let p = Project::with_rules("gpu 2s 0s 30m\n");
    p.start();
    let a = p.ask_mode("x");
    p.wait_state(&a, "held");
    // z's swap fails (the stub refuses z): Z is granted at A's expiry and fails at once.
    let z = p.ask_mode("z");
    p.wait_finished(&a, "done");
    p.wait_finished(&z, "failed");
    // A pass more, for any wake left over to be delivered.
    thread::sleep(Duration::from_millis(2500));
    let out = p.decree(&["status", "--format", "json"]).output().unwrap();
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    let rejected: Vec<&Value> = doc["runs"]["finished"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["machine"] != "lease")
        .collect();
    assert!(rejected.is_empty(), "{rejected:?}");
    assert!(
        fs::read_dir(p.root().join(".decree/inbox"))
            .unwrap()
            .next()
            .is_none(),
        "a wake was left queued"
    );
    p.stop();
}

/// AC: a client's release is queued at once even while a wake for the lease is queued, or
/// while its run is busy: no retry is needed.
#[test]
fn a_release_is_not_refused_while_a_wake_is_queued() {
    let p = Project::with_rules("gpu 20s 0s 30m\n");
    p.start();
    let a = p.ask_mode("x");
    p.wait_state(&a, "held");
    let b = p.ask_mode("x");
    p.wait_state(&b, "waiting");
    fs::write(
        p.root().join(".decree/inbox/lease-again-by-hand.md"),
        format!("---\nto: {b}\nevent: again\n---\n"),
    )
    .unwrap();
    p.decree(&["reply", &b, "release"]).assert().success();
    p.wait_finished(&b, "done");
    assert_eq!(p.info(&b)["state"], "cancelled");
    p.send(&a, "release");
    p.wait_finished(&a, "done");
    p.stop();
}

/// AC: a status file older than a day that never ended (a lease held, or waiting, that long)
/// is kept, and does not keep another lease's release from finishing `done`.
#[test]
fn an_old_status_that_never_ended_does_not_fail_a_release() {
    let p = Project::with_rules("gpu 20s 0s 30m\n");
    let leases = p.lease_dir().join("status/leases");
    fs::create_dir_all(&leases).unwrap();
    let old = leases.join("20200101T000000.000000Z-0old00.json");
    fs::write(&old, "{\"state\": \"held\"}\n").unwrap();
    let two_days = std::time::SystemTime::now() - Duration::from_secs(2 * 86400);
    fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(two_days)
        .unwrap();
    p.start();
    let a = p.ask_mode("x");
    p.wait_state(&a, "held");
    p.send(&a, "release");
    p.wait_finished(&a, "done");
    assert!(old.exists(), "an unended status was removed");
    p.stop();
}
