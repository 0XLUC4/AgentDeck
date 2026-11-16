//! Disk writes off the UI thread (a JSON save with fsync takes 5-50 ms, more under an antivirus).
//! Each write carries a file's whole new content, so a burst keeps only the newest, and a write
//! that runs late never overwrites a newer one.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

use gpui_kit::*;

/// Files written through here. Only files whose every key is written through here: a background
/// read-modify-write of a shared file (local.json) could drop a key another writer just set.
pub const STORE: &str = "store.json";
pub const SETTINGS: &str = "settings.json";

static NEXT: AtomicU64 = AtomicU64::new(1);
/// Newest write asked per file. Held only briefly: the UI thread takes it.
static ASKED: LazyLock<Mutex<HashMap<&'static str, u64>>> = LazyLock::new(Mutex::default);
/// Last write done per file. Held during a write: one writer at a time, never the UI thread
/// except at quit.
static WRITTEN: LazyLock<Mutex<HashMap<&'static str, u64>>> = LazyLock::new(Mutex::default);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn ask(file: &'static str) -> u64 {
    let seq = NEXT.fetch_add(1, Ordering::Relaxed);
    lock(&ASKED).insert(file, seq);
    seq
}

fn run(file: &'static str, seq: u64, write: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    let mut written = lock(&WRITTEN);
    let newer_asked = lock(&ASKED).get(file).is_some_and(|&asked| asked > seq);
    if newer_asked || written.get(file).is_some_and(|&done| done >= seq) {
        return Ok(()); // a newer content of the same file is (or will be) written
    }
    write()?;
    written.insert(file, seq);
    Ok(())
}

/// Writes in the background; the task resolves to the error, if any (skipped writes are `Ok`).
/// `file` names the target (one name per file) and `write` owns everything it writes.
pub fn write_later(
    file: &'static str,
    write: impl FnOnce() -> Result<(), String> + Send + 'static,
    cx: &App,
) -> Task<Result<(), String>> {
    let seq = ask(file);
    cx.background_spawn(async move { run(file, seq, write) })
}

/// Writes now, on this thread (quit): after any write already running, and newest of all.
pub fn write_now(file: &'static str, write: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    let seq = ask(file);
    run(file, seq, write)
}

#[cfg(test)]
mod tests {
    use super::{ask, run};

    #[test]
    fn a_late_older_write_is_skipped() {
        let (old, new) = (ask("t1"), ask("t1"));
        let mut order = Vec::new();
        run("t1", new, || Ok(order.push(new))).unwrap();
        run("t1", old, || Ok(order.push(old))).unwrap();
        assert_eq!(order, [new]);
    }

    #[test]
    fn a_burst_keeps_only_the_newest() {
        let seqs: Vec<u64> = (0..5).map(|_| ask("t2")).collect();
        let mut done = Vec::new();
        for &seq in &seqs {
            run("t2", seq, || Ok(done.push(seq))).unwrap();
        }
        assert_eq!(done, [seqs[4]]);
        // Files are independent.
        let other = ask("t3");
        run("t3", other, || Ok(done.push(other))).unwrap();
        assert_eq!(done, [seqs[4], other]);
    }
}
