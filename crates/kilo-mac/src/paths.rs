//! Where Kilo keeps things on disk.

use std::path::PathBuf;

/// Must match CFBundleIdentifier in the app's Info.plist: WebKit keys its
/// cookie store by it.
pub const BUNDLE_ID: &str = "io.github.alperkerimelmas.kilo";

fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from)
}

/// The sign-in web view's cookie store (written by WebKit).
pub fn cookies() -> PathBuf {
    home().join(format!("Library/HTTPStorages/{BUNDLE_ID}.binarycookies"))
}

/// Small state Kilo writes itself (cached page config).
pub fn support() -> PathBuf {
    home().join("Library/Application Support/Kilo")
}

/// Downloaded thumbnails; the system may purge this.
pub fn image_cache() -> PathBuf {
    home().join(format!("Library/Caches/{BUNDLE_ID}/images"))
}
