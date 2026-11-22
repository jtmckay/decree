//! How many runs of each machine execute at once (docs/reference/machines.md, Concurrency):
//! a counting semaphore across every decree process sharing the project. Machine `m` with
//! `concurrency: n` has the slots `.decree/cache/slots/m/0.lock` to `<n-1>.lock`; a run holds
//! one while it executes, as an exclusive `flock` on that file. A lock belongs to its open
//! file, so two threads of one process count as two holders, and a process that dies frees
//! its slots with it. A run of this process waiting for a slot goes before a new claim.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::layout::{CACHE_DIR, DECREE_DIR};

/// The slot folders of one project, and how many runs of this process wait for each key.
#[derive(Debug, Clone)]
pub struct Slots {
    dir: PathBuf,
    /// Waiters per key in this process (`take_blocking`): while any waits, `try_take` leaves
    /// the key's free slots to them, so a child or a continued run is not passed over by
    /// new claims.
    waiting: Arc<Mutex<HashMap<String, usize>>>,
}

/// A held slot; dropping it frees the slot.
#[derive(Debug)]
pub struct SlotGuard {
    _file: File,
}

/// How long `take_blocking` waits between tries.
const RETRY: Duration = Duration::from_millis(100);

impl Slots {
    /// The slots of the project at `project_root`.
    pub fn new(project_root: &std::path::Path) -> Slots {
        Slots {
            dir: project_root.join(DECREE_DIR).join(CACHE_DIR).join("slots"),
            waiting: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// A free slot of `key`, which has `n` of them, or `None` when all are held or a run of
    /// this process waits for one (`take_blocking`), which goes first.
    pub fn try_take(&self, key: &str, n: usize) -> io::Result<Option<SlotGuard>> {
        if self.waiters(key) > 0 {
            return Ok(None);
        }
        self.take(key, n)
    }

    fn waiters(&self, key: &str) -> usize {
        self.waiting
            .lock()
            .map_or(0, |w| w.get(key).copied().unwrap_or(0))
    }

    /// A free slot among `0..n`, if fewer than `n` slots of `key` are held in all: a slot
    /// file from a larger `concurrency` (lowered while its runs execute) still counts while
    /// it is held.
    fn take(&self, key: &str, n: usize) -> io::Result<Option<SlotGuard>> {
        let n = n.max(1);
        let dir = self.dir.join(key);
        fs::create_dir_all(&dir)?;
        let mut held = 0;
        let mut free = None;
        let mut k = 0;
        loop {
            let path = dir.join(format!("{k}.lock"));
            if k >= n && !path.exists() {
                break;
            }
            let file = OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(&path)?;
            // SAFETY: flock on a descriptor this function owns; it reads no memory.
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if rc == 0 {
                // A free slot past `n` is not one to take: dropping the file frees it.
                if k < n && free.is_none() {
                    free = Some(SlotGuard { _file: file });
                }
            } else {
                let err = io::Error::last_os_error();
                if err.raw_os_error() != Some(libc::EWOULDBLOCK) {
                    return Err(err);
                }
                held += 1;
            }
            k += 1;
        }
        Ok(if held < n { free } else { None })
    }

    /// A slot of `key`, waiting for one to free; `None` when none is free and `shutdown` is
    /// set. While it waits, `try_take` leaves this key's slots to it.
    pub fn take_blocking(
        &self,
        key: &str,
        n: usize,
        shutdown: &AtomicBool,
    ) -> io::Result<Option<SlotGuard>> {
        if let Some(guard) = self.take(key, n)? {
            return Ok(Some(guard));
        }
        if shutdown.load(Ordering::Relaxed) {
            return Ok(None);
        }
        self.count(key, 1);
        let result = loop {
            thread::sleep(RETRY);
            match self.take(key, n) {
                Ok(Some(guard)) => break Ok(Some(guard)),
                Ok(None) if shutdown.load(Ordering::Relaxed) => break Ok(None),
                Ok(None) => {}
                Err(e) => break Err(e),
            }
        };
        self.count(key, -1);
        result
    }

    fn count(&self, key: &str, by: isize) {
        if let Ok(mut w) = self.waiting.lock() {
            let n = w.entry(key.to_string()).or_insert(0);
            *n = n.saturating_add_signed(by);
            if *n == 0 {
                w.remove(key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    #[test]
    fn n_slots_are_taken_and_the_next_is_not() {
        let tmp = TempDir::new().unwrap();
        let slots = Slots::new(tmp.path());
        let a = slots.try_take("m", 2).unwrap();
        let b = slots.try_take("m", 2).unwrap();
        assert!(a.is_some() && b.is_some());
        assert!(slots.try_take("m", 2).unwrap().is_none());
        // Another machine's slots are its own.
        assert!(slots.try_take("other", 1).unwrap().is_some());
        drop(a);
        assert!(slots.try_take("m", 2).unwrap().is_some());
    }

    #[test]
    fn another_process_holding_a_slot_counts() {
        if Command::new("flock").arg("--version").output().is_err() {
            return;
        }
        let tmp = TempDir::new().unwrap();
        let slots = Slots::new(tmp.path());
        drop(slots.try_take("m", 1).unwrap());
        let file = tmp.path().join(".decree/cache/slots/m/0.lock");
        // flock holds the file while its `sleep` runs, then frees it.
        let mut child = Command::new("flock")
            .arg(&file)
            .args(["sleep", "1"])
            .spawn()
            .unwrap();
        let held = (0..40).any(|_| {
            thread::sleep(Duration::from_millis(20));
            slots.try_take("m", 1).unwrap().is_none()
        });
        child.wait().unwrap();
        assert!(held, "the other process's slot was not counted");
        assert!(slots.try_take("m", 1).unwrap().is_some());
    }

    #[test]
    fn take_blocking_waits_for_a_free_slot_and_gives_up_on_shutdown() {
        let tmp = TempDir::new().unwrap();
        let slots = Slots::new(tmp.path());
        let stop = AtomicBool::new(true);
        // A free slot is taken even after a signal; a wait is not started.
        assert!(slots.take_blocking("m", 1, &stop).unwrap().is_some());
        let held = slots.try_take("m", 1).unwrap();
        assert!(slots.take_blocking("m", 1, &stop).unwrap().is_none());
        let other = slots.clone();
        let waiter = thread::spawn(move || {
            other
                .take_blocking("m", 1, &AtomicBool::new(false))
                .unwrap()
                .is_some()
        });
        thread::sleep(Duration::from_millis(250));
        drop(held);
        assert!(waiter.join().unwrap());
    }

    #[test]
    fn a_waiter_goes_before_a_new_claim_and_lowered_slots_still_count() {
        let tmp = TempDir::new().unwrap();
        let slots = Slots::new(tmp.path());
        let held = slots.try_take("m", 1).unwrap();
        let other = slots.clone();
        let waiter = thread::spawn(move || {
            other
                .take_blocking("m", 1, &AtomicBool::new(false))
                .unwrap()
                .is_some()
        });
        thread::sleep(Duration::from_millis(250));
        drop(held);
        // The slot is free, but the waiter has it first.
        assert!(slots.try_take("m", 1).unwrap().is_none() || waiter.is_finished());
        assert!(waiter.join().unwrap());

        // Three held under `concurrency: 3`, lowered to 2: none is free until two are left.
        let a = slots.try_take("n", 3).unwrap();
        let b = slots.try_take("n", 3).unwrap();
        let c = slots.try_take("n", 3).unwrap();
        assert!(a.is_some() && b.is_some() && c.is_some());
        drop(a);
        assert!(slots.try_take("n", 2).unwrap().is_none());
        drop(b);
        assert!(slots.try_take("n", 2).unwrap().is_some());
    }
}
