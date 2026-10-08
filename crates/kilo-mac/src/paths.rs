//! Where Kilo keeps things on disk.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use objc2_foundation::{NSBundle, NSHomeDirectory};

/// The bundle id Kilo ships with (packaging/macos/Info.plist).
const BUNDLE_ID: &str = "io.github.alperkerimelmas.kilo";

/// The running app's bundle id, which WebKit keys its stores by (Kilo's
/// own for a bare binary).
fn bundle_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| NSBundle::mainBundle().bundleIdentifier().map_or_else(|| BUNDLE_ID.to_owned(), |id| id.to_string()))
}

/// `HOME`, or the account's home folder if that's unset: never a shared
/// folder like `/tmp`.
fn home() -> PathBuf {
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) if home.is_absolute() => home,
        _ => PathBuf::from(NSHomeDirectory().to_string()),
    }
}

/// The sign-in web view's cookie store (written by WebKit).
pub fn cookies() -> PathBuf {
    home().join(format!("Library/HTTPStorages/{}.binarycookies", bundle_id()))
}

/// music.youtube.com's page config, cached for a day.
pub fn config() -> PathBuf {
    home().join(format!("Library/Application Support/{}/innertube.txt", bundle_id()))
}

/// Where versions up to 0.3 kept the page config, whatever the bundle id.
fn legacy_config() -> PathBuf {
    home().join("Library/Application Support/Kilo/innertube.txt")
}

/// Bumped when an account's data is let go of (signing out, a session that
/// stopped working). Work that started before must not write it back.
static EPOCH: AtomicU64 = AtomicU64::new(0);

/// The current account epoch, to hand to work that will write to disk.
pub fn epoch() -> u64 {
    EPOCH.load(Ordering::SeqCst)
}

/// Starts a new account epoch: writes from work started before are dropped.
pub fn new_epoch() {
    EPOCH.fetch_add(1, Ordering::SeqCst);
}

/// Writes `bytes` to `path` (atomically) if no account has been let go of
/// since `epoch`. Returns whether it did.
pub fn write_if_current(path: &Path, bytes: &[u8], epoch: u64) -> bool {
    epoch == self::epoch() && kilo_core::write_atomically(path, bytes).is_ok()
}

/// Kilo's caches (thumbnails, and WebKit's), which the system may purge.
fn caches() -> PathBuf {
    home().join(format!("Library/Caches/{}", bundle_id()))
}

/// Downloaded thumbnails.
pub fn image_cache() -> PathBuf {
    caches().join("images")
}

/// Pages as YouTube last sent them (`pagecache`).
pub fn page_cache() -> PathBuf {
    caches().join("pages")
}

/// WebKit's other data for Kilo (site storage, and bookkeeping like its
/// tracking prevention's record of sites seen).
fn webkit() -> PathBuf {
    home().join(format!("Library/WebKit/{}", bundle_id()))
}

/// Signing out: deletes what Kilo wrote itself (the page config, the
/// caches) and whatever WebKit left after the sign-out helper emptied its
/// stores (its API keeps some bookkeeping, and old copies of the cookie
/// file). Nothing may be running WebKit for Kilo. Returns whether all of it
/// is gone.
pub fn remove_own_data() -> bool {
    // Every deletion is tried, whatever failed before it.
    let support = config().parent().map(Path::to_path_buf).unwrap_or_default();
    let files = [config(), cookies(), legacy_config()].map(|f| removed(std::fs::remove_file(f)));
    let dirs = [support, caches(), webkit()].map(|d| removed(std::fs::remove_dir_all(d)));
    // The old config's folder only if that left it empty: "Kilo" is a
    // common name, and another app may keep its files there.
    if let Some(old) = legacy_config().parent() {
        let _ = std::fs::remove_dir(old);
    }
    let copies = remove_cookie_copies();
    files.into_iter().chain(dirs).all(|ok| ok) && copies
}

/// Deletes the old copies of the cookie file WebKit leaves behind (it saves
/// the file through `<file>_tmp_<pid>.dat` copies). Returns whether they're
/// all gone.
pub fn remove_cookie_copies() -> bool {
    let cookies = cookies();
    let (Some(dir), Some(name)) = (cookies.parent(), cookies.file_name().and_then(|n| n.to_str())) else { return true };
    let copy = format!("{name}_tmp_");
    let failed = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.starts_with(&copy)))
        .map(|e| removed(std::fs::remove_file(e.path())))
        .filter(|ok| !ok)
        .count();
    failed == 0
}

/// Deleting worked, or there was nothing to delete.
fn removed(result: std::io::Result<()>) -> bool {
    match result {
        Ok(()) => true,
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_from_before_an_account_was_let_go_of_are_dropped() {
        let file = std::env::temp_dir().join(format!("kilo-epoch-test-{}/page", std::process::id()));
        let before = epoch();
        new_epoch();
        assert!(!write_if_current(&file, b"old account", before));
        assert!(!file.exists());
        assert!(write_if_current(&file, b"new account", epoch()));
        assert_eq!(std::fs::read(&file).unwrap(), b"new account");
        std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
    }

    #[test]
    fn signing_out_removes_kilos_files_and_nothing_else() {
        let root = std::env::temp_dir().join(format!("kilo-paths-test-{}", std::process::id()));
        // SAFETY: no other test reads or writes the environment.
        unsafe { std::env::set_var("HOME", &root) };
        // Never anywhere near the real home folder.
        assert!([cookies(), config(), caches(), webkit()].iter().all(|p| p.starts_with(&root)));
        let copy = cookies().with_file_name(format!("{BUNDLE_ID}.binarycookies_tmp_42.dat"));
        let other = cookies().with_file_name("com.example.other.binarycookies");
        let seen = webkit().join("WebsiteData/ResourceLoadStatistics/observations.db");
        for file in [cookies(), config(), image_cache().join("0123"), caches().join("WebKit/cache"), seen, copy.clone(), other.clone()] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, b"x").unwrap();
        }
        // The old config's folder has a generic name: another app's file in
        // it stays (and so does the folder).
        let legacy = legacy_config();
        let neighbour = legacy.with_file_name("someone-elses.db");
        for file in [&legacy, &neighbour] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"x").unwrap();
        }
        assert!(remove_own_data());
        for gone in [cookies(), config(), caches(), webkit(), copy, legacy.clone()] {
            assert!(!gone.exists(), "{}", gone.display());
        }
        assert!(other.exists() && neighbour.exists());
        // Alone in it, Kilo's old config takes the folder with it.
        std::fs::remove_file(&neighbour).unwrap();
        std::fs::write(&legacy, b"x").unwrap();
        assert!(remove_own_data());
        assert!(!legacy.parent().unwrap().exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
