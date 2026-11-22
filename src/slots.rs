//! How many runs of each machine execute at once (docs/reference/machines.md, Concurrency):
//! a counting semaphore across every decree process sharing the project. Machine `m` with
//! `concurrency: n` has the slots `.decree/cache/slots/m/0.lock` to `<n-1>.lock`; a run holds
//! one while it executes, as an exclusive `flock` on that file. A lock belongs to its open
//! file, so two threads of one process count as two holders, and a process that dies frees
//! its slots with it.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crate::layout::{CACHE_DIR, DECREE_DIR};

/// The slot folders of one project.
#[derive(Debug, Clone)]
pub struct Slots {
    dir: PathBuf,
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
        }
    }

    /// A free slot of `machine`, which has `n` of them, or `None` when all are held.
    pub fn try_take(&self, machine: &str, n: usize) -> io::Result<Option<SlotGuard>> {
        let dir = self.dir.join(machine);
        fs::create_dir_all(&dir)?;
        for k in 0..n.max(1) {
            let file = OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(dir.join(format!("{k}.lock")))?;
            // SAFETY: flock on a descriptor this function owns; it reads no memory.
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if rc == 0 {
                return Ok(Some(SlotGuard { _file: file }));
            }
            let err = io::Error::last_os_error();
            if err.raw_os_error() != Some(libc::EWOULDBLOCK) {
                return Err(err);
            }
        }
        Ok(None)
    }

    /// A slot of `machine`, waiting for one to free; `None` when none is free and `shutdown`
    /// is set.
    pub fn take_blocking(
        &self,
        machine: &str,
        n: usize,
        shutdown: &AtomicBool,
    ) -> io::Result<Option<SlotGuard>> {
        loop {
            if let Some(guard) = self.try_take(machine, n)? {
                return Ok(Some(guard));
            }
            if shutdown.load(Ordering::Relaxed) {
                return Ok(None);
            }
            thread::sleep(RETRY);
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
}
