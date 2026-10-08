//! Where Kilo keeps things on disk.

use std::path::PathBuf;
use std::sync::OnceLock;

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
    home().join("Library/Application Support/Kilo/innertube.txt")
}

/// Kilo's caches (thumbnails, and WebKit's), which the system may purge.
fn caches() -> PathBuf {
    home().join(format!("Library/Caches/{}", bundle_id()))
}

/// Downloaded thumbnails.
pub fn image_cache() -> PathBuf {
    caches().join("images")
}

/// WebKit's other data for Kilo (site storage, and bookkeeping like its
/// tracking prevention's record of sites seen).
fn webkit() -> PathBuf {
    home().join(format!("Library/WebKit/{}", bundle_id()))
}

/// Signing out: deletes what Kilo wrote itself (the page config, the
/// caches) and whatever WebKit left after the sign-out helper emptied its
/// stores (its API keeps some bookkeeping, and old copies of the cookie
/// file). Nothing may be running WebKit for Kilo.
pub fn remove_own_data() {
    let _ = std::fs::remove_file(config());
    let _ = std::fs::remove_dir_all(caches());
    let _ = std::fs::remove_dir_all(webkit());
    let _ = std::fs::remove_file(cookies());
    remove_cookie_copies();
}

/// Deletes the old copies of the cookie file WebKit leaves behind (it saves
/// the file through `<file>_tmp_<pid>.dat` copies).
pub fn remove_cookie_copies() {
    let cookies = cookies();
    let (Some(dir), Some(name)) = (cookies.parent(), cookies.file_name().and_then(|n| n.to_str())) else { return };
    let copy = format!("{name}_tmp_");
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.file_name().to_str().is_some_and(|n| n.starts_with(&copy)) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        remove_own_data();
        for gone in [cookies(), config(), caches(), webkit(), copy] {
            assert!(!gone.exists(), "{}", gone.display());
        }
        assert!(other.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
