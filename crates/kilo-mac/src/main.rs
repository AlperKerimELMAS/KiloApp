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
        Some("--login-helper") => login::run(),
        Some("--prune-helper") => login::prune(),
        Some("--sign-out-helper") => login::sign_out(),
        _ => app::run(),
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("This is the macOS front end; other platforms get their own.");
}
