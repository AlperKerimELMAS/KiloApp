//! Pages as YouTube last sent them, on disk, so a page Kilo has shown
//! before appears at once (at launch, going back). One fetched in the last
//! half hour is simply shown again; an older one is shown while a fresh
//! copy loads. The raw answers are kept and parsed again on use (a couple of
//! milliseconds), so this costs no memory and no code. Each sign-in has its
//! own files, and signing out deletes them with the other caches.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use crate::paths;

/// Pages younger than this are shown without asking YouTube again.
const FRESH: Duration = Duration::from_secs(30 * 60);
/// Older pages aren't shown, even while a fresh one loads.
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// The cache is trimmed back to this at launch, and every `TRIM_EVERY`
/// pages written after that (Kilo may stay open for days).
const BUDGET: u64 = 16 * 1024 * 1024;
const TRIM_EVERY: u32 = 32;

/// The file for a page: what it is (`what`), in which language (`hl`), for
/// which sign-in (`session`, `Session::fingerprint`).
pub fn file(what: &str, hl: &str, session: u64) -> PathBuf {
    let hash = kilo_core::fnv1a(format!("{session:016x}\n{hl}\n{what}").as_bytes());
    paths::page_cache().join(format!("{hash:016x}"))
}

/// The cached answer at `file`, unless it's older than a week, and whether
/// it's fresh (fetched in the last half hour; a file from the future, after
/// the clock was set back, isn't).
pub fn read(file: &Path) -> Option<(Vec<u8>, bool)> {
    let age = std::fs::metadata(file).ok()?.modified().ok()?.elapsed().unwrap_or(FRESH);
    (age < MAX_AGE).then(|| std::fs::read(file).ok().map(|json| (json, age < FRESH))).flatten()
}

/// Saves a page fetched for the account of `epoch` (`paths::epoch`), unless
/// that account has been let go of meanwhile. Off the main thread.
pub fn write(file: &Path, json: &[u8], epoch: u64) {
    static WRITES: AtomicU32 = AtomicU32::new(0);
    if paths::write_if_current(file, json, epoch) && WRITES.fetch_add(1, Ordering::Relaxed) % TRIM_EVERY == TRIM_EVERY - 1 {
        trim();
    }
}

/// Keeps the cache within its budget, dropping the oldest pages first. Run
/// off the main thread.
pub fn trim() {
    let Ok(dir) = std::fs::read_dir(paths::page_cache()) else { return };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = dir
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            Some((meta.modified().ok()?, meta.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort();
    for (when, len, path) in files {
        if total <= BUDGET && when.elapsed().unwrap_or_default() < MAX_AGE {
            continue;
        }
        if std::fs::remove_file(path).is_ok() {
            total -= len;
        }
    }
}
