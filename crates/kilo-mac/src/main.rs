//! Kilo for macOS. One executable, five roles:
//! - no arguments: the app (native AppKit UI, never loads a web engine);
//! - `--player-helper`: YouTube's player in a hidden web view (kilo-player);
//! - `--login-helper`: a Google sign-in window, exits once signed in;
//! - `--prune-helper`: deletes Kilo's web data except YouTube's, then exits;
//! - `--sign-out-helper`: deletes Kilo's web data (the session), then exits.

#[cfg(target_os = "macos")]
mod app;
#[cfg(target_os = "macos")]
mod debug;
#[cfg(target_os = "macos")]
mod images;
#[cfg(target_os = "macos")]
mod keys;
#[cfg(target_os = "macos")]
mod login;
#[cfg(target_os = "macos")]
mod net;
#[cfg(target_os = "macos")]
mod pagecache;
#[cfg(target_os = "macos")]
mod paths;
#[cfg(target_os = "macos")]
mod settings;
#[cfg(target_os = "macos")]
mod strings;
#[cfg(target_os = "macos")]
mod ui;

#[cfg(target_os = "macos")]
fn main() {
    debug::mark_launch();
    match std::env::args().nth(1).as_deref() {
        Some("--player-helper") => kilo_player::run_helper(),
        Some("--login-helper") => {
            identify_browser();
            login::run()
        }
        Some("--prune-helper") => login::prune(),
        Some("--sign-out-helper") => login::sign_out(),
        _ => {
            identify_browser();
            app::run()
        }
    }
}

/// Kilo presents itself as the Mac's own Safari (`kilo_core::http`): reads
/// which version that is.
#[cfg(target_os = "macos")]
fn identify_browser() {
    use objc2_foundation::{NSBundle, NSString};
    let version = NSBundle::bundleWithPath(&NSString::from_str("/Applications/Safari.app"))
        .and_then(|b| b.objectForInfoDictionaryKey(&NSString::from_str("CFBundleShortVersionString")))
        .and_then(|v| v.downcast::<NSString>().ok());
    if let Some(version) = version {
        kilo_core::http::set_safari_version(&version.to_string());
    }
    debug::trace(|| format!("launch: Safari {}", kilo_core::http::safari_version()));
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("This is the macOS front end; other platforms get their own.");
}
