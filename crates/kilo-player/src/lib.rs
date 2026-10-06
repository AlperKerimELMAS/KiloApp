//! Playback through YouTube's own player, in a helper process.
//!
//! The main app never loads a web engine. It spawns itself in helper mode
//! (`--player-helper`), talks to it with one-line text messages over
//! stdin/stdout, and kills it after a while paused so every byte the web
//! engine used goes back to the system.
//!
//! The helper hosts a hidden system web view running YouTube's mobile web
//! player. Kilo never fetches, captures or decodes media itself.

pub mod host;
pub mod protocol;

#[cfg(target_os = "macos")]
mod helper;

/// Runs the helper side. Never returns.
#[cfg(target_os = "macos")]
pub fn run_helper() -> ! {
    helper::run()
}

/// Runs the helper side. Never returns.
#[cfg(not(target_os = "macos"))]
pub fn run_helper() -> ! {
    eprintln!("kilo-player: the web view helper is macOS-only for now");
    std::process::exit(2);
}
