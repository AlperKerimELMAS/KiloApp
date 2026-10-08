//! Pages as YouTube last sent them, on disk, so a page Kilo has shown
//! before appears at once (at launch, going back). One fetched in the last
//! half hour is simply shown again; an older one is shown while a fresh
//! copy loads. The raw answers are kept and parsed again on use (a couple of
//! milliseconds), so this costs no memory and no code. Signing out deletes
//! them with the other caches.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use crate::paths;

/// Pages younger than this are shown without asking YouTube again.
const FRESH: Duration = Duration::from_secs(30 * 60);
/// Older pages aren't shown, even while a fresh one loads.
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// The cache is trimmed back to this at launch.
const BUDGET: u64 = 16 * 1024 * 1024;

/// The file for a page: what it is (`what`) in which language (`hl`).
pub fn file(what: &str, hl: &str) -> PathBuf {
    // FNV-1a: a stable, dependency-free file name.
    let hash = format!("{hl}\n{what}").bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    paths::page_cache().join(format!("{hash:016x}"))
}

/// The cached answer at `file`, unless it's older than a week, and whether
/// it's fresh (fetched in the last half hour).
pub fn read(file: &PathBuf) -> Option<(Vec<u8>, bool)> {
    let age = std::fs::metadata(file).ok()?.modified().ok()?.elapsed().unwrap_or_default();
    (age < MAX_AGE).then(|| std::fs::read(file).ok().map(|json| (json, age < FRESH))).flatten()
}

pub fn write(file: &PathBuf, json: &[u8]) {
    if std::fs::create_dir_all(paths::page_cache()).is_ok() {
        let _ = std::fs::write(file, json);
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
