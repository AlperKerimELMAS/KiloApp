//! Kilo for macOS. One executable, three roles:
//! - no arguments: the app (native AppKit UI, never loads a web engine);
//! - `--player-helper`: YouTube's player in a hidden web view (kilo-player);
//! - `--login-helper`: a Google sign-in window, exits once signed in.

#[cfg(target_os = "macos")]
mod app;
#[cfg(target_os = "macos")]
mod debug;
#[cfg(target_os = "macos")]
mod images;
#[cfg(target_os = "macos")]
mod login;
#[cfg(target_os = "macos")]
mod net;
#[cfg(target_os = "macos")]
mod paths;
#[cfg(target_os = "macos")]
mod ui;

#[cfg(target_os = "macos")]
fn main() {
    debug::mark_launch();
    match std::env::args().nth(1).as_deref() {
        Some("--player-helper") => kilo_player::run_helper(),
        Some("--login-helper") => login::run(),
        _ => app::run(),
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("This is the macOS front end; other platforms get their own.");
}
